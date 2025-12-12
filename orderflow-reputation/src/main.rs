use anyhow::{Context, Result};
use sqlx::PgPool;
use std::time::Duration;
use tracing::{error, info};

mod calculator;
mod models;

use calculator::ReputationCalculator;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("🧮 Starting Polymarket Reputation Calculator");

    // Load environment variables
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;

    let calculation_interval = std::env::var("CALCULATION_INTERVAL_SECONDS")
        .unwrap_or_else(|_| "3600".to_string())
        .parse::<u64>()
        .unwrap_or(3600);

    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL...");
    let db_pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("✅ Connected to PostgreSQL");

    let calculator = ReputationCalculator::new(db_pool);

    info!("📊 Calculating reputation scores every {} seconds", calculation_interval);
    info!("🔄 Starting calculation loop...");

    // Run calculation loop
    let mut iteration = 0u64;

    loop {
        iteration += 1;
        info!("🔄 Starting calculation iteration #{}", iteration);

        match calculator.calculate_all_wallets().await {
            Ok(count) => {
                info!("✅ Iteration #{}: Updated {} wallets", iteration, count);
            }
            Err(e) => {
                error!("❌ Iteration #{} failed: {}", iteration, e);
            }
        }

        info!("⏸️  Sleeping for {} seconds...", calculation_interval);
        tokio::time::sleep(Duration::from_secs(calculation_interval)).await;
    }
}
