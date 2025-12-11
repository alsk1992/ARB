//! Strategy 2: Scalper (0xf247... style)
//!
//! Buy low, sell high within the same market.
//! Take profits on price movements.
//! More active, higher risk/reward.

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::{
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

pub struct ScalperStrategy {
    name: String,
    metrics: StrategyMetrics,

    // Config
    take_profit_pct: Decimal,    // Sell when price up by this %
    stop_loss_pct: Decimal,      // Sell when price down by this %
    position_size: Decimal,
    max_position: Decimal,

    // Track entry prices
    up_entry_price: Option<Decimal>,
    down_entry_price: Option<Decimal>,
}

impl ScalperStrategy {
    pub fn new() -> Self {
        Self {
            name: "scalper".to_string(),
            metrics: StrategyMetrics {
                strategy_name: "Scalper".to_string(),
                ..Default::default()
            },
            take_profit_pct: dec!(5),   // Take profit at 5% gain
            stop_loss_pct: dec!(10),    // Stop loss at 10% loss
            position_size: dec!(50),
            max_position: dec!(500),
            up_entry_price: None,
            down_entry_price: None,
        }
    }

    fn should_take_profit(&self, entry: Decimal, current_bid: Decimal) -> bool {
        if entry == dec!(0) {
            return false;
        }
        let gain_pct = (current_bid - entry) / entry * dec!(100);
        gain_pct >= self.take_profit_pct
    }

    fn should_stop_loss(&self, entry: Decimal, current_bid: Decimal) -> bool {
        if entry == dec!(0) {
            return false;
        }
        let loss_pct = (entry - current_bid) / entry * dec!(100);
        loss_pct >= self.stop_loss_pct
    }
}

#[async_trait]
impl TradingStrategy for ScalperStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    async fn on_market_start(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal {
        self.up_entry_price = None;
        self.down_entry_price = None;

        // Enter both sides with small position
        if let (Some(up_ask), Some(down_ask)) = (state.up_best_ask, state.down_best_ask) {
            let orders = vec![
                StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Up,
                    price: up_ask - dec!(0.01), // Just below ask
                    size: self.position_size,
                },
                StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Down,
                    price: down_ask - dec!(0.01),
                    size: self.position_size,
                },
            ];
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
        let mut orders = Vec::new();

        // Check UP side for take profit / stop loss
        if position.up_shares > dec!(0) {
            if let (Some(up_bid), Some(entry)) = (state.up_best_bid, self.up_entry_price) {
                if self.should_take_profit(entry, up_bid) {
                    // Take profit - sell UP
                    orders.push(StrategyOrder {
                        side: OrderSide::Sell,
                        outcome: Outcome::Up,
                        price: up_bid,
                        size: position.up_shares,
                    });
                    self.up_entry_price = None;
                } else if self.should_stop_loss(entry, up_bid) {
                    // Stop loss - sell UP
                    orders.push(StrategyOrder {
                        side: OrderSide::Sell,
                        outcome: Outcome::Up,
                        price: up_bid,
                        size: position.up_shares,
                    });
                    self.up_entry_price = None;
                }
            }
        }

        // Check DOWN side for take profit / stop loss
        if position.down_shares > dec!(0) {
            if let (Some(down_bid), Some(entry)) = (state.down_best_bid, self.down_entry_price) {
                if self.should_take_profit(entry, down_bid) {
                    // Take profit - sell DOWN
                    orders.push(StrategyOrder {
                        side: OrderSide::Sell,
                        outcome: Outcome::Down,
                        price: down_bid,
                        size: position.down_shares,
                    });
                    self.down_entry_price = None;
                } else if self.should_stop_loss(entry, down_bid) {
                    // Stop loss - sell DOWN
                    orders.push(StrategyOrder {
                        side: OrderSide::Sell,
                        outcome: Outcome::Down,
                        price: down_bid,
                        size: position.down_shares,
                    });
                    self.down_entry_price = None;
                }
            }
        }

        // Re-enter if we exited and price looks good
        if position.up_shares == dec!(0) && position.total_cost() < self.max_position {
            if let Some(up_ask) = state.up_best_ask {
                if up_ask < dec!(0.50) {
                    // Good price to buy UP
                    orders.push(StrategyOrder {
                        side: OrderSide::Buy,
                        outcome: Outcome::Up,
                        price: up_ask,
                        size: self.position_size,
                    });
                }
            }
        }

        if position.down_shares == dec!(0) && position.total_cost() < self.max_position {
            if let Some(down_ask) = state.down_best_ask {
                if down_ask < dec!(0.50) {
                    // Good price to buy DOWN
                    orders.push(StrategyOrder {
                        side: OrderSide::Buy,
                        outcome: Outcome::Down,
                        price: down_ask,
                        size: self.position_size,
                    });
                }
            }
        }

        if orders.is_empty() {
            StrategySignal::Hold
        } else {
            StrategySignal::PlaceOrders(orders)
        }
    }

    async fn on_fill(
        &mut self,
        _market: &BtcMarket,
        outcome: Outcome,
        price: Decimal,
        _size: Decimal,
        _position: &PositionState,
    ) -> StrategySignal {
        // Track entry price on buys
        match outcome {
            Outcome::Up => {
                if self.up_entry_price.is_none() {
                    self.up_entry_price = Some(price);
                }
            }
            Outcome::Down => {
                if self.down_entry_price.is_none() {
                    self.down_entry_price = Some(price);
                }
            }
        }
        self.metrics.trades_executed += 1;
        StrategySignal::Hold
    }

    async fn on_tick(
        &mut self,
        _market: &BtcMarket,
        _state: &MarketState,
        _position: &PositionState,
    ) -> StrategySignal {
        StrategySignal::Hold
    }

    async fn on_pre_resolution(
        &mut self,
        _market: &BtcMarket,
        _position: &PositionState,
    ) -> StrategySignal {
        // Don't cancel - let positions ride to resolution
        // This way we still get arb profit if we're holding both sides
        StrategySignal::Hold
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

        self.up_entry_price = None;
        self.down_entry_price = None;
    }
}
