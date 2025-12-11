//! Multiple trading strategies for A/B testing
//!
//! Run all strategies in parallel on DRY_RUN mode,
//! compare performance, deploy the winner.

pub mod pure_arb;
pub mod scalper;
pub mod market_maker;
pub mod momentum;
pub mod hybrid;

use async_trait::async_trait;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use parking_lot::Mutex;

use crate::types::BtcMarket;
use crate::orderbook::OrderbookManager;

/// Strategy performance metrics
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyMetrics {
    pub strategy_name: String,
    pub sessions_run: u32,
    pub total_profit: Decimal,
    pub total_cost: Decimal,
    pub roi_percent: Decimal,
    pub win_count: u32,
    pub loss_count: u32,
    pub trades_executed: u32,
    pub avg_profit_per_session: Decimal,
    pub max_drawdown: Decimal,
    pub sharpe_ratio: Option<f64>,
}

/// Order to be placed
#[derive(Debug, Clone)]
pub struct StrategyOrder {
    pub side: OrderSide,
    pub outcome: Outcome,
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Up,
    Down,
}

/// Signal from strategy
#[derive(Debug, Clone)]
pub enum StrategySignal {
    /// Place these orders
    PlaceOrders(Vec<StrategyOrder>),
    /// Cancel all orders
    CancelAll,
    /// Do nothing
    Hold,
    /// Exit position (sell everything)
    ExitPosition,
}

/// Current position state passed to strategy
#[derive(Debug, Clone, Default)]
pub struct PositionState {
    pub up_shares: Decimal,
    pub down_shares: Decimal,
    pub up_cost: Decimal,
    pub down_cost: Decimal,
    pub up_avg_price: Decimal,
    pub down_avg_price: Decimal,
}

impl PositionState {
    pub fn total_cost(&self) -> Decimal {
        self.up_cost + self.down_cost
    }

    pub fn min_shares(&self) -> Decimal {
        self.up_shares.min(self.down_shares)
    }

    pub fn is_balanced(&self) -> bool {
        if self.up_shares == Decimal::ZERO || self.down_shares == Decimal::ZERO {
            return true;
        }
        let ratio = self.up_shares / self.down_shares;
        ratio >= Decimal::from_str_exact("0.8").unwrap()
            && ratio <= Decimal::from_str_exact("1.2").unwrap()
    }
}

/// Market state passed to strategy
#[derive(Debug, Clone)]
pub struct MarketState {
    pub up_best_bid: Option<Decimal>,
    pub up_best_ask: Option<Decimal>,
    pub down_best_bid: Option<Decimal>,
    pub down_best_ask: Option<Decimal>,
    pub combined_ask: Option<Decimal>,
    pub spread_pct: Option<Decimal>,
    pub seconds_to_resolution: i64,
    pub minute_of_period: f64,
}

/// Trait all strategies must implement
#[async_trait]
pub trait TradingStrategy: Send + Sync {
    /// Strategy name for logging/comparison
    fn name(&self) -> &str;

    /// Called when entering a new market
    async fn on_market_start(
        &mut self,
        market: &BtcMarket,
        state: &MarketState,
    ) -> StrategySignal;

    /// Called on every orderbook update
    async fn on_orderbook_update(
        &mut self,
        market: &BtcMarket,
        state: &MarketState,
        position: &PositionState,
    ) -> StrategySignal;

    /// Called when an order is filled
    async fn on_fill(
        &mut self,
        market: &BtcMarket,
        outcome: Outcome,
        price: Decimal,
        size: Decimal,
        position: &PositionState,
    ) -> StrategySignal;

    /// Called periodically (every 30 seconds)
    async fn on_tick(
        &mut self,
        market: &BtcMarket,
        state: &MarketState,
        position: &PositionState,
    ) -> StrategySignal;

    /// Called before market resolution (2 min before end)
    async fn on_pre_resolution(
        &mut self,
        market: &BtcMarket,
        position: &PositionState,
    ) -> StrategySignal;

    /// Get current metrics
    fn get_metrics(&self) -> StrategyMetrics;

    /// Update metrics after session ends
    fn record_session_result(&mut self, profit: Decimal, cost: Decimal);
}

/// Helper to parse decimal from string
trait DecimalExt {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error>;
}

impl DecimalExt for Decimal {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error> {
        s.parse()
    }
}
