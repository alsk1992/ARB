use anyhow::Result;
use sqlx::{PgPool, Row};
use tracing::{debug, info, warn};

use crate::models::ReputationScore;

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

    /// Win rate: % of closed positions that were profitable
    /// Matches BUY/SELL pairs on same market+token to calculate realized P&L
    async fn get_win_rate(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            WITH position_pairs AS (
                SELECT
                    (t2.price - t1.price) * t1.size as realized_pnl
                FROM orderflow_trades t1
                JOIN orderflow_trades t2
                    ON t1.wallet_address = t2.wallet_address
                    AND t1.market_id = t2.market_id
                    AND t1.token_id = t2.token_id
                WHERE t1.wallet_address = $1
                    AND t1.side = 'BUY'
                    AND t2.side = 'SELL'
                    AND t2.timestamp > t1.timestamp
            )
            SELECT
                COUNT(CASE WHEN realized_pnl > 0 THEN 1 END) as wins,
                COUNT(*) as total
            FROM position_pairs
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let total: i64 = row.try_get("total").unwrap_or(0);
        if total == 0 {
            return Ok(0.5); // Neutral if no closed positions
        }

        let wins: i64 = row.try_get("wins").unwrap_or(0);
        Ok(wins as f64 / total as f64)
    }

    /// Profit factor: Average profit per trade as % of position size
    /// Based on realized P&L from closed positions
    async fn get_profit_factor(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            WITH position_pairs AS (
                SELECT
                    ((t2.price - t1.price) / t1.price) as profit_pct
                FROM orderflow_trades t1
                JOIN orderflow_trades t2
                    ON t1.wallet_address = t2.wallet_address
                    AND t1.market_id = t2.market_id
                    AND t1.token_id = t2.token_id
                WHERE t1.wallet_address = $1
                    AND t1.side = 'BUY'
                    AND t2.side = 'SELL'
                    AND t2.timestamp > t1.timestamp
            )
            SELECT AVG(profit_pct) as avg_profit_pct
            FROM position_pairs
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let avg_profit_pct: Option<f64> = row.try_get("avg_profit_pct").ok();

        let profit_factor = avg_profit_pct.unwrap_or(0.0);

        // Normalize to 0-1 range (assume good traders make 10-50% per round trip)
        // 0% = 0.5, +50% = 1.0, -50% = 0.0
        let normalized = (profit_factor + 0.5) / 1.0;
        Ok(normalized.max(0.0).min(1.0))
    }

    /// Consistency: Low variance in P&L = better
    async fn get_consistency_score(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            WITH position_pairs AS (
                SELECT
                    CASE
                        WHEN (t2.price - t1.price) > 0 THEN 1.0
                        ELSE 0.0
                    END as is_win
                FROM orderflow_trades t1
                JOIN orderflow_trades t2
                    ON t1.wallet_address = t2.wallet_address
                    AND t1.market_id = t2.market_id
                    AND t1.token_id = t2.token_id
                WHERE t1.wallet_address = $1
                    AND t1.side = 'BUY'
                    AND t2.side = 'SELL'
                    AND t2.timestamp > t1.timestamp
            )
            SELECT STDDEV(is_win) as variance
            FROM position_pairs
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let variance: Option<f64> = row.try_get("variance").ok();
        let variance_val = variance.unwrap_or(0.5);

        // Lower variance = higher score
        let consistency = 1.0 - variance_val.min(1.0);
        Ok(consistency)
    }

    /// Volume score: More trades = more confident in reputation
    async fn get_volume_score(&self, wallet: &str) -> Result<f64> {
        let count = self.get_trade_count(wallet).await?;

        // Logarithmic scale: 10 trades = 0.5, 100 trades = 0.75, 1000+ = 1.0
        let score = (count as f64).log10() / 3.0;
        Ok(score.min(1.0))
    }

    /// Timing score: Average hold duration (shorter = more conviction/skill)
    async fn get_timing_score(&self, wallet: &str) -> Result<f64> {
        let row = sqlx::query(
            r#"
            WITH position_pairs AS (
                SELECT
                    EXTRACT(EPOCH FROM (t2.timestamp - t1.timestamp)) / 3600.0 as hold_hours
                FROM orderflow_trades t1
                JOIN orderflow_trades t2
                    ON t1.wallet_address = t2.wallet_address
                    AND t1.market_id = t2.market_id
                    AND t1.token_id = t2.token_id
                WHERE t1.wallet_address = $1
                    AND t1.side = 'BUY'
                    AND t2.side = 'SELL'
                    AND t2.timestamp > t1.timestamp
            )
            SELECT AVG(hold_hours) as avg_hold_hours
            FROM position_pairs
            "#
        )
        .bind(wallet)
        .fetch_one(&self.db)
        .await?;

        let avg_hold_hours: Option<f64> = row.try_get("avg_hold_hours").ok();
        let hold_hours = avg_hold_hours.unwrap_or(24.0);

        // Shorter hold = higher score (scalpers are skilled)
        // 1 hour = 1.0, 24 hours = 0.5, 168 hours (1 week) = 0.0
        let score = 1.0 - (hold_hours / 168.0).min(1.0);
        Ok(score.max(0.0))
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
        .bind(reputation.score)
        .bind(reputation.confidence)
        .bind(reputation.tier.as_str())
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
        .bind(reputation.score)
        .bind(reputation.tier.as_str())
        .bind(trade_count as i32)
        .execute(&self.db)
        .await?;

        Ok(())
    }
}
