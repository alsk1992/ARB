use anyhow::{Context, Result};
use ethers::prelude::*;
use sqlx::PgPool;
use std::sync::Arc;
use tracing::{error, info, warn};

mod polygon;
mod storage;
mod types;

use polygon::PolygonListener;
use storage::TradeStorage;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .init();

    info!("🚀 Starting Polymarket Order Flow Listener");

    // Load environment variables
    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set")?;
    let polygon_rpc_url = std::env::var("POLYGON_RPC_URL")
        .context("POLYGON_RPC_URL must be set")?;

    // Connect to PostgreSQL
    info!("Connecting to PostgreSQL...");
    let db_pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to PostgreSQL")?;

    info!("✅ Connected to PostgreSQL");

    // Create storage handler
    let storage = TradeStorage::new(db_pool.clone());

    // Connect to Polygon WebSocket
    info!("Connecting to Polygon RPC: {}", polygon_rpc_url);
    let listener = PolygonListener::new(&polygon_rpc_url, storage)
        .await
        .context("Failed to create Polygon listener")?;

    info!("✅ Connected to Polygon");
    info!("📡 Listening for Polymarket trades...");

    // Start listening (runs forever)
    if let Err(e) = listener.start_listening().await {
        error!("❌ Listener stopped with error: {}", e);
        return Err(e);
    }

    Ok(())
}
