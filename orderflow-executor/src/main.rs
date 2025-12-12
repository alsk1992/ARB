use anyhow::{Context, Result};
use sqlx::PgPool;
use std::time::Duration;
use tracing::{error, info};

mod config;
mod executor;
mod risk;
mod signals;

use config::ExecutorConfig;
use executor::OrderExecutor;
use signals::SignalGenerator;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("🎯 Starting Polymarket Order Flow Executor");

    // Load environment variables
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;

    let config = ExecutorConfig::from_env()?;

    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL...");
    let db_pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("✅ Connected to PostgreSQL");

    let signal_generator = SignalGenerator::new(db_pool.clone(), &config);
    let executor = OrderExecutor::new(db_pool.clone(), config.clone()).await?;

    info!("🚀 Signal execution loop started");
    info!("📊 Config: Min confidence={}, Max position=${}, Paper trading={}",
        config.min_signal_confidence,
        config.max_position_usd,
        config.enable_paper_trading
    );

    let mut iteration = 0u64;

    loop {
        iteration += 1;

        // 1. Generate signals from recent trades
        match signal_generator.check_for_new_signals().await {
            Ok(signals) => {
                if !signals.is_empty() {
                    info!("🔔 Generated {} new signals", signals.len());
                }
            }
            Err(e) => {
                error!("Failed to generate signals: {}", e);
            }
        }

        // 2. Execute pending high-confidence signals
        match executor.execute_pending_signals().await {
            Ok(executed) => {
                if executed > 0 {
                    info!("✅ Executed {} signals", executed);
                }
            }
            Err(e) => {
                error!("Failed to execute signals: {}", e);
            }
        }

        // Check every 3 seconds for new whale trades
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}
