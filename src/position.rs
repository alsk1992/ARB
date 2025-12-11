use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use tracing::{debug, info};

use crate::types::{Position, Side, TradeFill};

/// Position manager - tracks fills and calculates P&L
pub struct PositionManager {
    positions: HashMap<String, Position>, // condition_id -> Position
    order_to_market: HashMap<String, (String, Side)>, // order_id -> (condition_id, side)
}

impl PositionManager {
    pub fn new() -> Self {
        Self {
            positions: HashMap::new(),
            order_to_market: HashMap::new(),
        }
    }

    /// Register orders so we can track fills
    pub fn register_orders(
        &mut self,
        condition_id: &str,
        up_order_ids: &[String],
        down_order_ids: &[String],
    ) {
        // Initialize position for this market
        self.positions.entry(condition_id.to_string())
            .or_insert_with(Position::default);

        // Map orders to their market and side
        for order_id in up_order_ids {
            self.order_to_market.insert(
                order_id.clone(),
                (condition_id.to_string(), Side::Buy), // UP side
            );
        }

        for order_id in down_order_ids {
            self.order_to_market.insert(
                order_id.clone(),
                (condition_id.to_string(), Side::Sell), // Using Sell to indicate DOWN
            );
        }

        info!("Registered {} UP orders and {} DOWN orders for market {}",
            up_order_ids.len(), down_order_ids.len(), condition_id);
    }

    /// Process a trade fill
    pub fn process_fill(&mut self, fill: &TradeFill) {
        let price: Decimal = fill.price.parse().unwrap_or_default();
        let size: Decimal = fill.size.parse().unwrap_or_default();
        let cost = price * size;

        // Try to find which market/side this fill belongs to
        if let Some((condition_id, side)) = self.order_to_market.get(&fill.order_id) {
            if let Some(position) = self.positions.get_mut(condition_id) {
                match side {
                    Side::Buy => {
                        // UP side
                        position.up_shares += size;
                        position.up_cost += cost;
                        info!("UP fill: {} shares @ {} = ${}", size, price, cost);
                    }
                    Side::Sell => {
                        // DOWN side (we used Sell to indicate DOWN)
                        position.down_shares += size;
                        position.down_cost += cost;
                        info!("DOWN fill: {} shares @ {} = ${}", size, price, cost);
                    }
                }

                debug!("Position update - UP: {} shares (${} cost), DOWN: {} shares (${} cost)",
                    position.up_shares, position.up_cost,
                    position.down_shares, position.down_cost);
            }
        } else {
            // Fill for unknown order - might be from asset_id matching
            debug!("Fill for unknown order: {}", fill.order_id);
        }
    }

    /// Process fill by asset_id (when we don't have order mapping)
    pub fn process_fill_by_asset(
        &mut self,
        condition_id: &str,
        asset_id: &str,
        up_token_id: &str,
        down_token_id: &str,
        price: Decimal,
        size: Decimal,
    ) {
        let cost = price * size;

        let position = self.positions.entry(condition_id.to_string())
            .or_insert_with(Position::default);

        if asset_id == up_token_id {
            position.up_shares += size;
            position.up_cost += cost;
            info!("UP fill: {} shares @ {} = ${}", size, price, cost);
        } else if asset_id == down_token_id {
            position.down_shares += size;
            position.down_cost += cost;
            info!("DOWN fill: {} shares @ {} = ${}", size, price, cost);
        }
    }

    /// Get position for a market
    pub fn get_position(&self, condition_id: &str) -> Option<&Position> {
        self.positions.get(condition_id)
    }

    /// Get or create position for a market
    pub fn get_or_create_position(&mut self, condition_id: &str) -> &mut Position {
        self.positions.entry(condition_id.to_string())
            .or_insert_with(Position::default)
    }

    /// Calculate unrealized P&L for a position
    pub fn calculate_pnl(&self, position: &Position) -> PnlReport {
        let total_cost = position.total_cost();
        let min_shares = position.min_shares();
        let guaranteed_payout = position.guaranteed_payout();
        let locked_profit = position.locked_profit();

        let excess_up = position.up_shares - min_shares;
        let excess_down = position.down_shares - min_shares;

        // Estimate value of excess shares (worst case = 0, best case = $1 each)
        // Use 0.5 as expected value
        let excess_value = (excess_up + excess_down) * dec!(0.5);

        let expected_pnl = locked_profit + excess_value;
        let roi_pct = if total_cost > dec!(0) {
            locked_profit / total_cost * dec!(100)
        } else {
            dec!(0)
        };

        PnlReport {
            total_cost,
            guaranteed_payout,
            locked_profit,
            excess_up_shares: excess_up,
            excess_down_shares: excess_down,
            expected_pnl,
            roi_pct,
        }
    }

    /// Print position summary
    pub fn print_summary(&self, condition_id: &str) {
        if let Some(position) = self.positions.get(condition_id) {
            let report = self.calculate_pnl(position);

            info!("=== Position Summary ===");
            info!("UP:   {} shares, ${} cost", position.up_shares, position.up_cost);
            info!("DOWN: {} shares, ${} cost", position.down_shares, position.down_cost);
            info!("Total cost: ${}", report.total_cost);
            info!("Guaranteed payout: ${}", report.guaranteed_payout);
            info!("Locked profit: ${} ({:.2}% ROI)", report.locked_profit, report.roi_pct);
            info!("Excess shares: UP={}, DOWN={}", report.excess_up_shares, report.excess_down_shares);
            info!("========================");
        }
    }

    /// Clear position for a market (after resolution)
    pub fn clear_position(&mut self, condition_id: &str) {
        self.positions.remove(condition_id);

        // Also remove order mappings for this market
        self.order_to_market.retain(|_, (cid, _)| cid != condition_id);
    }
}

#[derive(Debug, Clone)]
pub struct PnlReport {
    pub total_cost: Decimal,
    pub guaranteed_payout: Decimal,
    pub locked_profit: Decimal,
    pub excess_up_shares: Decimal,
    pub excess_down_shares: Decimal,
    pub expected_pnl: Decimal,
    pub roi_pct: Decimal,
}
