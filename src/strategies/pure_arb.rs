//! Strategy 1: Pure Arbitrage (nobuyoshi005 style)
//!
//! Buy both UP and DOWN via limit orders.
//! Hold to resolution. Profit = $1 - combined_cost.
//! No selling, no scalping. Safest strategy.

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

use super::{
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

pub struct PureArbStrategy {
    name: String,
    metrics: StrategyMetrics,

    // Config
    ladder_levels: usize,
    order_size_per_level: Decimal,
    min_spread_pct: Decimal,
    target_spread_pct: Decimal,
    max_position: Decimal,

    // State
    orders_placed: bool,
}

impl PureArbStrategy {
    pub fn new() -> Self {
        Self {
            name: "pure_arb".to_string(),
            metrics: StrategyMetrics {
                strategy_name: "Pure Arbitrage".to_string(),
                ..Default::default()
            },
            ladder_levels: 30,
            order_size_per_level: dec!(20),
            min_spread_pct: dec!(2),
            target_spread_pct: dec!(4),
            max_position: dec!(1000),
            orders_placed: false,
        }
    }

    fn generate_ladder(&self, best_ask: Decimal, is_up: bool) -> Vec<StrategyOrder> {
        let mut orders = Vec::new();
        let tick = dec!(0.01);

        // Start just below best ask, spread down
        for i in 0..self.ladder_levels {
            let offset = tick * Decimal::from(i + 1);
            let price = best_ask - offset;

            if price <= dec!(0.01) || price >= dec!(0.99) {
                continue;
            }

            orders.push(StrategyOrder {
                side: OrderSide::Buy,
                outcome: if is_up { Outcome::Up } else { Outcome::Down },
                price,
                size: self.order_size_per_level,
            });
        }

        orders
    }
}

#[async_trait]
impl TradingStrategy for PureArbStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    async fn on_market_start(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal {
        self.orders_placed = false;

        // Check if spread is good enough
        if let Some(spread) = state.spread_pct {
            if spread < self.min_spread_pct {
                return StrategySignal::Hold;
            }
        }

        // Generate initial ladder
        if let (Some(up_ask), Some(down_ask)) = (state.up_best_ask, state.down_best_ask) {
            let mut orders = Vec::new();
            orders.extend(self.generate_ladder(up_ask, true));
            orders.extend(self.generate_ladder(down_ask, false));

            self.orders_placed = true;
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
        // If we haven't placed orders yet and spread is now good
        if !self.orders_placed {
            if let Some(spread) = state.spread_pct {
                if spread >= self.min_spread_pct {
                    if let (Some(up_ask), Some(down_ask)) = (state.up_best_ask, state.down_best_ask) {
                        let mut orders = Vec::new();
                        orders.extend(self.generate_ladder(up_ask, true));
                        orders.extend(self.generate_ladder(down_ask, false));
                        self.orders_placed = true;
                        return StrategySignal::PlaceOrders(orders);
                    }
                }
            }
        }

        // Check if position is too imbalanced - add orders to lagging side
        if position.total_cost() > dec!(0) && !position.is_balanced() {
            let imbalance = if position.up_shares > position.down_shares {
                (position.up_shares - position.down_shares) / position.up_shares
            } else {
                (position.down_shares - position.up_shares) / position.down_shares
            };

            // If >30% imbalanced, add aggressive orders on lagging side
            if imbalance > dec!(0.3) {
                let is_up_lagging = position.up_shares < position.down_shares;
                let best_ask = if is_up_lagging {
                    state.up_best_ask
                } else {
                    state.down_best_ask
                };

                if let Some(ask) = best_ask {
                    // Place at best ask for immediate fill
                    let order = StrategyOrder {
                        side: OrderSide::Buy,
                        outcome: if is_up_lagging { Outcome::Up } else { Outcome::Down },
                        price: ask,
                        size: (position.up_shares - position.down_shares).abs() * dec!(0.5),
                    };
                    return StrategySignal::PlaceOrders(vec![order]);
                }
            }
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
        // Pure arb: just hold, don't react to fills
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
        // Cancel all unfilled orders before resolution
        StrategySignal::CancelAll
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

        self.orders_placed = false;
    }
}
