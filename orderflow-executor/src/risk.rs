use anyhow::Result;
use rust_decimal::Decimal;
use sqlx::PgPool;
use tracing::warn;

use crate::config::ExecutorConfig;

pub struct RiskManager {
    db: PgPool,
    config: ExecutorConfig,
}

impl RiskManager {
    pub fn new(db: PgPool, config: &ExecutorConfig) -> Self {
        Self {
            db,
            config: config.clone(),
        }
    }

    /// Check if we can open a new position
    pub async fn can_open_position(&self) -> Result<bool> {
        // 1. Check max open positions
        let open_positions = self.get_open_position_count().await?;
        if open_positions >= self.config.max_open_positions {
            warn!(
                "Max open positions reached: {}/{}",
                open_positions, self.config.max_open_positions
            );
            return Ok(false);
        }

        // 2. Check daily loss limit
        let daily_pnl = self.get_daily_pnl().await?;
        if daily_pnl < -self.config.max_daily_loss {
            warn!(
                "Daily loss limit reached: ${} (limit: -${})",
                daily_pnl, self.config.max_daily_loss
            );
            return Ok(false);
        }

        Ok(true)
    }

    async fn get_open_position_count(&self) -> Result<i32> {
        let count = sqlx::query_scalar!(
            r#"
            SELECT COUNT(*) as count
            FROM orderflow_positions
            WHERE status = 'OPEN'
            "#
        )
        .fetch_one(&self.db)
        .await?;

        Ok(count.unwrap_or(0) as i32)
    }

    async fn get_daily_pnl(&self) -> Result<Decimal> {
        let pnl = sqlx::query_scalar!(
            r#"
            SELECT COALESCE(SUM(profit_loss_usd), 0) as total_pnl
            FROM orderflow_signals
            WHERE created_at >= CURRENT_DATE
            AND outcome_status IN ('WIN', 'LOSS')
            "#
        )
        .fetch_one(&self.db)
        .await?;

        Ok(pnl.unwrap_or(Decimal::ZERO))
    }
}
