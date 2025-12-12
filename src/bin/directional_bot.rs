//! Directional Trading Bot
//!
//! This is the PRO TRADER REPLICATION strategy.
//! Instead of buying BOTH sides (arbitrage), we buy ONE side
//! based on observed BTC price direction.
//!
//! Run with: cargo run --bin directional_bot --release
//!
//! HOW IT WORKS:
//! 1. Connect to Binance WebSocket for real-time BTC price
//! 2. When new 15-min market opens, record BTC price
//! 3. Wait until late in the period (minute 10-13)
//! 4. Observe if BTC is UP or DOWN from market open
//! 5. Buy the winning outcome BEFORE prices hit $1

use anyhow::Result;
use btc_arb_bot::{
    alerts::AlertClient,
    btc_price::{BtcPriceFeed, spawn_btc_price_feed},
    clob::ClobClient,
    config::Config,
    market::MarketMonitor,
    orderbook::OrderbookManager,
    signer::OrderSigner,
    strategies::directional::DirectionalConfig,
    trade_db::{TradeDb, TradeRecord},
    types::BtcMarket,
    websocket::{spawn_websocket_with_orderbook, WsEvent},
};
use parking_lot::Mutex;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::VecDeque;
use std::sync::Arc;
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

/// Volatility tracker - stores recent BTC prices for volatility calculation
struct VolatilityTracker {
    prices: VecDeque<Decimal>,
    max_samples: usize,
}

impl VolatilityTracker {
    fn new(max_samples: usize) -> Self {
        Self {
            prices: VecDeque::with_capacity(max_samples),
            max_samples,
        }
    }

    fn add_price(&mut self, price: Decimal) {
        if self.prices.len() >= self.max_samples {
            self.prices.pop_front();
        }
        self.prices.push_back(price);
    }

    /// Calculate volatility as coefficient of variation (stddev/mean * 100)
    fn volatility_pct(&self) -> Option<Decimal> {
        if self.prices.len() < 10 {
            return None;
        }

        let sum: Decimal = self.prices.iter().copied().sum();
        let mean = sum / Decimal::from(self.prices.len());

        if mean == Decimal::ZERO {
            return None;
        }

        let variance_sum: Decimal = self.prices
            .iter()
            .map(|p| (*p - mean) * (*p - mean))
            .sum();
        let variance = variance_sum / Decimal::from(self.prices.len());

        // Approximate sqrt using Newton's method
        let stddev = sqrt_approx(variance);

        Some(stddev / mean * dec!(100))
    }
}

/// Approximate square root using Newton's method
fn sqrt_approx(n: Decimal) -> Decimal {
    if n <= Decimal::ZERO {
        return Decimal::ZERO;
    }

    let mut x = n;
    for _ in 0..10 {
        x = (x + n / x) / dec!(2);
    }
    x
}

/// Calculate position size based on confidence level
/// 15m-a4 STRATEGY: Enter with small moves, scale position by confidence
fn confidence_position_sizing(
    btc_change_pct: Decimal,
    account_balance: Decimal,
) -> (Decimal, &'static str, Decimal) {
    let abs_change = btc_change_pct.abs();

    // Dynamic position sizing based on account balance and confidence
    // Uses conservative Kelly-inspired percentages:
    // - LOW confidence: 5-8% of balance (safer)
    // - MED confidence: 10-12% of balance
    // - HIGH confidence: 15-18% of balance
    // - VERY HIGH: 20-25% of balance (max risk)

    if abs_change < dec!(0.02) {
        (Decimal::ZERO, "TOO LOW (<0.02%) - NO TRADE", Decimal::ZERO)
    } else if abs_change < dec!(0.05) {
        let pct = dec!(0.08); // 8% of balance
        let size = account_balance * pct;
        (size, "LOW (0.02-0.05%) - 8%", pct * dec!(100))
    } else if abs_change < dec!(0.10) {
        let pct = dec!(0.12); // 12% of balance
        let size = account_balance * pct;
        (size, "MED (0.05-0.10%) - 12%", pct * dec!(100))
    } else if abs_change < dec!(0.20) {
        let pct = dec!(0.18); // 18% of balance
        let size = account_balance * pct;
        (size, "HIGH (0.10-0.20%) - 18%", pct * dec!(100))
    } else {
        let pct = dec!(0.25); // 25% of balance (max)
        let size = account_balance * pct;
        (size, "VERY HIGH (>0.20%) - 25%", pct * dec!(100))
    }
}

/// Tracks direction reversals during a session
struct ReversalTracker {
    last_direction: Option<bool>,  // true=UP, false=DOWN
    reversal_count: u32,
    consecutive_same: u32,
}

impl ReversalTracker {
    fn new() -> Self {
        Self {
            last_direction: None,
            reversal_count: 0,
            consecutive_same: 0,
        }
    }

    /// Update with new direction reading. Returns true if this was a reversal.
    fn update(&mut self, is_up: Option<bool>) -> bool {
        let Some(current) = is_up else {
            return false;
        };

        let was_reversal = if let Some(last) = self.last_direction {
            if last != current {
                self.reversal_count += 1;
                self.consecutive_same = 1;
                true
            } else {
                self.consecutive_same += 1;
                false
            }
        } else {
            self.consecutive_same = 1;
            false
        };

        self.last_direction = Some(current);
        was_reversal
    }

    fn is_choppy(&self) -> bool {
        self.reversal_count >= 2
    }

    fn is_trend_consistent(&self) -> bool {
        self.consecutive_same >= 3
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Load config
    let config = Config::from_env()?;

    // Setup logging
    let _subscriber = FmtSubscriber::builder()
        .with_max_level(match config.log_level.as_str() {
            "debug" => Level::DEBUG,
            "trace" => Level::TRACE,
            "warn" => Level::WARN,
            "error" => Level::ERROR,
            _ => Level::INFO,
        })
        .with_target(false)
        .with_thread_ids(false)
        .compact()
        .init();

    info!("╔═══════════════════════════════════════════════════╗");
    info!("║   DIRECTIONAL BOT - Pro Trader Strategy           ║");
    info!("║   (Replicate 100% win rate traders)               ║");
    info!("╠═══════════════════════════════════════════════════╣");
    info!("║ Mode: {:42} ║", if config.dry_run { "DRY RUN (no real orders)" } else { "LIVE TRADING" });
    info!("║ Max position: ${:36} ║", config.max_position_usd);
    info!("╚═══════════════════════════════════════════════════╝");

    // Initialize alert client
    let alerts = Arc::new(AlertClient::new(config.discord_webhook.clone()));
    alerts.bot_started(config.dry_run).await;

    // Initialize trade database
    let db_path = std::env::var("TRADE_DB_PATH").unwrap_or_else(|_| "trades.db".to_string());
    let trade_db = match TradeDb::new(&db_path) {
        Ok(db) => {
            info!("Trade database initialized: {}", db_path);
            Some(Arc::new(Mutex::new(db)))
        }
        Err(e) => {
            warn!("Failed to initialize trade database: {}", e);
            None
        }
    };

    // Initialize volatility tracker (300 samples = ~5 minutes at 1/sec)
    let volatility_tracker = Arc::new(Mutex::new(VolatilityTracker::new(300)));

    // Initialize BTC price feed (KEY COMPONENT!)
    info!("Connecting to Coinbase for real-time BTC price...");
    let btc_feed = spawn_btc_price_feed();

    // Wait for BTC feed to connect
    for _ in 0..50 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        if btc_feed.get_price() > Decimal::ZERO {
            break;
        }
    }

    if btc_feed.get_price() > Decimal::ZERO {
        info!("BTC price feed connected: ${}", btc_feed.get_price().round_dp(2));
    } else {
        warn!("BTC price feed not connected yet, continuing...");
    }

    // Initialize strategy - 15m-a4 EXACT LOGIC
    // Analysis: 15m-a4 trades 1,822 times with $161K profit = $88.50/trade
    // Entry prices 5-74¢ = they enter as EARLY as minute 3
    // Fractional avgPrice = LADDERING (multiple orders at different prices)
    let strategy_config = DirectionalConfig {
        entry_minute_min: 3.0,   // Enter from minute 3 (catches 15-50¢ prices)
        entry_minute_max: 13.5,  // Stop by minute 13.5 (safety buffer)
        min_confidence_pct: dec!(0.02), // Match 15m-a4: enter with small BTC moves
        max_entry_price: dec!(0.75), // Match 15m-a4 max (74¢)
        position_size: config.max_position_usd,
        max_position: config.max_position_usd,
        use_limit_orders: true,
        limit_offset: dec!(0.02), // 2 cents below best ask
        ladder_levels: 5,        // 5 price levels like pro traders
        ladder_spacing: dec!(0.02), // 2¢ between levels
    };

    info!("Strategy config:");
    info!("  Entry window: minute {:.0}-{:.0}", strategy_config.entry_minute_min, strategy_config.entry_minute_max);
    info!("  Min confidence: {}%", strategy_config.min_confidence_pct);
    info!("  Max entry price: ${}", strategy_config.max_entry_price);
    info!("  Position size: ${}", strategy_config.position_size);
    info!("  Laddering: {} levels @ {}¢ spacing", strategy_config.ladder_levels, strategy_config.ladder_spacing * dec!(100));

    // Initialize components
    let market_monitor = MarketMonitor::new(config.clone());
    let orderbook_manager = Arc::new(OrderbookManager::new());
    let clob = ClobClient::new(config.clone())?;
    let signer = OrderSigner::new(&config.private_key, &config.address)?;

    // Main trading loop
    run_directional_loop(
        config,
        btc_feed,
        strategy_config,
        market_monitor,
        orderbook_manager,
        clob,
        signer,
        alerts,
        trade_db,
        volatility_tracker,
    ).await
}

/// Main directional trading loop
async fn run_directional_loop(
    config: Config,
    btc_feed: Arc<BtcPriceFeed>,
    strategy_config: DirectionalConfig,
    market_monitor: MarketMonitor,
    orderbook_manager: Arc<OrderbookManager>,
    clob: ClobClient,
    signer: OrderSigner,
    alerts: Arc<AlertClient>,
    trade_db: Option<Arc<Mutex<TradeDb>>>,
    volatility_tracker: Arc<Mutex<VolatilityTracker>>,
) -> Result<()> {
    loop {
        info!("═══════════════════════════════════════════════════");
        info!("Searching for active BTC 15-min market...");
        info!("Current BTC price: ${}", btc_feed.get_price().round_dp(2));

        // Poll for market
        let market = market_monitor.wait_for_next_market().await;

        info!("Found market: {}", market.title);
        info!("  UP token:   {}", market.up_token_id);
        info!("  DOWN token: {}", market.down_token_id);
        info!("  Ends at:    {}", market.end_time);

        alerts.market_found(&market.title, &market.end_time.to_string()).await;

        // Start WebSocket for this market's orderbooks
        let market_ws_rx = spawn_websocket_with_orderbook(
            config.clone(),
            vec![market.up_token_id.clone(), market.down_token_id.clone()],
            orderbook_manager.clone(),
        );

        // Run directional trading session
        if let Err(e) = run_directional_session(
            &config,
            btc_feed.clone(),
            &strategy_config,
            &market,
            orderbook_manager.clone(),
            &clob,
            &signer,
            alerts.clone(),
            trade_db.clone(),
            volatility_tracker.clone(),
            market_ws_rx,
        ).await {
            error!("Session error: {}", e);
            alerts.error("Session failed", &e.to_string()).await;
        }

        // Wait before next market
        tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
    }
}

/// Run a directional trading session
async fn run_directional_session(
    config: &Config,
    btc_feed: Arc<BtcPriceFeed>,
    strategy_config: &DirectionalConfig,
    market: &BtcMarket,
    orderbook_manager: Arc<OrderbookManager>,
    clob: &ClobClient,
    signer: &OrderSigner,
    alerts: Arc<AlertClient>,
    trade_db: Option<Arc<Mutex<TradeDb>>>,
    volatility_tracker: Arc<Mutex<VolatilityTracker>>,
    mut ws_rx: tokio::sync::mpsc::Receiver<WsEvent>,
) -> Result<()> {
    // Mark market open BTC price
    btc_feed.mark_market_open();
    let open_price = btc_feed.get_price();
    info!("Market open BTC price: ${}", open_price.round_dp(2));

    // Wait for WebSocket connection
    let mut connected = false;
    let timeout = tokio::time::Instant::now() + tokio::time::Duration::from_secs(10);

    while !connected {
        if tokio::time::Instant::now() > timeout {
            warn!("Timeout waiting for WebSocket");
            break;
        }

        tokio::select! {
            Some(event) = ws_rx.recv() => {
                match event {
                    WsEvent::Connected => {
                        connected = true;
                        info!("WebSocket connected");
                    }
                    WsEvent::OrderbookUpdate { .. } => {
                        connected = true;
                    }
                    _ => {}
                }
            }
            _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {}
        }
    }

    // State
    let mut has_entered = false;
    let mut position_shares = Decimal::ZERO;
    let mut position_cost = Decimal::ZERO;
    let mut predicted_outcome: Option<bool> = None;
    let mut entry_price = Decimal::ZERO;
    let mut trade_record_id: Option<i64> = None;
    let mut minute_of_entry = 0.0;
    let mut skip_reason: Option<String> = None; // Track why we didn't enter

    // PRO TRADER: Track reversals and trend consistency
    let mut reversal_tracker = ReversalTracker::new();

    // Extract market time for alerts (e.g., "1:00AM-1:15AM ET")
    let market_time = market.title.split(" - ").last().unwrap_or(&market.title).to_string();

    // Monitor loop
    let end_time = market.end_time;
    let cancel_time = end_time - chrono::Duration::minutes(1);

    info!("Monitoring until {}...", cancel_time);
    info!("Will enter in minute {:.0}-{:.0} window", strategy_config.entry_minute_min, strategy_config.entry_minute_max);

    // Timer for entry checks (every 500ms)
    let mut entry_check_interval = tokio::time::interval(tokio::time::Duration::from_millis(500));

    loop {
        let now = chrono::Utc::now();
        if now >= cancel_time {
            info!("Approaching resolution, stopping...");
            break;
        }

        // Calculate minute of period
        let seconds_to_end = (end_time - now).num_seconds();
        let minute_of_period = 15.0 - (seconds_to_end as f64 / 60.0);

        // Get current BTC state
        let btc_price = btc_feed.get_price();
        let _btc_change = btc_feed.get_price_change();
        let btc_change_pct = btc_feed.get_price_change_pct();
        let btc_is_up = btc_feed.get_predicted_outcome();

        tokio::select! {
            // Process WebSocket events (for orderbook updates)
            Some(event) = ws_rx.recv() => {
                match event {
                    WsEvent::OrderbookUpdate { asset_id, .. } => {
                        // Just update orderbook manager (already done in websocket.rs)
                        debug!("Orderbook update for {}", asset_id);
                    }
                    WsEvent::TradeFill(fill) => {
                        info!("FILL: {} shares @ ${}", fill.size, fill.price);
                    }
                    WsEvent::Disconnected => {
                        warn!("WebSocket disconnected");
                    }
                    WsEvent::PriceChange { asset_id, best_bid, best_ask } => {
                        debug!("Price change for {}: bid={}, ask={}", asset_id, best_bid, best_ask);
                    }
                    _ => {}
                }
            }

            // TIMER-BASED ENTRY CHECK (runs every 500ms)
            _ = entry_check_interval.tick() => {
                // Update volatility tracker
                volatility_tracker.lock().add_price(btc_price);

                // PRO TRADER: Track direction reversals
                let was_reversal = reversal_tracker.update(btc_is_up);
                if was_reversal {
                    debug!("Direction reversal detected! Count: {}", reversal_tracker.reversal_count);
                }

                // Check if we should enter
                if !has_entered
                    && minute_of_period >= strategy_config.entry_minute_min
                    && minute_of_period <= strategy_config.entry_minute_max
                {
                    // PRO TRADER: Skip choppy markets (2+ reversals)
                    if reversal_tracker.is_choppy() {
                        skip_reason = Some(format!("Choppy market ({} reversals)", reversal_tracker.reversal_count));
                        info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                        continue;
                    }

                    // PRO TRADER: Require trend consistency (3+ same direction)
                    if !reversal_tracker.is_trend_consistent() {
                        skip_reason = Some(format!("Trend not consistent ({}/3)", reversal_tracker.consecutive_same));
                        info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                        continue;
                    }

                    // Get orderbook prices
                    if let Some(spread) = orderbook_manager.get_combined_spread(
                        &market.up_token_id, &market.down_token_id
                    ) {
                        // Check BTC direction
                        if let Some(is_up) = btc_is_up {
                            let pct = btc_change_pct.unwrap_or(Decimal::ZERO);

                            // VOLATILITY CHECK
                            let volatility = volatility_tracker.lock().volatility_pct();
                            if let Some(vol) = volatility {
                                if vol > dec!(0.5) {
                                    skip_reason = Some(format!("Volatility too high ({:.3}%)", vol));
                                    info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                                    continue;
                                }
                                if vol < dec!(0.01) {
                                    skip_reason = Some(format!("Market too flat ({:.4}%)", vol));
                                    info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                                    continue;
                                }
                            }

                            // MOMENTUM CHECK (PRO UPGRADE)
                            let momentum_aligned = btc_feed.is_momentum_aligned();
                            let momentum_conf = btc_feed.get_momentum_confidence();
                            if !momentum_aligned && minute_of_period < 10.0 {
                                skip_reason = Some(format!("Momentum not aligned (min {:.1})", minute_of_period));
                                info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                                continue;
                            }

                            // CONFIDENCE-BASED POSITION SIZING (% of account balance)
                            let (position_size, confidence_level, risk_pct) = confidence_position_sizing(
                                pct,
                                config.account_balance,
                            );

                            if position_size == Decimal::ZERO {
                                skip_reason = Some(format!("Confidence too low ({:.4}%)", pct.abs()));
                                info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                                continue;
                            }

                            // Check confidence threshold
                            if pct.abs() >= strategy_config.min_confidence_pct {
                                let (outcome, best_ask, token_id) = if is_up {
                                    ("UP", spread.up_best_ask, &market.up_token_id)
                                } else {
                                    ("DOWN", spread.down_best_ask, &market.down_token_id)
                                };

                                // Check price is acceptable
                                if best_ask <= strategy_config.max_entry_price {
                                    info!("╔═══════════════════════════════════════════════════╗");
                                    info!("║          ENTRY SIGNAL DETECTED!                   ║");
                                    info!("╚═══════════════════════════════════════════════════╝");
                                    info!("  BTC: ${} ({:+.4}% from open)", btc_price.round_dp(2), pct);
                                    info!("  Direction: {}", outcome);
                                    info!("  Best ask: ${}", best_ask);
                                    info!("  Minute: {:.1}", minute_of_period);
                                    info!("  Confidence: {} | Momentum: {:.1}", confidence_level, momentum_conf);
                                    info!("  Momentum aligned: {}", momentum_aligned);
                                    info!("  Account: ${} | Risk: {}% | Entry: ${}", config.account_balance, risk_pct, position_size.round_dp(2));
                                    if let Some(vol) = volatility {
                                        info!("  Volatility: {:.4}%", vol);
                                    }

                                    // Calculate laddered orders
                                    // Split position across multiple price levels
                                    let levels = strategy_config.ladder_levels.max(1);
                                    let size_per_level = position_size / Decimal::from(levels);
                                    minute_of_entry = minute_of_period;

                                    let mut total_shares = Decimal::ZERO;
                                    let mut total_cost = Decimal::ZERO;
                                    let mut avg_price = Decimal::ZERO;

                                    info!("📊 LADDERING: {} levels, ${:.2} per level", levels, size_per_level);

                                    for level in 0..levels {
                                        // Calculate price for this level
                                        let level_offset = strategy_config.ladder_spacing * Decimal::from(level);
                                        let level_price = if strategy_config.use_limit_orders {
                                            (best_ask - strategy_config.limit_offset - level_offset).max(dec!(0.01))
                                        } else {
                                            (best_ask - level_offset).max(dec!(0.01))
                                        };
                                        let level_shares = size_per_level / level_price;

                                        if config.dry_run {
                                            info!("  [L{}] {} shares @ {}¢", level + 1, level_shares.round_dp(0), level_price * dec!(100));
                                            total_shares += level_shares;
                                            total_cost += size_per_level;
                                            avg_price += level_price;
                                        } else {
                                            // Live order placement
                                            match create_and_submit_order(
                                                clob,
                                                signer,
                                                token_id,
                                                level_price,
                                                level_shares,
                                                market.tick_size,
                                                market.neg_risk,
                                            ).await {
                                                Ok(order_id) => {
                                                    info!("  [L{}] Order {}: {} shares @ {}¢", level + 1, order_id, level_shares.round_dp(0), level_price * dec!(100));
                                                    total_shares += level_shares;
                                                    total_cost += size_per_level;
                                                    avg_price += level_price;
                                                }
                                                Err(e) => {
                                                    warn!("  [L{}] Order failed: {}", level + 1, e);
                                                }
                                            }
                                        }
                                    }

                                    // Set entry state
                                    if total_shares > Decimal::ZERO {
                                        entry_price = avg_price / Decimal::from(levels);
                                        has_entered = true;
                                        position_shares = total_shares;
                                        position_cost = total_cost;
                                        predicted_outcome = Some(is_up);

                                        info!("✅ ENTRY: {} {} shares, avg {}¢, total ${:.2}",
                                            total_shares.round_dp(0), outcome, (entry_price * dec!(100)).round_dp(1), total_cost);

                                        // Send Telegram alert for entry
                                        alerts.market_entry(&market_time, outcome, entry_price, total_shares, pct).await;

                                        // Log to database
                                        if let Some(ref db) = trade_db {
                                            let record = TradeRecord {
                                                timestamp: chrono::Utc::now(),
                                                market_id: market.condition_id.clone(),
                                                market_title: market.title.clone(),
                                                direction: outcome.to_string(),
                                                entry_price,
                                                shares: total_shares,
                                                btc_open_price: open_price,
                                                btc_entry_price: btc_price,
                                                btc_change_pct: pct,
                                                confidence_score: pct.abs() * dec!(100),
                                                minute_of_entry,
                                                outcome: "PENDING".to_string(),
                                                profit: Decimal::ZERO,
                                                is_dry_run: config.dry_run,
                                            };
                                            match db.lock().insert_trade(&record) {
                                                Ok(id) => {
                                                    trade_record_id = Some(id);
                                                    info!("Trade logged to database (id: {})", id);
                                                }
                                                Err(e) => warn!("Failed to log trade: {}", e),
                                            }
                                        }
                                    }
                                } else {
                                    skip_reason = Some(format!("Price too high ({}¢ > {}¢)",
                                        (best_ask * dec!(100)).round_dp(0),
                                        (strategy_config.max_entry_price * dec!(100)).round_dp(0)));
                                    info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                                }
                            } else {
                                skip_reason = Some(format!("BTC move too small ({:.4}%)", pct.abs()));
                                info!("⏭️ SKIP: {}", skip_reason.as_ref().unwrap());
                            }
                        }
                    }
                }
            }

            // Periodic status logging
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(1)) => {
                // Periodic status
                if minute_of_period >= 0.0 {
                    let btc_dir = match btc_is_up {
                        Some(true) => "UP",
                        Some(false) => "DOWN",
                        None => "FLAT",
                    };
                    let pct = btc_change_pct.unwrap_or(Decimal::ZERO);

                    // Only log every 30 seconds or when in entry window
                    static LAST_LOG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                    let now_secs = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0);

                    let should_log = now_secs - LAST_LOG.load(std::sync::atomic::Ordering::Relaxed) >= 30
                        || (minute_of_period >= strategy_config.entry_minute_min && !has_entered);

                    if should_log {
                        LAST_LOG.store(now_secs, std::sync::atomic::Ordering::Relaxed);
                        info!(
                            "Minute {:.1}: BTC ${} ({:+.4}%) = {} | Position: {} shares @ ${}",
                            minute_of_period,
                            btc_price.round_dp(2),
                            pct,
                            btc_dir,
                            position_shares.round_dp(0),
                            position_cost.round_dp(2)
                        );
                    }
                }
            }
        }
    }

    // Final summary
    info!("╔═══════════════════════════════════════════════════╗");
    info!("║           SESSION SUMMARY                         ║");
    info!("╚═══════════════════════════════════════════════════╝");

    let btc_final = btc_feed.get_price();
    let btc_change = btc_feed.get_price_change().unwrap_or(Decimal::ZERO);
    let outcome = if btc_change > Decimal::ZERO { "UP" } else { "DOWN" };

    info!("BTC: ${} → ${} ({:+.2})", open_price.round_dp(2), btc_final.round_dp(2), btc_change);
    info!("Actual outcome: {}", outcome);

    if has_entered {
        let predicted = if predicted_outcome.unwrap_or(false) { "UP" } else { "DOWN" };
        let won = predicted == outcome;

        info!("Predicted: {} | Actual: {} | {}", predicted, outcome, if won { "WIN!" } else { "LOSS" });
        info!("Position: {} shares @ ${}", position_shares.round_dp(0), position_cost.round_dp(2));

        let profit = if won {
            let profit = position_shares - position_cost;
            let roi = profit / position_cost * dec!(100);
            info!("Profit: ${} ({:.1}% ROI)", profit.round_dp(2), roi);
            alerts.market_resolved(&market.title, profit).await;
            profit
        } else {
            let loss = position_cost;
            info!("Loss: ${}", loss.round_dp(2));
            alerts.market_resolved(&market.title, -loss).await;
            -loss
        };

        // Update trade database with outcome
        if let (Some(ref db), Some(id)) = (&trade_db, trade_record_id) {
            let outcome_str = if won { "WIN" } else { "LOSS" };
            if let Err(e) = db.lock().update_outcome(id, outcome_str, profit) {
                warn!("Failed to update trade outcome: {}", e);
            } else {
                info!("Trade outcome updated in database: {} ${:.2}", outcome_str, profit);
            }

            // Show stats
            match db.lock().get_stats(config.dry_run) {
                Ok(stats) => info!("Session stats: {}", stats),
                Err(e) => warn!("Failed to get stats: {}", e),
            }
        }
    } else {
        info!("No position taken this session");
        // Send skip reason to Telegram
        let final_btc_change = btc_feed.get_price_change_pct().unwrap_or(Decimal::ZERO);
        let reason = skip_reason.unwrap_or_else(|| "No clear signal".to_string());
        alerts.market_skipped(&market_time, &reason, final_btc_change).await;
    }

    // Clear market open price
    btc_feed.clear_market_open();

    // Wait for resolution
    let time_to_resolution = (end_time - chrono::Utc::now()).num_seconds();
    if time_to_resolution > 0 {
        info!("Waiting {} seconds for resolution...", time_to_resolution);
        tokio::time::sleep(tokio::time::Duration::from_secs(
            (time_to_resolution + 30) as u64
        )).await;
    }

    Ok(())
}

/// Create and submit an order
async fn create_and_submit_order(
    clob: &ClobClient,
    signer: &OrderSigner,
    token_id: &str,
    price: Decimal,
    size: Decimal,
    tick_size: Decimal,
    is_neg_risk: bool,
) -> Result<String> {
    use btc_arb_bot::types::Side;

    // Create and sign order using the signer
    let order = signer.create_order(
        token_id,
        price,
        size,
        Side::Buy,
        tick_size,
        is_neg_risk,
    ).await?;

    // Submit to CLOB
    let result = clob.post_order(&order).await?;

    // Extract order ID
    let order_id = result["orderID"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    Ok(order_id)
}
