//! Strategy 3: Market Maker
//!
//! Post both buy and sell orders on both sides.
//! Capture the bid-ask spread repeatedly.
//! Requires tight spreads and good liquidity.

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::{
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

pub struct MarketMakerStrategy {
    name: String,
    metrics: StrategyMetrics,

    // Config
    spread_to_capture: Decimal,  // How much spread to capture (e.g., 2¢)
    order_size: Decimal,
    max_inventory: Decimal,      // Max shares to hold on one side

    // State
    has_active_orders: bool,
}

impl MarketMakerStrategy {
    pub fn new() -> Self {
        Self {
            name: "market_maker".to_string(),
            metrics: StrategyMetrics {
                strategy_name: "Market Maker".to_string(),
                ..Default::default()
            },
            spread_to_capture: dec!(0.02),  // 2 cents
            order_size: dec!(25),
            max_inventory: dec!(200),
            has_active_orders: false,
        }
    }

    fn generate_mm_orders(&self, state: &MarketState, position: &PositionState) -> Vec<StrategyOrder> {
        let mut orders = Vec::new();

        // UP side market making
        if let (Some(up_bid), Some(up_ask)) = (state.up_best_bid, state.up_best_ask) {
            let mid = (up_bid + up_ask) / dec!(2);

            // Buy order below mid
            if position.up_shares < self.max_inventory {
                orders.push(StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Up,
                    price: mid - self.spread_to_capture / dec!(2),
                    size: self.order_size,
                });
            }

            // Sell order above mid (if we have shares)
            if position.up_shares > dec!(0) {
                orders.push(StrategyOrder {
                    side: OrderSide::Sell,
                    outcome: Outcome::Up,
                    price: mid + self.spread_to_capture / dec!(2),
                    size: position.up_shares.min(self.order_size),
                });
            }
        }

        // DOWN side market making
        if let (Some(down_bid), Some(down_ask)) = (state.down_best_bid, state.down_best_ask) {
            let mid = (down_bid + down_ask) / dec!(2);

            // Buy order below mid
            if position.down_shares < self.max_inventory {
                orders.push(StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Down,
                    price: mid - self.spread_to_capture / dec!(2),
                    size: self.order_size,
                });
            }

            // Sell order above mid (if we have shares)
            if position.down_shares > dec!(0) {
                orders.push(StrategyOrder {
                    side: OrderSide::Sell,
                    outcome: Outcome::Down,
                    price: mid + self.spread_to_capture / dec!(2),
                    size: position.down_shares.min(self.order_size),
                });
            }
        }

        orders
    }
}

#[async_trait]
impl TradingStrategy for MarketMakerStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    async fn on_market_start(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal {
        self.has_active_orders = false;

        let orders = self.generate_mm_orders(state, &PositionState::default());
        if !orders.is_empty() {
            self.has_active_orders = true;
            return StrategySignal::PlaceOrders(orders);
        }

        StrategySignal::Hold
    }

    async fn on_orderbook_update(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
        position: &PositionState,
    ) -> StrategySignal {
        // Continuously adjust quotes based on market
        let orders = self.generate_mm_orders(state, position);
        if !orders.is_empty() {
            return StrategySignal::PlaceOrders(orders);
        }

        StrategySignal::Hold
    }

    async fn on_fill(
        &mut self,
        _market: &BtcMarket,
        _outcome: Outcome,
        _price: Decimal,
        _size: Decimal,
        _position: &PositionState,
    ) -> StrategySignal {
        self.metrics.trades_executed += 1;
        // After a fill, we'll requote on next orderbook update
        StrategySignal::Hold
    }

    async fn on_tick(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
        position: &PositionState,
    ) -> StrategySignal {
        // Refresh quotes periodically
        let orders = self.generate_mm_orders(state, position);
        if !orders.is_empty() {
            return StrategySignal::PlaceOrders(orders);
        }

        StrategySignal::Hold
    }

    async fn on_pre_resolution(
        &mut self,
        _market: &BtcMarket,
        position: &PositionState,
    ) -> StrategySignal {
        // Before resolution, try to flatten inventory
        // If we have more of one side, sell excess
        let mut orders = Vec::new();

        if position.up_shares > position.down_shares {
            let excess = position.up_shares - position.down_shares;
            if excess > dec!(1) {
                // Sell excess UP at any price to balance
                orders.push(StrategyOrder {
                    side: OrderSide::Sell,
                    outcome: Outcome::Up,
                    price: dec!(0.01), // Will get best bid
                    size: excess,
                });
            }
        } else if position.down_shares > position.up_shares {
            let excess = position.down_shares - position.up_shares;
            if excess > dec!(1) {
                orders.push(StrategyOrder {
                    side: OrderSide::Sell,
                    outcome: Outcome::Down,
                    price: dec!(0.01),
                    size: excess,
                });
            }
        }

        if orders.is_empty() {
            StrategySignal::CancelAll
        } else {
            StrategySignal::PlaceOrders(orders)
        }
    }

    fn get_metrics(&self) -> StrategyMetrics {
        self.metrics.clone()
    }

    fn record_session_result(&mut self, profit: Decimal, cost: Decimal) {
        self.metrics.sessions_run += 1;
        self.metrics.total_profit += profit;
        self.metrics.total_cost += cost;

        if profit > dec!(0) {
            self.metrics.win_count += 1;
        } else {
            self.metrics.loss_count += 1;
        }

        if self.metrics.total_cost > dec!(0) {
            self.metrics.roi_percent =
                self.metrics.total_profit / self.metrics.total_cost * dec!(100);
        }

        self.metrics.avg_profit_per_session =
            self.metrics.total_profit / Decimal::from(self.metrics.sessions_run);

        self.has_active_orders = false;
    }
}
