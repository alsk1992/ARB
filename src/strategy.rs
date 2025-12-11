use anyhow::Result;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, info, warn};

use crate::clob::ClobClient;
use crate::config::Config;
use crate::signer::OrderSigner;
use crate::types::{BtcMarket, Order, Orderbook, Position, Side};

/// Ladder arbitrage strategy
///
/// Key insight: In BTC 15-min binary markets, one outcome ALWAYS wins.
/// If we buy BOTH Up and Down, one side pays out $1 per share.
/// If combined cost < $1, we profit regardless of outcome.
///
/// Pro traders like nobuyoshi005 use 30-40 price levels to get filled
/// as the market price oscillates during the 15-min window.
pub struct LadderStrategy {
    config: Config,
    clob: ClobClient,
    signer: OrderSigner,
}

impl LadderStrategy {
    pub fn new(config: Config, clob: ClobClient, signer: OrderSigner) -> Self {
        Self { config, clob, signer }
    }

    /// Calculate optimal ladder prices for both sides
    ///
    /// Strategy: We want to buy both sides such that combined cost < $1.
    /// The best ask on each side tells us the minimum we'd pay for instant fill.
    /// We place limit orders BELOW the best ask to get better prices as the market moves.
    ///
    /// Pro strategy observed:
    /// - 30-40 price levels on each side
    /// - Start just below best ask (to catch dips)
    /// - Spread across a range (e.g., 40¢ to 48¢)
    /// - Combined target: up_price + down_price < 96¢ for 4%+ profit
    pub fn calculate_ladder_prices(
        &self,
        up_orderbook: &Orderbook,
        down_orderbook: &Orderbook,
        tick_size: Decimal,
    ) -> (Vec<Decimal>, Vec<Decimal>) {
        let levels = self.config.ladder_levels;
        let target_spread_pct = self.config.target_spread_percent;

        // Get current best asks (what we'd pay for instant fill)
        let up_best_ask = self.get_best_ask(up_orderbook);
        let down_best_ask = self.get_best_ask(down_orderbook);

        let combined_ask = up_best_ask + down_best_ask;
        let current_spread_pct = (dec!(1) - combined_ask) / combined_ask * dec!(100);

        info!("Current market: UP ask={}, DOWN ask={}, Combined={}, Spread={}%",
            up_best_ask, down_best_ask, combined_ask, current_spread_pct);

        // Calculate target prices for desired profit margin
        // If we want 4% profit: up + down = 0.96
        // Distribute evenly: each side ~0.48 average
        let target_combined = dec!(1) - (target_spread_pct / dec!(100));
        let target_each = target_combined / dec!(2);

        // For each side, generate a ladder from just below best ask down to our target
        // This way we catch fills as the market oscillates
        let up_prices = self.generate_dynamic_ladder(
            up_best_ask,
            target_each,
            levels,
            tick_size,
        );

        let down_prices = self.generate_dynamic_ladder(
            down_best_ask,
            target_each,
            levels,
            tick_size,
        );

        // Log ladder summary
        if let (Some(up_high), Some(up_low)) = (up_prices.first(), up_prices.last()) {
            info!("UP ladder: {} levels from {} to {}", up_prices.len(), up_high, up_low);
        }
        if let (Some(down_high), Some(down_low)) = (down_prices.first(), down_prices.last()) {
            info!("DOWN ladder: {} levels from {} to {}", down_prices.len(), down_high, down_low);
        }

        (up_prices, down_prices)
    }

    /// Generate a ladder of prices from just below current ask down to target
    fn generate_dynamic_ladder(
        &self,
        best_ask: Decimal,
        target_price: Decimal,
        levels: u32,
        tick_size: Decimal,
    ) -> Vec<Decimal> {
        let mut prices = Vec::with_capacity(levels as usize);

        // Start just below best ask (1 tick below to not cross spread)
        let start_price = ((best_ask - tick_size) / tick_size).floor() * tick_size;

        // End at our target or a minimum floor
        let min_price = dec!(0.35); // Don't go below 35¢
        let end_price = target_price.max(min_price);

        // Calculate spacing to distribute levels evenly
        let price_range = start_price - end_price;
        if price_range <= Decimal::ZERO || levels == 0 {
            // If no room for ladder, just place at target
            return vec![target_price];
        }

        let spacing = price_range / Decimal::from(levels - 1);
        let spacing = (spacing / tick_size).floor() * tick_size; // Round to tick
        let spacing = spacing.max(tick_size); // At least 1 tick apart

        for i in 0..levels {
            let price = start_price - (spacing * Decimal::from(i));
            let price = (price / tick_size).round() * tick_size;

            // Validate price is in reasonable range
            if price > min_price && price < dec!(0.65) {
                prices.push(price);
            }
        }

        prices
    }

    /// Get best ask price from orderbook
    fn get_best_ask(&self, orderbook: &Orderbook) -> Decimal {
        orderbook.asks.first()
            .and_then(|p| p.price.parse::<Decimal>().ok())
            .unwrap_or(dec!(0.50)) // Default to 50¢ if no asks
    }

    /// Get mid price from orderbook
    fn get_mid_price(&self, orderbook: &Orderbook) -> Decimal {
        let best_bid = orderbook.bids.first()
            .and_then(|p| p.price.parse::<Decimal>().ok())
            .unwrap_or(dec!(0.40));

        let best_ask = orderbook.asks.first()
            .and_then(|p| p.price.parse::<Decimal>().ok())
            .unwrap_or(dec!(0.60));

        (best_bid + best_ask) / dec!(2)
    }

    /// Create ladder orders for both sides of a market
    pub async fn create_ladder_orders(
        &self,
        market: &BtcMarket,
        up_orderbook: &Orderbook,
        down_orderbook: &Orderbook,
    ) -> Result<(Vec<Order>, Vec<Order>)> {
        let (up_prices, down_prices) = self.calculate_ladder_prices(
            up_orderbook,
            down_orderbook,
            market.tick_size,
        );

        info!("UP ladder prices: {:?}", up_prices);
        info!("DOWN ladder prices: {:?}", down_prices);

        // Calculate size per order
        let total_per_side = self.config.max_position_usd / dec!(2);
        let size_per_level = total_per_side / Decimal::from(self.config.ladder_levels);

        // Create UP orders
        let mut up_orders = Vec::new();
        for price in &up_prices {
            // Size in shares = USD / price
            let shares = size_per_level / *price;
            let order = self.signer.create_order(
                &market.up_token_id,
                *price,
                shares,
                Side::Buy,
                market.tick_size,
                market.neg_risk,
            ).await?;
            up_orders.push(order);
        }

        // Create DOWN orders
        let mut down_orders = Vec::new();
        for price in &down_prices {
            let shares = size_per_level / *price;
            let order = self.signer.create_order(
                &market.down_token_id,
                *price,
                shares,
                Side::Buy,
                market.tick_size,
                market.neg_risk,
            ).await?;
            down_orders.push(order);
        }

        Ok((up_orders, down_orders))
    }

    /// Submit all ladder orders
    pub async fn submit_ladder(
        &self,
        market: &BtcMarket,
    ) -> Result<(Vec<String>, Vec<String>)> {
        // Fetch current orderbooks
        let up_book = self.clob.get_orderbook(&market.up_token_id).await?;
        let down_book = self.clob.get_orderbook(&market.down_token_id).await?;

        // Create orders
        let (up_orders, down_orders) = self.create_ladder_orders(market, &up_book, &down_book).await?;

        if self.config.dry_run {
            info!("[DRY RUN] Would submit {} UP orders and {} DOWN orders", up_orders.len(), down_orders.len());
            return Ok((vec![], vec![]));
        }

        // Submit in parallel
        let (up_results, down_results) = tokio::join!(
            self.clob.post_orders(&up_orders),
            self.clob.post_orders(&down_orders),
        );

        let up_order_ids: Vec<String> = up_results?
            .iter()
            .filter_map(|r| r.get("orderID").and_then(|id| id.as_str()).map(|s| s.to_string()))
            .collect();

        let down_order_ids: Vec<String> = down_results?
            .iter()
            .filter_map(|r| r.get("orderID").and_then(|id| id.as_str()).map(|s| s.to_string()))
            .collect();

        info!("Submitted {} UP orders, {} DOWN orders", up_order_ids.len(), down_order_ids.len());

        Ok((up_order_ids, down_order_ids))
    }

    /// Check if position is profitable
    pub fn is_profitable(&self, position: &Position) -> bool {
        let total_cost = position.total_cost();
        let guaranteed_payout = position.guaranteed_payout();
        let min_profit = total_cost * self.config.min_spread_percent / dec!(100);

        guaranteed_payout > total_cost + min_profit
    }

    /// Calculate current spread from orderbooks
    pub fn calculate_spread(
        &self,
        up_orderbook: &Orderbook,
        down_orderbook: &Orderbook,
    ) -> Decimal {
        let up_ask = up_orderbook.asks.first()
            .and_then(|p| p.price.parse::<Decimal>().ok())
            .unwrap_or(dec!(0.50));

        let down_ask = down_orderbook.asks.first()
            .and_then(|p| p.price.parse::<Decimal>().ok())
            .unwrap_or(dec!(0.50));

        let combined = up_ask + down_ask;
        let spread = dec!(1) - combined;
        let spread_pct = spread / combined * dec!(100);

        debug!("UP ask: {}, DOWN ask: {}, Combined: {}, Spread: {}%", up_ask, down_ask, combined, spread_pct);

        spread_pct
    }

    /// Rebalance position if one side is over-filled
    ///
    /// This is CRITICAL for the arbitrage to work. If we have significantly more
    /// UP shares than DOWN shares (or vice versa), we need to aggressively fill
    /// the lagging side before the market closes.
    ///
    /// Strategy:
    /// - If imbalance > 20%: Place more aggressive (higher price) orders on lagging side
    /// - If imbalance > 40%: Market buy the lagging side to force balance
    pub async fn rebalance_if_needed(
        &self,
        market: &BtcMarket,
        position: &Position,
    ) -> Result<Option<Vec<String>>> {
        if position.is_balanced() {
            return Ok(None);
        }

        let diff = (position.up_shares - position.down_shares).abs();
        let avg = (position.up_shares + position.down_shares) / dec!(2);

        if avg.is_zero() {
            return Ok(None);
        }

        let imbalance_pct = diff / avg * dec!(100);
        warn!("Position imbalance: {}% (UP: {}, DOWN: {})", imbalance_pct, position.up_shares, position.down_shares);

        // Determine which side needs more fills
        let (lagging_side, lagging_token, needed_shares) = if position.up_shares > position.down_shares {
            (Side::Buy, &market.down_token_id, position.up_shares - position.down_shares)
        } else {
            (Side::Buy, &market.up_token_id, position.down_shares - position.up_shares)
        };

        // If imbalance > 20%, place aggressive limit orders
        if imbalance_pct > dec!(20) && imbalance_pct <= dec!(40) {
            info!("Placing aggressive rebalance orders for {} shares", needed_shares);

            // Get current orderbook to find best ask
            let orderbook = self.clob.get_orderbook(lagging_token).await?;
            let best_ask = self.get_best_ask(&orderbook);

            // Place orders at best ask - we want to get filled!
            let aggressive_price = best_ask - market.tick_size; // Just below best ask

            // Split into a few orders
            let orders_count = 3u32;
            let size_per_order = needed_shares / Decimal::from(orders_count);

            let mut order_ids = Vec::new();
            for _ in 0..orders_count {
                let order = self.signer.create_order(
                    lagging_token,
                    aggressive_price,
                    size_per_order,
                    lagging_side,
                    market.tick_size,
                    market.neg_risk,
                ).await?;

                if !self.config.dry_run {
                    if let Ok(result) = self.clob.post_order(&order).await {
                        if let Some(id) = result.get("orderID").and_then(|v| v.as_str()) {
                            order_ids.push(id.to_string());
                        }
                    }
                }
            }

            info!("Placed {} rebalance orders", order_ids.len());
            return Ok(Some(order_ids));
        }

        // If imbalance > 40%, this is critical - we might need market orders
        // But Polymarket only has limit orders, so place at best ask
        if imbalance_pct > dec!(40) {
            warn!("CRITICAL imbalance {}%! Placing aggressive orders at best ask", imbalance_pct);

            let orderbook = self.clob.get_orderbook(lagging_token).await?;
            let best_ask = self.get_best_ask(&orderbook);

            // Place at best ask to get immediate fill
            let order = self.signer.create_order(
                lagging_token,
                best_ask, // Match best ask for immediate fill
                needed_shares,
                lagging_side,
                market.tick_size,
                market.neg_risk,
            ).await?;

            if !self.config.dry_run {
                if let Ok(result) = self.clob.post_order(&order).await {
                    if let Some(id) = result.get("orderID").and_then(|v| v.as_str()) {
                        info!("Emergency rebalance order placed: {}", id);
                        return Ok(Some(vec![id.to_string()]));
                    }
                }
            }
        }

        Ok(None)
    }

    /// Spread sniping - aggressively capture large spread opportunities
    ///
    /// When the combined spread (1 - up_ask - down_ask) is larger than our target,
    /// we can place immediate orders at best ask to lock in profit instantly.
    ///
    /// This is the aggressive complement to the passive ladder strategy.
    pub async fn snipe_spread(
        &self,
        market: &BtcMarket,
        up_ask: Decimal,
        down_ask: Decimal,
    ) -> Result<Option<(Vec<String>, Vec<String>)>> {
        let combined = up_ask + down_ask;
        let spread_pct = (dec!(1) - combined) / combined * dec!(100);

        // Only snipe if spread exceeds target
        if spread_pct < self.config.target_spread_percent {
            return Ok(None);
        }

        info!("🎯 SNIPING spread {}%! UP@{}, DOWN@{}", spread_pct, up_ask, down_ask);

        if self.config.dry_run {
            info!("[DRY RUN] Would snipe spread");
            return Ok(None);
        }

        // Calculate how much to buy
        // We want equal shares on both sides
        // With $1200 budget: $600 each side
        let budget_per_side = self.config.max_position_usd / dec!(2);

        // Size = budget / price (shares we can afford)
        let up_shares = (budget_per_side / up_ask).round();
        let down_shares = (budget_per_side / down_ask).round();

        // Take the minimum to ensure balance
        let shares = up_shares.min(down_shares);

        info!("Sniping {} shares each side", shares);

        // Create orders at best ask for immediate fill
        let up_order = self.signer.create_order(
            &market.up_token_id,
            up_ask,
            shares,
            Side::Buy,
            market.tick_size,
            market.neg_risk,
        ).await?;

        let down_order = self.signer.create_order(
            &market.down_token_id,
            down_ask,
            shares,
            Side::Buy,
            market.tick_size,
            market.neg_risk,
        ).await?;

        // Submit both orders simultaneously
        let (up_result, down_result) = tokio::join!(
            self.clob.post_order(&up_order),
            self.clob.post_order(&down_order),
        );

        let mut up_ids = Vec::new();
        let mut down_ids = Vec::new();

        if let Ok(result) = up_result {
            if let Some(id) = result.get("orderID").and_then(|v| v.as_str()) {
                up_ids.push(id.to_string());
                info!("UP snipe order: {}", id);
            }
        }

        if let Ok(result) = down_result {
            if let Some(id) = result.get("orderID").and_then(|v| v.as_str()) {
                down_ids.push(id.to_string());
                info!("DOWN snipe order: {}", id);
            }
        }

        if !up_ids.is_empty() && !down_ids.is_empty() {
            let profit = (dec!(1) - combined) * shares;
            info!("🎯 Snipe successful! Potential profit: ${}", profit);
            Ok(Some((up_ids, down_ids)))
        } else {
            warn!("Snipe partially failed");
            Ok(None)
        }
    }

    /// Cancel all orders for a market
    pub async fn cancel_all_orders(&self, condition_id: &str) -> Result<()> {
        if self.config.dry_run {
            info!("[DRY RUN] Would cancel all orders for market {}", condition_id);
            return Ok(());
        }

        info!("Cancelling all orders for market {}...", condition_id);
        self.clob.cancel_market_orders(condition_id).await?;
        info!("All orders cancelled");
        Ok(())
    }

    /// Get CLOB client reference (for direct API access)
    pub fn clob(&self) -> &ClobClient {
        &self.clob
    }
}
