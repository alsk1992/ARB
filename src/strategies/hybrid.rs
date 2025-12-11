//! Strategy 5: Hybrid (Best of all worlds)
//!
//! Combines:
//! - Pure arb ladder (base position)
//! - Scalping when profitable
//! - Momentum detection for position sizing
//!
//! This is the "kitchen sink" strategy.

use async_trait::async_trait;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::VecDeque;

use super::{
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

const PRICE_HISTORY_LEN: usize = 10;

pub struct HybridStrategy {
    name: String,
    metrics: StrategyMetrics,

    // Config
    base_ladder_levels: usize,
    base_order_size: Decimal,
    take_profit_pct: Decimal,
    momentum_threshold: Decimal,
    max_position: Decimal,

    // State
    up_entry_avg: Decimal,
    down_entry_avg: Decimal,
    up_prices: VecDeque<Decimal>,
    down_prices: VecDeque<Decimal>,
    initial_orders_placed: bool,
}

impl HybridStrategy {
    pub fn new() -> Self {
        Self {
            name: "hybrid".to_string(),
            metrics: StrategyMetrics {
                strategy_name: "Hybrid".to_string(),
                ..Default::default()
            },
            base_ladder_levels: 15,
            base_order_size: dec!(15),
            take_profit_pct: dec!(8),
            momentum_threshold: dec!(0.02),
            max_position: dec!(800),
            up_entry_avg: dec!(0),
            down_entry_avg: dec!(0),
            up_prices: VecDeque::with_capacity(PRICE_HISTORY_LEN),
            down_prices: VecDeque::with_capacity(PRICE_HISTORY_LEN),
            initial_orders_placed: false,
        }
    }

    fn generate_base_ladder(&self, best_ask: Decimal, outcome: Outcome) -> Vec<StrategyOrder> {
        let mut orders = Vec::new();
        let tick = dec!(0.01);

        for i in 0..self.base_ladder_levels {
            let offset = tick * Decimal::from(i + 1);
            let price = best_ask - offset;

            if price <= dec!(0.01) || price >= dec!(0.99) {
                continue;
            }

            orders.push(StrategyOrder {
                side: OrderSide::Buy,
                outcome,
                price,
                size: self.base_order_size,
            });
        }

        orders
    }

    fn should_take_profit(&self, entry: Decimal, current_bid: Decimal) -> bool {
        if entry <= dec!(0) {
            return false;
        }
        let gain_pct = (current_bid - entry) / entry * dec!(100);
        gain_pct >= self.take_profit_pct
    }

    fn detect_momentum(&self) -> Option<Outcome> {
        if self.up_prices.len() < 5 || self.down_prices.len() < 5 {
            return None;
        }

        let up_recent: Vec<_> = self.up_prices.iter().rev().take(5).collect();
        let down_recent: Vec<_> = self.down_prices.iter().rev().take(5).collect();

        let up_trend = **up_recent.first().unwrap() - **up_recent.last().unwrap();
        let down_trend = **down_recent.first().unwrap() - **down_recent.last().unwrap();

        if up_trend > self.momentum_threshold && up_trend > down_trend {
            return Some(Outcome::Up);
        }
        if down_trend > self.momentum_threshold && down_trend > up_trend {
            return Some(Outcome::Down);
        }

        None
    }
}

#[async_trait]
impl TradingStrategy for HybridStrategy {
    fn name(&self) -> &str {
        &self.name
    }

    async fn on_market_start(
        &mut self,
        _market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal {
        self.up_entry_avg = dec!(0);
        self.down_entry_avg = dec!(0);
        self.up_prices.clear();
        self.down_prices.clear();
        self.initial_orders_placed = false;

        // Check spread is acceptable
        if let Some(spread) = state.spread_pct {
            if spread < dec!(2) {
                return StrategySignal::Hold;
            }
        }

        // Place base ladder on both sides
        if let (Some(up_ask), Some(down_ask)) = (state.up_best_ask, state.down_best_ask) {
            let mut orders = Vec::new();
            orders.extend(self.generate_base_ladder(up_ask, Outcome::Up));
            orders.extend(self.generate_base_ladder(down_ask, Outcome::Down));

            self.initial_orders_placed = true;

            // Record initial prices
            self.up_prices.push_back(up_ask);
            self.down_prices.push_back(down_ask);

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

        // Track prices for momentum
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

        // SCALPING: Check for take-profit opportunities
        if position.up_shares > dec!(0) && self.up_entry_avg > dec!(0) {
            if let Some(up_bid) = state.up_best_bid {
                if self.should_take_profit(self.up_entry_avg, up_bid) {
                    // Sell HALF to lock in profit, keep rest for arb
                    let sell_size = position.up_shares / dec!(2);
                    if sell_size >= dec!(1) {
                        orders.push(StrategyOrder {
                            side: OrderSide::Sell,
                            outcome: Outcome::Up,
                            price: up_bid,
                            size: sell_size,
                        });
                    }
                }
            }
        }

        if position.down_shares > dec!(0) && self.down_entry_avg > dec!(0) {
            if let Some(down_bid) = state.down_best_bid {
                if self.should_take_profit(self.down_entry_avg, down_bid) {
                    let sell_size = position.down_shares / dec!(2);
                    if sell_size >= dec!(1) {
                        orders.push(StrategyOrder {
                            side: OrderSide::Sell,
                            outcome: Outcome::Down,
                            price: down_bid,
                            size: sell_size,
                        });
                    }
                }
            }
        }

        // MOMENTUM: Add to winning side if strong trend
        if position.total_cost() < self.max_position {
            if let Some(trend) = self.detect_momentum() {
                match trend {
                    Outcome::Up => {
                        if let Some(up_ask) = state.up_best_ask {
                            orders.push(StrategyOrder {
                                side: OrderSide::Buy,
                                outcome: Outcome::Up,
                                price: up_ask,
                                size: self.base_order_size * dec!(2), // Larger on momentum
                            });
                        }
                    }
                    Outcome::Down => {
                        if let Some(down_ask) = state.down_best_ask {
                            orders.push(StrategyOrder {
                                side: OrderSide::Buy,
                                outcome: Outcome::Down,
                                price: down_ask,
                                size: self.base_order_size * dec!(2),
                            });
                        }
                    }
                }
            }
        }

        // REBALANCING: If too imbalanced, add to lagging side
        if !position.is_balanced() && position.total_cost() > dec!(0) {
            let (lagging_side, best_ask) = if position.up_shares < position.down_shares {
                (Outcome::Up, state.up_best_ask)
            } else {
                (Outcome::Down, state.down_best_ask)
            };

            if let Some(ask) = best_ask {
                let diff = (position.up_shares - position.down_shares).abs();
                if diff > dec!(10) {
                    orders.push(StrategyOrder {
                        side: OrderSide::Buy,
                        outcome: lagging_side,
                        price: ask,
                        size: diff * dec!(0.3),
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
        size: Decimal,
        position: &PositionState,
    ) -> StrategySignal {
        self.metrics.trades_executed += 1;

        // Update average entry prices
        match outcome {
            Outcome::Up => {
                if position.up_shares > dec!(0) {
                    self.up_entry_avg = position.up_cost / position.up_shares;
                }
            }
            Outcome::Down => {
                if position.down_shares > dec!(0) {
                    self.down_entry_avg = position.down_cost / position.down_shares;
                }
            }
        }

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
        // Cancel unfilled orders, hold positions for resolution
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

        self.up_entry_avg = dec!(0);
        self.down_entry_avg = dec!(0);
        self.up_prices.clear();
        self.down_prices.clear();
        self.initial_orders_placed = false;
    }
}
