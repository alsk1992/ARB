use anyhow::{Context, Result};
use sqlx::{PgPool, Row};
use tracing::{debug, warn};

use crate::types::Trade;

pub struct TradeStorage {
    pool: PgPool,
}

impl TradeStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn save_trade(&self, trade: &Trade) -> Result<()> {
        // Insert trade into orderflow_trades table
        let result = sqlx::query(
            r#"
            INSERT INTO orderflow_trades (
                tx_hash, block_number, timestamp, wallet_address, is_maker,
                market_id, market_title, token_id, outcome, side,
                price, size, value_usd, order_hash, fee_paid, gas_price
            ) VALUES (
                $1, $2, $3, $4, $5,
                $6, $7, $8, $9, $10,
                $11, $12, $13, $14, $15, $16
            )
            ON CONFLICT (tx_hash) DO NOTHING
            "#
        )
        .bind(&trade.tx_hash)
        .bind(trade.block_number)
        .bind(trade.timestamp)
        .bind(&trade.wallet_address)
        .bind(trade.is_maker)
        .bind(&trade.market_id)
        .bind(&trade.market_title)
        .bind(&trade.token_id)
        .bind(&trade.outcome)
        .bind(&trade.side)
        .bind(trade.price)
        .bind(trade.size)
        .bind(trade.value_usd)
        .bind(&trade.order_hash)
        .bind(trade.fee_paid)
        .bind(trade.gas_price)
        .execute(&self.pool)
        .await;

        match result {
            Ok(result) => {
                if result.rows_affected() > 0 {
                    debug!("✅ Saved trade: {}", trade.tx_hash);

                    // Trigger wallet stats update
                    self.update_wallet_stats(&trade.wallet_address).await.ok();
                } else {
                    debug!("⏭️  Trade already exists: {}", trade.tx_hash);
                }
                Ok(())
            }
            Err(e) => {
                warn!("Failed to save trade {}: {}", trade.tx_hash, e);
                Err(e.into())
            }
        }
    }

    async fn update_wallet_stats(&self, wallet_address: &str) -> Result<()> {
        // The database trigger will handle basic stats updates
        // This is a placeholder for any additional real-time processing
        debug!("Wallet stats updated for {}", wallet_address);
        Ok(())
    }

    pub async fn get_trade_count(&self) -> Result<i64> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM orderflow_trades"
        )
        .fetch_one(&self.pool)
        .await
        .context("Failed to get trade count")?;

        Ok(count)
    }

    pub async fn get_recent_trades(&self, limit: i64) -> Result<Vec<Trade>> {
        let records = sqlx::query(
            r#"
            SELECT
                tx_hash, block_number, timestamp, wallet_address, is_maker,
                market_id, market_title, token_id, outcome, side,
                price, size, value_usd, order_hash, fee_paid, gas_price
            FROM orderflow_trades
            ORDER BY timestamp DESC
            LIMIT $1
            "#
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .context("Failed to fetch recent trades")?;

        let trades = records
            .into_iter()
            .map(|r| Trade {
                tx_hash: r.get("tx_hash"),
                block_number: r.get("block_number"),
                timestamp: r.get("timestamp"),
                wallet_address: r.get("wallet_address"),
                is_maker: r.get("is_maker"),
                market_id: r.get("market_id"),
                market_title: r.get("market_title"),
                token_id: r.get("token_id"),
                outcome: r.get("outcome"),
                side: r.get("side"),
                price: r.get("price"),
                size: r.get("size"),
                value_usd: r.get("value_usd"),
                order_hash: r.get("order_hash"),
                fee_paid: r.get("fee_paid"),
                gas_price: r.get("gas_price"),
            })
            .collect();

        Ok(trades)
    }
}
