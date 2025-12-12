use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::config::ExecutorConfig;

pub struct SignalGenerator {
    db: PgPool,
    config: ExecutorConfig,
}

#[derive(Debug, Clone)]
pub enum SignalType {
    FollowWhale,
    FadeDegen,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SignalType::FollowWhale => "FOLLOW_WHALE",
            SignalType::FadeDegen => "FADE_DEGEN",
        }
    }
}

impl SignalGenerator {
    pub fn new(db: PgPool, config: &ExecutorConfig) -> Self {
        Self {
            db,
            config: config.clone(),
        }
    }

    /// Check for new whale trades and generate signals
    pub async fn check_for_new_signals(&self) -> Result<Vec<i64>> {
        let mut signal_ids = Vec::new();

        // Find whale trades in the last 10 seconds that we haven't signaled yet
        let recent_whale_trades = sqlx::query!(
            r#"
            SELECT
                t.tx_hash,
                t.wallet_address,
                t.market_id,
                t.token_id,
                t.outcome,
                t.side,
                t.price,
                t.size,
                w.reputation_score,
                w.trader_tier,
                m.question as market_title
            FROM orderflow_trades t
            JOIN orderflow_wallet_stats w ON t.wallet_address = w.wallet_address
            LEFT JOIN (
                SELECT condition_id, question
                FROM orderflow_market_outcomes
            ) m ON t.market_id = m.condition_id
            WHERE t.timestamp > NOW() - INTERVAL '10 seconds'
            AND w.reputation_score >= $1
            AND NOT EXISTS (
                SELECT 1 FROM orderflow_signals s
                WHERE s.trigger_tx_hash = t.tx_hash
            )
            ORDER BY t.timestamp DESC
            "#,
            Decimal::try_from(self.config.min_whale_score).unwrap()
        )
        .fetch_all(&self.db)
        .await?;

        for trade in recent_whale_trades {
            if !self.config.enable_whale_following {
                continue;
            }

            let wallet_score = trade.reputation_score.unwrap_or(Decimal::ZERO);
            let confidence = (wallet_score.to_f64().unwrap_or(0.0) / 10.0).min(1.0);

            // Only follow BUY signals from whales
            if trade.side == "BUY" {
                let signal_id = self.create_signal(
                    &trade.wallet_address,
                    &trade.tx_hash,
                    wallet_score,
                    &trade.trader_tier.unwrap_or("UNKNOWN".to_string()),
                    SignalType::FollowWhale,
                    "BUY",
                    &trade.market_id,
                    trade.market_title.as_deref(),
                    trade.outcome.as_deref().unwrap_or("YES"),
                    confidence,
                    trade.price,
                    trade.size,
                )
                .await?;

                signal_ids.push(signal_id);

                info!(
                    "🐋 WHALE SIGNAL: {} bought {} @ {} (score: {:.1}, confidence: {:.0}%)",
                    trade.wallet_address,
                    trade.outcome.unwrap_or("YES".to_string()),
                    trade.price,
                    wallet_score,
                    confidence * 100.0
                );
            }
        }

        // Check for degen panic selling (multiple low-score wallets selling same market)
        if self.config.enable_degen_fading {
            let panic_signals = self.check_for_panic_sells().await?;
            signal_ids.extend(panic_signals);
        }

        Ok(signal_ids)
    }

    async fn check_for_panic_sells(&self) -> Result<Vec<i64>> {
        let mut signal_ids = Vec::new();

        // Find markets where 5+ degens sold in last 30 seconds
        let panic_markets = sqlx::query!(
            r#"
            SELECT
                t.market_id,
                t.outcome,
                COUNT(*) as panic_count,
                AVG(t.price) as avg_price,
                MAX(m.question) as market_title
            FROM orderflow_trades t
            JOIN orderflow_wallet_stats w ON t.wallet_address = w.wallet_address
            LEFT JOIN orderflow_market_outcomes m ON t.market_id = m.condition_id
            WHERE t.timestamp > NOW() - INTERVAL '30 seconds'
            AND t.side = 'SELL'
            AND w.reputation_score <= $1
            GROUP BY t.market_id, t.outcome
            HAVING COUNT(*) >= 5
            "#,
            Decimal::try_from(self.config.max_fade_score).unwrap()
        )
        .fetch_all(&self.db)
        .await?;

        for market in panic_markets {
            let confidence = 0.7; // Fixed confidence for panic fade signals
            let avg_price = market.avg_price.unwrap_or(Decimal::ZERO);

            let signal_id = self.create_signal(
                "MULTIPLE_DEGENS",
                "PANIC_SELL",
                Decimal::ZERO,
                "DEGEN",
                SignalType::FadeDegen,
                "BUY", // Buy what they're panic selling
                &market.market_id,
                market.market_title.as_deref(),
                &market.outcome.unwrap_or("YES".to_string()),
                confidence,
                avg_price,
                Decimal::ZERO,
            )
            .await?;

            signal_ids.push(signal_id);

            warn!(
                "🚨 PANIC SIGNAL: {} degens sold {} @ {} - FADING!",
                market.panic_count.unwrap_or(0),
                market.outcome.unwrap_or("YES".to_string()),
                avg_price
            );
        }

        Ok(signal_ids)
    }

    async fn create_signal(
        &self,
        trigger_wallet: &str,
        trigger_tx_hash: &str,
        wallet_score: Decimal,
        trader_tier: &str,
        signal_type: SignalType,
        action: &str,
        market_id: &str,
        market_title: Option<&str>,
        outcome: &str,
        confidence: f64,
        trigger_price: Decimal,
        trigger_size: Decimal,
    ) -> Result<i64> {
        // Calculate recommended size using Kelly criterion
        let recommended_size = self.calculate_position_size(confidence, trigger_price);

        // Set max price (don't buy above this)
        let max_price = trigger_price * Decimal::try_from(1.05).unwrap(); // 5% slippage tolerance

        let confidence_decimal = Decimal::try_from(confidence).unwrap();

        let signal_id = sqlx::query_scalar!(
            r#"
            INSERT INTO orderflow_signals (
                trigger_wallet,
                trigger_tx_hash,
                wallet_score,
                trader_tier,
                signal_type,
                action,
                market_id,
                market_title,
                outcome,
                confidence,
                recommended_size_usd,
                max_price,
                expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NOW() + INTERVAL '5 minutes')
            RETURNING id
            "#,
            trigger_wallet,
            trigger_tx_hash,
            wallet_score,
            trader_tier,
            signal_type.as_str(),
            action,
            market_id,
            market_title,
            outcome,
            confidence_decimal,
            recommended_size,
            max_price
        )
        .fetch_one(&self.db)
        .await?;

        debug!(
            "Created signal #{}: {} {} {} @ {} (confidence: {:.0}%)",
            signal_id,
            signal_type.as_str(),
            action,
            outcome,
            trigger_price,
            confidence * 100.0
        );

        Ok(signal_id)
    }

    fn calculate_position_size(&self, confidence: f64, price: Decimal) -> Decimal {
        // Kelly criterion: f = (bp - q) / b
        // where:
        //   b = odds (profit if win)
        //   p = probability of winning (confidence)
        //   q = probability of losing (1 - confidence)

        let p = confidence;
        let q = 1.0 - p;
        let price_f64 = price.to_f64().unwrap_or(0.5);
        let b = (1.0 - price_f64) / price_f64; // Profit if win

        let kelly = ((b * p) - q) / b;
        let fraction = kelly * self.config.kelly_fraction; // Use fraction of Kelly for safety

        let size = Decimal::try_from(fraction).unwrap_or(Decimal::ZERO)
            * self.config.max_position_usd;

        // Ensure size is positive and within limits
        size.max(Decimal::ZERO).min(self.config.max_position_usd)
    }
}
