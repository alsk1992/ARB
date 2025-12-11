mod alerts;
mod auth;
mod clob;
mod config;
mod datalog;
mod market;
mod ml_client;
mod orderbook;
mod position;
mod presign;
mod retry;
mod signer;
mod strategy;
mod types;
mod websocket;

use anyhow::Result;
use parking_lot::Mutex;
use rust_decimal_macros::dec;
use std::sync::Arc;
use tracing::{error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

use crate::alerts::AlertClient;
use crate::clob::ClobClient;
use crate::config::Config;
use crate::datalog::{DataLogger, MarketSnapshot, OrderLog, FillLog, SessionSummary};
use crate::market::MarketMonitor;
use crate::ml_client::MlClient;
use crate::orderbook::OrderbookManager;
use crate::position::PositionManager;
use crate::presign::PreSignCache;
use crate::signer::OrderSigner;
use crate::strategy::LadderStrategy;
use crate::types::BtcMarket;
use crate::websocket::{spawn_websocket_with_orderbook, WsEvent};

#[tokio::main]
async fn main() -> Result<()> {
    // Load config
    let config = Config::from_env()?;

    // Setup logging
    let _subscriber = FmtSubscriber::builder()
        .with_max_level(match config.log_level.as_str() {
            "debug" => Level::DEBUG,
            "warn" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        })
        .with_target(false)
        .with_thread_ids(false)
        .compact()
        .init();

    info!("╔═══════════════════════════════════════╗");
    info!("║     BTC 15-Min Arbitrage Bot          ║");
    info!("║     Ladder Strategy Edition           ║");
    info!("╠═══════════════════════════════════════╣");
    info!("║ Mode: {:30} ║", if config.dry_run { "DRY RUN (no real orders)" } else { "LIVE TRADING" });
    info!("║ Max position: ${:25} ║", config.max_position_usd);
    info!("║ Target spread: {:23}% ║", config.target_spread_percent);
    info!("║ Ladder levels: {:24} ║", config.ladder_levels);
    info!("╚═══════════════════════════════════════╝");

    // Initialize alert client
    let alerts = Arc::new(AlertClient::new(config.discord_webhook.clone()));
    alerts.bot_started(config.dry_run).await;

    // Initialize data logger for ML analysis
    let data_logger = Arc::new(DataLogger::new("./data")?);
    info!("Data logging to ./data/ (session: {})", data_logger.session_id());

    // Initialize ML client
    let mut ml_client = MlClient::new();
    if ml_client.health_check().await {
        info!("ML prediction server connected");
    } else {
        info!("ML prediction server not available (will use defaults)");
    }
    let ml_client = Arc::new(ml_client);

    // Initialize components
    let clob = ClobClient::new(config.clone())?;
    let signer = OrderSigner::new(&config.private_key, &config.address)?;

    // HFT MODE: Enable pre-signing (reduces execution latency by ~150ms)
    info!("Initializing HFT pre-sign cache...");
    let presign_cache = Arc::new(PreSignCache::new(OrderSigner::new(&config.private_key, &config.address)?));
    let strategy = LadderStrategy::new(config.clone(), clob, signer)
        .with_presign(presign_cache.clone());

    let market_monitor = MarketMonitor::new(config.clone());
    let position_manager = Arc::new(Mutex::new(PositionManager::new()));
    let orderbook_manager = Arc::new(OrderbookManager::new());

    // Pre-warm connections
    info!("Pre-warming connections...");
    prewarm_connections(&config).await;

    // Main trading loop - no global WebSocket, we poll for markets
    run_trading_loop(
        config,
        strategy,
        market_monitor,
        position_manager,
        orderbook_manager,
        alerts,
        data_logger,
        presign_cache,
    ).await
}

/// Pre-warm HTTP connections
async fn prewarm_connections(config: &Config) {
    let client = reqwest::Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(5)
        .build()
        .unwrap();

    // Warm up CLOB
    let _ = client.get(&config.clob_url).send().await;
    // Warm up Gamma
    let _ = client.get(&config.gamma_url).send().await;

    info!("Connections pre-warmed");
}

/// Main trading loop
async fn run_trading_loop(
    config: Config,
    strategy: LadderStrategy,
    market_monitor: MarketMonitor,
    position_manager: Arc<Mutex<PositionManager>>,
    orderbook_manager: Arc<OrderbookManager>,
    alerts: Arc<AlertClient>,
    data_logger: Arc<DataLogger>,
    presign_cache: Arc<PreSignCache>,
) -> Result<()> {
    loop {
        info!("═══════════════════════════════════════");
        info!("Searching for active BTC 15-min market...");

        // Poll for market (REST API)
        let market = market_monitor.wait_for_next_market().await;

        info!("Found market: {}", market.title);
        info!("  UP token:   {}", market.up_token_id);
        info!("  DOWN token: {}", market.down_token_id);
        info!("  Ends at:    {}", market.end_time);
        info!("  Tick size:  {}", market.tick_size);

        alerts.market_found(&market.title, &market.end_time.to_string()).await;

        // HFT MODE: Pre-sign orders for this market (takes ~2-3min, saves ~150ms per trade)
        info!("⚡ Pre-signing orders for HFT mode...");
        if let Err(e) = presign_cache.presign_market(&market).await {
            warn!("Failed to pre-sign orders: {}", e);
        } else {
            let stats = presign_cache.stats();
            info!("✅ Pre-signed {} orders ready for instant execution", stats.total_orders);
        }

        // Start WebSocket for this market's orderbooks
        let market_ws_rx = spawn_websocket_with_orderbook(
            config.clone(),
            vec![market.up_token_id.clone(), market.down_token_id.clone()],
            orderbook_manager.clone(),
        );

        // Run trading session
        if let Err(e) = run_market_session(
            &config,
            &strategy,
            &market,
            position_manager.clone(),
            orderbook_manager.clone(),
            alerts.clone(),
            data_logger.clone(),
            market_ws_rx,
        ).await {
            error!("Market session error: {}", e);
            alerts.error("Market session failed", &e.to_string()).await;
        }

        // Wait before next market
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

/// Run a trading session for a single market
async fn run_market_session(
    config: &Config,
    strategy: &LadderStrategy,
    market: &BtcMarket,
    position_manager: Arc<Mutex<PositionManager>>,
    orderbook_manager: Arc<OrderbookManager>,
    alerts: Arc<AlertClient>,
    data_logger: Arc<DataLogger>,
    mut ws_rx: tokio::sync::mpsc::Receiver<WsEvent>,
) -> Result<()> {
    let session_start = chrono::Utc::now();
    let mut orders_placed: u32 = 0;
    let mut fills_received: u32 = 0;
    // Wait for WebSocket connection and initial orderbook
    let mut connected = false;
    let mut orderbook_received = false;
    let timeout = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);

    while !connected || !orderbook_received {
        if tokio::time::Instant::now() > timeout {
            warn!("Timeout waiting for WebSocket/orderbook");
            break;
        }

        tokio::select! {
            Some(event) = ws_rx.recv() => {
                match event {
                    WsEvent::Connected => {
                        connected = true;
                        info!("WebSocket connected");
                    }
                    WsEvent::OrderbookUpdate { asset_id, .. } => {
                        if asset_id == market.up_token_id || asset_id == market.down_token_id {
                            orderbook_received = true;
                            info!("Orderbook received for {}", asset_id);
                        }
                    }
                    WsEvent::Error(e) => {
                        warn!("WebSocket error during setup: {}", e);
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {}
        }
    }

    // Check spread before entering
    if let Some(spread) = orderbook_manager.get_combined_spread(&market.up_token_id, &market.down_token_id) {
        info!("Current spread: {}% (UP ask: {}, DOWN ask: {})",
            spread.spread_pct, spread.up_best_ask, spread.down_best_ask);

        if !spread.meets_threshold(config.min_spread_percent) {
            warn!("Spread {}% below minimum {}%, skipping market",
                spread.spread_pct, config.min_spread_percent);
            alerts.warning(&format!("Skipping market - spread too tight: {}%", spread.spread_pct)).await;
            return Ok(());
        }
    }

    // Submit ladder orders
    info!("Submitting ladder orders...");
    let (up_order_ids, down_order_ids) = match strategy.submit_ladder(market).await {
        Ok(ids) => ids,
        Err(e) => {
            error!("Failed to submit orders: {}", e);
            alerts.error("Order submission failed", &e.to_string()).await;
            return Err(e);
        }
    };

    alerts.orders_submitted(
        up_order_ids.len(),
        down_order_ids.len(),
        config.max_position_usd,
    ).await;

    // Register orders for fill tracking
    {
        let mut pm = position_manager.lock();
        pm.register_orders(&market.condition_id, &up_order_ids, &down_order_ids);
    }

    // Monitor fills until market closes
    let end_time = market.end_time;
    let cancel_time = end_time - chrono::Duration::minutes(2);
    let mut last_status_update = tokio::time::Instant::now();

    info!("Monitoring fills until {}...", cancel_time);

    loop {
        let now = chrono::Utc::now();
        if now >= cancel_time {
            info!("Approaching resolution, cancelling all open orders...");
            // CRITICAL: Cancel all orders before resolution to prevent unwanted fills
            if let Err(e) = strategy.cancel_all_orders(&market.condition_id).await {
                error!("Failed to cancel orders: {}", e);
                alerts.error("Order cancellation failed", &e.to_string()).await;
            }
            break;
        }

        tokio::select! {
            Some(event) = ws_rx.recv() => {
                match event {
                    WsEvent::TradeFill(fill) => {
                        info!("Fill: {} shares @ ${}", fill.size, fill.price);
                        fills_received += 1;

                        {
                            let mut pm = position_manager.lock();
                            pm.process_fill(&fill);
                        }

                        let side = if fill.asset_id == market.up_token_id { "UP" } else { "DOWN" };
                        alerts.fill_received(side, &fill.size, &fill.price).await;

                        // Log fill for ML analysis
                        let _ = data_logger.log_fill(&FillLog {
                            timestamp: chrono::Utc::now(),
                            market_id: market.condition_id.clone(),
                            side: side.to_string(),
                            price: fill.price.parse().unwrap_or_default(),
                            size: fill.size.parse().unwrap_or_default(),
                            order_id: fill.order_id.clone(),
                        });

                        // Check position balance and rebalance if needed
                        let should_rebalance = {
                            let pm = position_manager.lock();
                            if let Some(pos) = pm.get_position(&market.condition_id) {
                                if !pos.is_balanced() {
                                    alerts.position_imbalance(pos.up_shares, pos.down_shares).await;
                                    Some(pos.clone())
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        // Trigger rebalancing outside of lock
                        if let Some(pos) = should_rebalance {
                            if let Err(e) = strategy.rebalance_if_needed(market, &pos).await {
                                warn!("Rebalance failed: {}", e);
                            }
                        }
                    }
                    WsEvent::OrderbookUpdate { asset_id: _, bids: _, asks: _ } => {
                        // Orderbook already updated by WebSocket client
                        // Check for spread opportunities and potentially snipe
                        if let Some(spread) = orderbook_manager.get_combined_spread(
                            &market.up_token_id, &market.down_token_id
                        ) {
                            // Get full orderbook depth for ML
                            let (up_bids, up_asks, down_bids, down_asks) = orderbook_manager
                                .get_orderbook_depth(&market.up_token_id, &market.down_token_id)
                                .unwrap_or_default();

                            // Log market snapshot for ML analysis
                            let _ = data_logger.log_market_snapshot(&MarketSnapshot {
                                timestamp: chrono::Utc::now(),
                                market_id: market.condition_id.clone(),
                                market_title: market.title.clone(),
                                end_time: market.end_time,
                                up_token_id: market.up_token_id.clone(),
                                down_token_id: market.down_token_id.clone(),
                                up_best_bid: Some(spread.up_best_ask - rust_decimal_macros::dec!(0.01)), // approximate
                                up_best_ask: Some(spread.up_best_ask),
                                down_best_bid: Some(spread.down_best_ask - rust_decimal_macros::dec!(0.01)), // approximate
                                down_best_ask: Some(spread.down_best_ask),
                                combined_ask: Some(spread.up_best_ask + spread.down_best_ask),
                                spread_pct: Some(spread.spread_pct),
                                // Full orderbook depth
                                up_bids,
                                up_asks,
                                down_bids,
                                down_asks,
                            });
                            // If spread is large enough, try to snipe
                            if spread.spread_pct >= config.target_spread_percent {
                                info!("Large spread detected: {}%! Attempting snipe...", spread.spread_pct);

                                match strategy.snipe_spread(
                                    market,
                                    spread.up_best_ask,
                                    spread.down_best_ask,
                                ).await {
                                    Ok(Some((up_ids, down_ids))) => {
                                        // Register snipe orders for tracking
                                        let mut pm = position_manager.lock();
                                        pm.register_orders(&market.condition_id, &up_ids, &down_ids);
                                        alerts.orders_submitted(
                                            up_ids.len(),
                                            down_ids.len(),
                                            config.max_position_usd,
                                        ).await;
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        warn!("Snipe failed: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    WsEvent::Disconnected => {
                        warn!("WebSocket disconnected during session");
                    }
                    WsEvent::Error(e) => {
                        warn!("WebSocket error: {}", e);
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(30)) => {
                // Periodic status update
                if last_status_update.elapsed() > tokio::time::Duration::from_secs(60) {
                    let pm = position_manager.lock();
                    pm.print_summary(&market.condition_id);

                    if let Some(pos) = pm.get_position(&market.condition_id) {
                        let report = pm.calculate_pnl(pos);
                        alerts.position_update(pos.up_shares, pos.down_shares, report.locked_profit).await;
                    }

                    last_status_update = tokio::time::Instant::now();
                }
            }
        }
    }

    // Final position summary
    let (final_profit, session_summary) = {
        let pm = position_manager.lock();
        info!("╔═══════════════════════════════════════╗");
        info!("║        FINAL POSITION SUMMARY         ║");
        info!("╚═══════════════════════════════════════╝");
        pm.print_summary(&market.condition_id);

        let profit = pm.get_position(&market.condition_id)
            .map(|pos| pm.calculate_pnl(pos).locked_profit)
            .unwrap_or_default();

        // Build session summary for ML analysis
        let summary = pm.get_position(&market.condition_id).map(|pos| {
            let total_cost = pos.total_cost();
            SessionSummary {
                session_id: data_logger.session_id().to_string(),
                start_time: session_start,
                end_time: chrono::Utc::now(),
                market_id: market.condition_id.clone(),
                market_title: market.title.clone(),
                total_up_shares: pos.up_shares,
                total_down_shares: pos.down_shares,
                total_up_cost: pos.up_cost,
                total_down_cost: pos.down_cost,
                total_cost,
                min_shares: pos.min_shares(),
                guaranteed_payout: pos.guaranteed_payout(),
                locked_profit: pos.locked_profit(),
                profit_pct: if total_cost > rust_decimal::Decimal::ZERO {
                    pos.locked_profit() / total_cost * rust_decimal_macros::dec!(100)
                } else {
                    rust_decimal::Decimal::ZERO
                },
                is_dry_run: config.dry_run,
                orders_placed,
                fills_received,
            }
        });

        (profit, summary)
    };

    // Log session summary for ML analysis
    if let Some(summary) = session_summary {
        info!("Logging session summary: profit=${}, profit_pct={}%", summary.locked_profit, summary.profit_pct);
        let _ = data_logger.log_session_summary(&summary);
    }

    // Wait for resolution
    let time_to_resolution = (end_time - chrono::Utc::now()).num_seconds();
    if time_to_resolution > 0 {
        info!("Waiting {} seconds for resolution...", time_to_resolution);
        tokio::time::sleep(tokio::time::Duration::from_secs(
            (time_to_resolution + 30) as u64
        )).await;
    }

    // Log result
    alerts.market_resolved(&market.title, final_profit).await;

    // Clear position
    {
        let mut pm = position_manager.lock();
        pm.clear_position(&market.condition_id);
    }

    info!("Market session complete");
    Ok(())
}
