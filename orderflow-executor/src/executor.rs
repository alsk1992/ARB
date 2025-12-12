use anyhow::{Context, Result};
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::{debug, info, warn};

use crate::config::ExecutorConfig;
use crate::risk::RiskManager;

pub struct OrderExecutor {
    db: PgPool,
    config: ExecutorConfig,
    risk_manager: RiskManager,
}

impl OrderExecutor {
    pub async fn new(db: PgPool, config: ExecutorConfig) -> Result<Self> {
        let risk_manager = RiskManager::new(db.clone(), &config);

        Ok(Self {
            db,
            config,
            risk_manager,
        })
    }

    /// Execute all pending high-confidence signals
    pub async fn execute_pending_signals(&self) -> Result<usize> {
        // Get pending signals above confidence threshold
        let pending_signals = sqlx::query!(
            r#"
            SELECT
                id,
                signal_type,
                action,
                market_id,
                market_title,
                outcome,
                confidence,
                recommended_size_usd,
                max_price,
                trigger_wallet,
                wallet_score
            FROM orderflow_signals
            WHERE status = 'PENDING'
            AND confidence >= $1
            AND created_at > NOW() - INTERVAL '5 minutes'
            ORDER BY confidence DESC, created_at ASC
            LIMIT 10
            "#,
            Decimal::try_from(self.config.min_signal_confidence).unwrap()
        )
        .fetch_all(&self.db)
        .await?;

        if pending_signals.is_empty() {
            return Ok(0);
        }

        debug!("Found {} pending signals", pending_signals.len());

        let mut executed_count = 0;

        for signal in pending_signals {
            // Check risk limits
            if !self.risk_manager.can_open_position().await? {
                warn!("Risk limits reached, skipping signal #{}", signal.id);
                self.mark_signal_skipped(signal.id, "Risk limits reached").await?;
                continue;
            }

            // Execute signal
            match self.execute_signal(
                signal.id,
                &signal.market_id,
                signal.market_title.as_deref().unwrap_or("Unknown Market"),
                &signal.outcome.unwrap_or("YES".to_string()),
                &signal.action,
                signal.recommended_size_usd.unwrap_or(Decimal::ZERO),
                signal.max_price.unwrap_or(Decimal::ZERO),
            )
            .await
            {
                Ok(_) => {
                    executed_count += 1;
                    info!(
                        "✅ Executed signal #{}: {} {} @ {} (confidence: {:.0}%)",
                        signal.id,
                        signal.action,
                        signal.outcome.unwrap_or("YES".to_string()),
                        signal.max_price.unwrap_or(Decimal::ZERO),
                        signal.confidence.unwrap_or(Decimal::ZERO).to_f64().unwrap_or(0.0) * 100.0
                    );
                }
                Err(e) => {
                    warn!("Failed to execute signal #{}: {}", signal.id, e);
                    self.mark_signal_skipped(signal.id, &e.to_string()).await?;
                }
            }
        }

        Ok(executed_count)
    }

    async fn execute_signal(
        &self,
        signal_id: i64,
        market_id: &str,
        market_title: &str,
        outcome: &str,
        side: &str,
        size_usd: Decimal,
        max_price: Decimal,
    ) -> Result<()> {
        if self.config.enable_paper_trading {
            // Paper trading: just log and mark as executed
            info!(
                "📝 PAPER TRADE: {} {} in {} @ {} for ${}",
                side, outcome, market_title, max_price, size_usd
            );

            sqlx::query!(
                r#"
                UPDATE orderflow_signals
                SET status = 'EXECUTED',
                    executed_at = NOW(),
                    executed_price = $2
                WHERE id = $1
                "#,
                signal_id,
                max_price
            )
            .execute(&self.db)
            .await?;

            return Ok(());
        }

        // Real execution would go here:
        // 1. Fetch current orderbook from Polymarket CLOB
        // 2. Check if price is still good (within max_price)
        // 3. Build EIP-712 signed order
        // 4. Submit to Polymarket CLOB API
        // 5. Store order ID and track fill status

        warn!("Real execution not yet implemented - enable ENABLE_PAPER_TRADING=true");

        Ok(())
    }

    async fn mark_signal_skipped(&self, signal_id: i64, reason: &str) -> Result<()> {
        sqlx::query!(
            r#"
            UPDATE orderflow_signals
            SET status = 'SKIPPED'
            WHERE id = $1
            "#,
            signal_id
        )
        .execute(&self.db)
        .await?;

        debug!("Signal #{} skipped: {}", signal_id, reason);
        Ok(())
    }
}
