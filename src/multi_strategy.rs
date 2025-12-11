//! Multi-Strategy Runner
//!
//! Runs all 5 strategies in parallel on the same market data.
//! Each strategy gets its own virtual position tracker.
//! Compare performance in real-time.

use anyhow::Result;
use parking_lot::Mutex;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::strategies::{
    hybrid::HybridStrategy,
    market_maker::MarketMakerStrategy,
    momentum::MomentumStrategy,
    pure_arb::PureArbStrategy,
    scalper::ScalperStrategy,
    MarketState, Outcome, OrderSide, PositionState, StrategyMetrics,
    StrategyOrder, StrategySignal, TradingStrategy,
};
use crate::types::BtcMarket;

/// Virtual position for each strategy
#[derive(Debug, Clone, Default)]
struct VirtualPosition {
    up_shares: Decimal,
    down_shares: Decimal,
    up_cost: Decimal,
    down_cost: Decimal,
    pending_orders: Vec<StrategyOrder>,
}

impl VirtualPosition {
    fn to_position_state(&self) -> PositionState {
        PositionState {
            up_shares: self.up_shares,
            down_shares: self.down_shares,
            up_cost: self.up_cost,
            down_cost: self.down_cost,
            up_avg_price: if self.up_shares > dec!(0) {
                self.up_cost / self.up_shares
            } else {
                dec!(0)
            },
            down_avg_price: if self.down_shares > dec!(0) {
                self.down_cost / self.down_shares
            } else {
                dec!(0)
            },
        }
    }

    fn process_fill(&mut self, outcome: Outcome, side: OrderSide, price: Decimal, size: Decimal) {
        match (outcome, side) {
            (Outcome::Up, OrderSide::Buy) => {
                self.up_shares += size;
                self.up_cost += price * size;
            }
            (Outcome::Up, OrderSide::Sell) => {
                let avg = if self.up_shares > dec!(0) {
                    self.up_cost / self.up_shares
                } else {
                    dec!(0)
                };
                self.up_shares -= size;
                self.up_cost -= avg * size;
            }
            (Outcome::Down, OrderSide::Buy) => {
                self.down_shares += size;
                self.down_cost += price * size;
            }
            (Outcome::Down, OrderSide::Sell) => {
                let avg = if self.down_shares > dec!(0) {
                    self.down_cost / self.down_shares
                } else {
                    dec!(0)
                };
                self.down_shares -= size;
                self.down_cost -= avg * size;
            }
        }
    }

    fn total_cost(&self) -> Decimal {
        self.up_cost + self.down_cost
    }

    fn calculate_pnl(&self) -> Decimal {
        // Profit = min(up, down) shares * $1 - total cost
        let min_shares = self.up_shares.min(self.down_shares);
        min_shares - self.total_cost()
    }
}

/// Comparison report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyComparison {
    pub timestamp: String,
    pub sessions_compared: u32,
    pub strategies: Vec<StrategyMetrics>,
    pub winner: String,
    pub winner_roi: Decimal,
}

pub struct MultiStrategyRunner {
    strategies: Vec<Box<dyn TradingStrategy>>,
    positions: HashMap<String, VirtualPosition>,
    comparisons: Vec<StrategyComparison>,
}

impl MultiStrategyRunner {
    pub fn new() -> Self {
        let strategies: Vec<Box<dyn TradingStrategy>> = vec![
            Box::new(PureArbStrategy::new()),
            Box::new(ScalperStrategy::new()),
            Box::new(MarketMakerStrategy::new()),
            Box::new(MomentumStrategy::new()),
            Box::new(HybridStrategy::new()),
        ];

        let mut positions = HashMap::new();
        for s in &strategies {
            positions.insert(s.name().to_string(), VirtualPosition::default());
        }

        Self {
            strategies,
            positions,
            comparisons: Vec::new(),
        }
    }

    pub fn strategy_names(&self) -> Vec<String> {
        self.strategies.iter().map(|s| s.name().to_string()).collect()
    }

    /// Called when market starts
    pub async fn on_market_start(&mut self, market: &BtcMarket, state: &MarketState) {
        // Reset positions for new market
        for (_, pos) in self.positions.iter_mut() {
            *pos = VirtualPosition::default();
        }

        // Collect signals first
        let mut signals = Vec::new();
        for strategy in &mut self.strategies {
            let signal = strategy.on_market_start(market, state).await;
            signals.push((strategy.name().to_string(), signal));
        }

        // Then process them
        for (name, signal) in signals {
            self.process_signal(&name, signal, state);
        }

        info!("╔═══════════════════════════════════════════════════════╗");
        info!("║       MULTI-STRATEGY DRY RUN - {} Strategies       ║", self.strategies.len());
        info!("╚═══════════════════════════════════════════════════════╝");
        for s in &self.strategies {
            info!("  • {}", s.name());
        }
    }

    /// Called on orderbook update
    pub async fn on_orderbook_update(&mut self, market: &BtcMarket, state: &MarketState) {
        // Simulate fills based on price movements
        self.simulate_fills(state);

        // Collect positions first
        let positions: Vec<_> = self.strategies.iter()
            .map(|s| {
                let pos = self.positions
                    .get(s.name())
                    .map(|p| p.to_position_state())
                    .unwrap_or_default();
                (s.name().to_string(), pos)
            })
            .collect();

        // Collect signals
        let mut signals = Vec::new();
        for (i, strategy) in self.strategies.iter_mut().enumerate() {
            let position = &positions[i].1;
            let signal = strategy.on_orderbook_update(market, state, position).await;
            signals.push((strategy.name().to_string(), signal));
        }

        // Process signals
        for (name, signal) in signals {
            self.process_signal(&name, signal, state);
        }
    }

    /// Called on fill (in real mode, this would come from WebSocket)
    pub async fn on_fill(
        &mut self,
        market: &BtcMarket,
        outcome: Outcome,
        price: Decimal,
        size: Decimal,
    ) {
        // In simulation, fills are handled in simulate_fills
        // This is for real fills if needed
        for strategy in &mut self.strategies {
            let position = self.positions
                .get(strategy.name())
                .map(|p| p.to_position_state())
                .unwrap_or_default();

            let _signal = strategy.on_fill(market, outcome, price, size, &position).await;
        }
    }

    /// Called every 30 seconds
    pub async fn on_tick(&mut self, market: &BtcMarket, state: &MarketState) {
        // Collect positions first
        let positions: Vec<_> = self.strategies.iter()
            .map(|s| {
                let pos = self.positions
                    .get(s.name())
                    .map(|p| p.to_position_state())
                    .unwrap_or_default();
                (s.name().to_string(), pos)
            })
            .collect();

        // Collect signals
        let mut signals = Vec::new();
        for (i, strategy) in self.strategies.iter_mut().enumerate() {
            let position = &positions[i].1;
            let signal = strategy.on_tick(market, state, position).await;
            signals.push((strategy.name().to_string(), signal));
        }

        // Process signals
        for (name, signal) in signals {
            self.process_signal(&name, signal, state);
        }

        // Print current standings
        self.print_standings();
    }

    /// Called before resolution
    pub async fn on_pre_resolution(&mut self, market: &BtcMarket) {
        for strategy in &mut self.strategies {
            let position = self.positions
                .get(strategy.name())
                .map(|p| p.to_position_state())
                .unwrap_or_default();

            let signal = strategy.on_pre_resolution(market, &position).await;

            // For pre-resolution, cancel all orders means clear pending
            if matches!(signal, StrategySignal::CancelAll) {
                if let Some(pos) = self.positions.get_mut(strategy.name()) {
                    pos.pending_orders.clear();
                }
            }
        }
    }

    /// Called when market resolves
    pub fn on_market_end(&mut self, winning_outcome: Outcome) {
        info!("\n╔═══════════════════════════════════════════════════════╗");
        info!("║              SESSION RESULTS                          ║");
        info!("╚═══════════════════════════════════════════════════════╝");
        info!("Winner: {:?}\n", winning_outcome);

        let mut results: Vec<(String, Decimal, Decimal, Decimal)> = Vec::new();

        for strategy in &mut self.strategies {
            let pos = self.positions.get(strategy.name()).unwrap();

            // Calculate final P&L
            let winning_shares = match winning_outcome {
                Outcome::Up => pos.up_shares,
                Outcome::Down => pos.down_shares,
            };
            let payout = winning_shares; // $1 per share
            let profit = payout - pos.total_cost();
            let roi = if pos.total_cost() > dec!(0) {
                profit / pos.total_cost() * dec!(100)
            } else {
                dec!(0)
            };

            results.push((strategy.name().to_string(), profit, pos.total_cost(), roi));

            // Record to strategy
            strategy.record_session_result(profit, pos.total_cost());
        }

        // Sort by profit descending
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

        info!("┌─────────────────┬──────────────┬──────────────┬──────────┐");
        info!("│ Strategy        │ Profit       │ Cost         │ ROI %    │");
        info!("├─────────────────┼──────────────┼──────────────┼──────────┤");
        for (name, profit, cost, roi) in &results {
            let profit_str = if *profit >= dec!(0) {
                format!("+${:.2}", profit)
            } else {
                format!("-${:.2}", profit.abs())
            };
            info!(
                "│ {:15} │ {:>12} │ ${:>10.2} │ {:>7.2}% │",
                name, profit_str, cost, roi
            );
        }
        info!("└─────────────────┴──────────────┴──────────────┴──────────┘");

        // Winner
        if let Some((winner, profit, _, roi)) = results.first() {
            info!("\n🏆 WINNER: {} (+${:.2}, {:.2}% ROI)", winner, profit, roi);
        }

        // Store comparison
        let comparison = StrategyComparison {
            timestamp: chrono::Utc::now().to_rfc3339(),
            sessions_compared: self.strategies[0].get_metrics().sessions_run,
            strategies: self.strategies.iter().map(|s| s.get_metrics()).collect(),
            winner: results.first().map(|r| r.0.clone()).unwrap_or_default(),
            winner_roi: results.first().map(|r| r.3).unwrap_or_default(),
        };
        self.comparisons.push(comparison);
    }

    /// Get cumulative comparison across all sessions
    pub fn get_cumulative_comparison(&self) -> Vec<StrategyMetrics> {
        self.strategies.iter().map(|s| s.get_metrics()).collect()
    }

    /// Print current standings (mid-session)
    fn print_standings(&self) {
        info!("\n--- Current P&L (unrealized) ---");
        for (name, pos) in &self.positions {
            let pnl = pos.calculate_pnl();
            let symbol = if pnl >= dec!(0) { "+" } else { "" };
            info!(
                "  {} | UP: {:.0} @ ${:.2} | DOWN: {:.0} @ ${:.2} | P&L: {}${:.2}",
                name,
                pos.up_shares,
                pos.up_cost,
                pos.down_shares,
                pos.down_cost,
                symbol,
                pnl
            );
        }
    }

    /// Process strategy signal
    fn process_signal(&mut self, strategy_name: &str, signal: StrategySignal, state: &MarketState) {
        match signal {
            StrategySignal::PlaceOrders(orders) => {
                if let Some(pos) = self.positions.get_mut(strategy_name) {
                    for order in orders {
                        // In dry run, immediately "fill" market orders
                        let fill_price = match (&order.outcome, &order.side) {
                            (Outcome::Up, OrderSide::Buy) => state.up_best_ask.unwrap_or(dec!(0.50)),
                            (Outcome::Up, OrderSide::Sell) => state.up_best_bid.unwrap_or(dec!(0.50)),
                            (Outcome::Down, OrderSide::Buy) => state.down_best_ask.unwrap_or(dec!(0.50)),
                            (Outcome::Down, OrderSide::Sell) => state.down_best_bid.unwrap_or(dec!(0.50)),
                        };

                        // Check if order would fill
                        let would_fill = match order.side {
                            OrderSide::Buy => order.price >= fill_price,
                            OrderSide::Sell => order.price <= fill_price,
                        };

                        if would_fill {
                            // Simulate immediate fill
                            pos.process_fill(order.outcome, order.side, fill_price, order.size);
                        } else {
                            // Add to pending orders
                            pos.pending_orders.push(order);
                        }
                    }
                }
            }
            StrategySignal::CancelAll => {
                if let Some(pos) = self.positions.get_mut(strategy_name) {
                    pos.pending_orders.clear();
                }
            }
            StrategySignal::ExitPosition => {
                if let Some(pos) = self.positions.get_mut(strategy_name) {
                    // Sell everything at current bid
                    if pos.up_shares > dec!(0) {
                        let price = state.up_best_bid.unwrap_or(dec!(0.50));
                        pos.process_fill(Outcome::Up, OrderSide::Sell, price, pos.up_shares);
                    }
                    if pos.down_shares > dec!(0) {
                        let price = state.down_best_bid.unwrap_or(dec!(0.50));
                        pos.process_fill(Outcome::Down, OrderSide::Sell, price, pos.down_shares);
                    }
                }
            }
            StrategySignal::Hold => {}
        }
    }

    /// Simulate fills based on price movement
    fn simulate_fills(&mut self, state: &MarketState) {
        for (_, pos) in self.positions.iter_mut() {
            let mut filled_indices = Vec::new();

            for (i, order) in pos.pending_orders.iter().enumerate() {
                let current_price = match (&order.outcome, &order.side) {
                    (Outcome::Up, OrderSide::Buy) => state.up_best_ask,
                    (Outcome::Up, OrderSide::Sell) => state.up_best_bid,
                    (Outcome::Down, OrderSide::Buy) => state.down_best_ask,
                    (Outcome::Down, OrderSide::Sell) => state.down_best_bid,
                };

                if let Some(price) = current_price {
                    let would_fill = match order.side {
                        OrderSide::Buy => order.price >= price,
                        OrderSide::Sell => order.price <= price,
                    };

                    if would_fill {
                        filled_indices.push((i, price));
                    }
                }
            }

            // Process fills (in reverse to maintain indices)
            for (i, fill_price) in filled_indices.into_iter().rev() {
                let order = pos.pending_orders.remove(i);
                pos.process_fill(order.outcome, order.side, fill_price, order.size);
            }
        }
    }

    /// Save comparison results to file
    pub fn save_results(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.comparisons)?;
        std::fs::write(path, json)?;
        info!("Saved comparison results to {}", path);
        Ok(())
    }
}
