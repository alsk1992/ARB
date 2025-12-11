//! Strategy 4: Momentum
//!
//! Follow the trend. If UP is rising, buy more UP.
//! If DOWN is rising, buy more DOWN.
//! Uses recent price movement to predict direction.

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::VecDeque;

use super::{
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

const PRICE_HISTORY_LEN: usize = 20;

pub struct MomentumStrategy {
    name: String,
    metrics: StrategyMetrics,

    // Config
    momentum_threshold: Decimal, // Min price change to trigger
    position_size: Decimal,
    max_position: Decimal,

    // Price history
    up_prices: VecDeque<Decimal>,
    down_prices: VecDeque<Decimal>,
}

impl MomentumStrategy {
    pub fn new() -> Self {
        Self {
            name: "momentum".to_string(),
            metrics: StrategyMetrics {
                strategy_name: "Momentum".to_string(),
                ..Default::default()
            },
            momentum_threshold: dec!(0.03), // 3 cent move
            position_size: dec!(50),
            max_position: dec!(500),
            up_prices: VecDeque::with_capacity(PRICE_HISTORY_LEN),
            down_prices: VecDeque::with_capacity(PRICE_HISTORY_LEN),
        }
    }

    fn calculate_momentum(&self, prices: &VecDeque<Decimal>) -> Option<Decimal> {
        if prices.len() < 5 {
            return None;
        }

        // Simple momentum: current - average of last 5
        let recent: Vec<_> = prices.iter().rev().take(5).collect();
        let avg: Decimal = recent.iter().copied().copied().sum::<Decimal>() / dec!(5);
        let current = *prices.back()?;

        Some(current - avg)
    }

    fn detect_trend(&self) -> Option<Outcome> {
        let up_momentum = self.calculate_momentum(&self.up_prices)?;
        let down_momentum = self.calculate_momentum(&self.down_prices)?;

        // If UP is rising more than threshold, trend is UP
        if up_momentum > self.momentum_threshold && up_momentum > down_momentum {
            return Some(Outcome::Up);
        }

        // If DOWN is rising more than threshold, trend is DOWN
        if down_momentum > self.momentum_threshold && down_momentum > up_momentum {
            return Some(Outcome::Down);
        }

        None
    }
}

#[async_trait]
impl TradingStrategy for MomentumStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    async fn on_market_start(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal {
        self.up_prices.clear();
        self.down_prices.clear();

        // Record initial prices
        if let Some(up_ask) = state.up_best_ask {
            self.up_prices.push_back(up_ask);
        }
        if let Some(down_ask) = state.down_best_ask {
            self.down_prices.push_back(down_ask);
        }

        // Start with small positions on both sides (arb base)
        if let (Some(up_ask), Some(down_ask)) = (state.up_best_ask, state.down_best_ask) {
            let orders = vec![
                StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Up,
                    price: up_ask - dec!(0.02),
                    size: self.position_size / dec!(2),
                },
                StrategyOrder {
                    side: OrderSide::Buy,
                    outcome: Outcome::Down,
                    price: down_ask - dec!(0.02),
                    size: self.position_size / dec!(2),
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
        // Record prices
        if let Some(up_ask) = state.up_best_ask {
            self.up_prices.push_back(up_ask);
            if self.up_prices.len() > PRICE_HISTORY_LEN {
                self.up_prices.pop_front();
            }
        }
        if let Some(down_ask) = state.down_best_ask {
            self.down_prices.push_back(down_ask);
            if self.down_prices.len() > PRICE_HISTORY_LEN {
                self.down_prices.pop_front();
            }
        }

        // Check if we should add to position based on momentum
        if position.total_cost() >= self.max_position {
            return StrategySignal::Hold;
        }

        if let Some(trend) = self.detect_trend() {
            let order = match trend {
                Outcome::Up => {
                    if let Some(up_ask) = state.up_best_ask {
                        Some(StrategyOrder {
                            side: OrderSide::Buy,
                            outcome: Outcome::Up,
                            price: up_ask, // Market buy
                            size: self.position_size,
                        })
                    } else {
                        None
                    }
                }
                Outcome::Down => {
                    if let Some(down_ask) = state.down_best_ask {
                        Some(StrategyOrder {
                            side: OrderSide::Buy,
                            outcome: Outcome::Down,
                            price: down_ask,
                            size: self.position_size,
                        })
                    } else {
                        None
                    }
                }
            };

            if let Some(o) = order {
                return StrategySignal::PlaceOrders(vec![o]);
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
        // Hold to resolution - we want the arb profit
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

        self.up_prices.clear();
        self.down_prices.clear();
    }
}
