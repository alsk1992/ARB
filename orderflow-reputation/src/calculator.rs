use anyhow::{Result};
use rust_decimal::Decimal;
use rust_decimal::prelude::*;
use sqlx::{PgPool, Row};
use tracing::{debug, info, warn};

use crate::models::{ReputationScore};

pub struct ReputationCalculator {
    db: PgPool,
}

impl ReputationCalculator {
    pub fn new(db: PgPool) -> Self {
        Self { db }
    }

    /// Calculate reputation for all wallets with recent activity
    pub async fn calculate_all_wallets(&self) -> Result<usize> {
        // Get all wallets with trades in the last 30 days
        let wallets: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT wallet_address FROM orderflow_trades WHERE timestamp > NOW() - INTERVAL '30 days'"
        )
        .fetch_all(&self.db)
        .await?;

        info!("📊 Calculating reputation for {} wallets", wallets.len());

        let mut updated_count = 0;

        for wallet in &wallets {
            match self.calculate_wallet_reputation(wallet).await {
                Ok(_) => {
                    updated_count += 1;
                    if updated_count % 100 == 0 {
                        info!("Progress: {}/{} wallets updated", updated_count, wallets.len());
                    }
                }
                Err(e) => {
                    warn!("Failed to calculate reputation for {}: {}", wallet, e);
                }
            }
        }

        info!("✅ Updated reputation for {} wallets", updated_count);
        Ok(updated_count)
    }

    /// Calculate reputation for a single wallet
    pub async fn calculate_wallet_reputation(&self, wallet: &str) -> Result<ReputationScore> {
        debug!("Calculating reputation for {}", wallet);

        // 1. Win rate (40% weight)
        let win_rate = self.get_win_rate(wallet).await?;

        // 2. Profit factor (30% weight)
        let profit_factor = self.get_profit_factor(wallet).await?;

        // 3. Consistency (15% weight)
        let consistency = self.get_consistency_score(wallet).await?;

        // 4. Volume score (10% weight)
        let volume_score = self.get_volume_score(wallet).await?;

        // 5. Timing score (5% weight)
        let timing_score = self.get_timing_score(wallet).await?;

        // Calculate weighted score
        let raw_score = win_rate * 0.4
            + profit_factor * 0.3
            + consistency * 0.15
            + volume_score * 0.1
            + timing_score * 0.05;

        let score = raw_score * 10.0; // Scale to 0-10

        // Confidence based on sample size
        let trade_count = self.get_trade_count(wallet).await?;
        let confidence = self.calculate_confidence(trade_count);

        let reputation = ReputationScore::new(score, confidence);

        // Save to database
        self.save_reputation(wallet, &reputation).await?;

        // Save to history
        self.save_reputation_history(wallet, &reputation, trade_count).await?;

        debug!(
            "Wallet {}: Score={:.2}, Tier={}, Confidence={:.2}",
            wallet,
            reputation.score,
            reputation.tier.as_str(),
            reputation.confidence
        );

        Ok(reputation)
    }

    /// Win rate: % of trades that were profitable
    async fn get_win_rate(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            SELECT
                COUNT(CASE WHEN
                    (t.side = 'BUY' AND t.outcome = m.winning_outcome) OR
                    (t.side = 'SELL' AND t.outcome != m.winning_outcome)
                THEN 1 END) as wins,
                COUNT(*) as total
            FROM orderflow_trades t
            JOIN orderflow_market_outcomes m ON t.market_id = m.market_id
            WHERE t.wallet_address = $1
            AND m.resolved_at IS NOT NULL
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let total: i64 = row.try_get("total").unwrap_or(0);
        if total == 0 {
            return Ok(0.5); // Neutral if no resolved trades
        }

        let wins: i64 = row.try_get("wins").unwrap_or(0);
        Ok(wins as f64 / total as f64)
    }

    /// Profit factor: Average profit per trade as % of position size
    async fn get_profit_factor(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            SELECT AVG(
                CASE
                    WHEN (t.side = 'BUY' AND t.outcome = m.winning_outcome) THEN
                        ((1.0 - t.price::float) / t.price::float)
                    WHEN (t.side = 'SELL' AND t.outcome != m.winning_outcome) THEN
                        (t.price::float / (1.0 - t.price::float))
                    ELSE -1.0
                END
            ) as avg_profit_factor
            FROM orderflow_trades t
            JOIN orderflow_market_outcomes m ON t.market_id = m.market_id
            WHERE t.wallet_address = $1
            AND m.resolved_at IS NOT NULL
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let profit_factor: f64 = row.try_get("avg_profit_factor").unwrap_or(0.0);

        // Normalize to 0-1 range (assume good traders make 20-30% per trade)
        let normalized = (profit_factor + 1.0) / 2.5;
        Ok(normalized.max(0.0).min(1.0))
    }

    /// Consistency: Low variance in results = better
    async fn get_consistency_score(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            SELECT STDDEV(
                CASE
                    WHEN (t.side = 'BUY' AND t.outcome = m.winning_outcome) THEN 1.0
                    WHEN (t.side = 'SELL' AND t.outcome != m.winning_outcome) THEN 1.0
                    ELSE 0.0
                END
            ) as variance
            FROM orderflow_trades t
            JOIN orderflow_market_outcomes m ON t.market_id = m.market_id
            WHERE t.wallet_address = $1
            AND m.resolved_at IS NOT NULL
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let variance: f64 = row.try_get("variance").unwrap_or(0.5);

        // Lower variance = higher score
        let consistency = 1.0 - variance.min(1.0);
        Ok(consistency)
    }

    /// Volume score: More trades = more confident in reputation
    async fn get_volume_score(&self, wallet: &str) -> Result<f64> {
        let count = self.get_trade_count(wallet).await?;

        // Logarithmic scale: 10 trades = 0.5, 100 trades = 0.75, 1000+ = 1.0
        let score = (count as f64).log10() / 3.0;
        Ok(score.min(1.0))
    }

    /// Timing score: Early entry = conviction
    async fn get_timing_score(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            SELECT AVG(
                EXTRACT(EPOCH FROM (m.ends_at_timestamp - t.timestamp)) / 900.0
            ) as avg_time_remaining
            FROM orderflow_trades t
            JOIN orderflow_market_outcomes m ON t.market_id = m.market_id
            WHERE t.wallet_address = $1
            AND m.resolved_at IS NOT NULL
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let avg_time_remaining: f64 = row.try_get("avg_time_remaining").unwrap_or(0.5);

        // Earlier entry (more time remaining) = higher score
        Ok(avg_time_remaining.max(0.0).min(1.0))
    }

    async fn get_trade_count(&self, wallet: &str) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM orderflow_trades WHERE wallet_address = $1"
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        Ok(count)
    }

    /// Confidence in reputation score based on sample size
    fn calculate_confidence(&self, trade_count: i64) -> f64 {
        // Logarithmic confidence:
        // 5 trades = 0.3 confidence
        // 20 trades = 0.5 confidence
        // 100 trades = 0.8 confidence
        // 500+ trades = 1.0 confidence

        if trade_count < 5 {
            return 0.2;
        }

        let confidence = (trade_count as f64).log10() / 2.7; // log10(500) ≈ 2.7
        confidence.min(1.0)
    }

    async fn save_reputation(&self, wallet: &str, reputation: &ReputationScore) -> Result<()> {
        let score = Decimal::from_f64(reputation.score).unwrap_or(Decimal::ZERO);
        let confidence = Decimal::from_f64(reputation.confidence).unwrap_or(Decimal::ZERO);
        let tier = reputation.tier.as_str();

        sqlx::query(
            r#"
            INSERT INTO orderflow_wallet_stats (
                wallet_address,
                reputation_score,
                confidence_level,
                trader_tier,
                last_calculated_at,
                calculation_version
            )
            VALUES ($1, $2, $3, $4, NOW(), 1)
            ON CONFLICT (wallet_address)
            DO UPDATE SET
                reputation_score = $2,
                confidence_level = $3,
                trader_tier = $4,
                last_calculated_at = NOW(),
                updated_at = NOW()
            "#
        )
        .bind(wallet)
        .bind(score)
        .bind(confidence)
        .bind(tier)
        .execute(&self.db)
        .await?;

        Ok(())
    }

    async fn save_reputation_history(
        &self,
        wallet: &str,
        reputation: &ReputationScore,
        trade_count: i64,
    ) -> Result<()> {
        let score = Decimal::from_f64(reputation.score).unwrap_or(Decimal::ZERO);
        let tier = reputation.tier.as_str();

        sqlx::query(
            r#"
            INSERT INTO orderflow_reputation_history (
                wallet_address,
                score,
                tier,
                total_trades,
                calculated_at
            )
            VALUES ($1, $2, $3, $4, NOW())
            "#
        )
        .bind(wallet)
        .bind(score)
        .bind(tier)
        .bind(trade_count as i32)
        .execute(&self.db)
        .await?;

        Ok(())
    }
}
