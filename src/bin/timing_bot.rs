//! Timing Arbitrage Bot (Sharky6999 style)
//!
//! Captures price lag in the final seconds before resolution.
//! When outcome is 99.9% certain but price hasn't hit $1 yet,
//! buy and collect the difference.
//!
//! Run with: cargo run --bin timing_bot --release

use anyhow::Result;
use btc_arb_bot::{
    btc_price::spawn_btc_price_feed,
    clob::ClobClient,
    config::Config,
    market::MarketMonitor,
    orderbook::OrderbookManager,
    signer::OrderSigner,
    types::BtcMarket,
    websocket::{spawn_websocket_with_orderbook, WsEvent},
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::sync::Arc;
use tracing::{debug, error, info, warn, Level};
use tracing_subscriber::FmtSubscriber;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;

    let _subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .compact()
        .init();

    info!("╔═══════════════════════════════════════════════════╗");
    info!("║   TIMING BOT - Sharky6999 Style                   ║");
    info!("║   (Last-second price lag arbitrage)               ║");
    info!("╠═══════════════════════════════════════════════════╣");
    info!("║ Mode: {:42} ║", if config.dry_run { "DRY RUN" } else { "LIVE" });
    info!("║ Entry window: Minute 14.8-15.0 (last 12 sec)      ║");
    info!("║ Target: Buy at 95-99.5¢, collect $1 (Sharky6999) ║");
    info!("╚═══════════════════════════════════════════════════╝");

    // Initialize BTC price feed
    info!("Connecting to Coinbase for BTC price...");
    let btc_feed = spawn_btc_price_feed();

    // Wait for connection
    for _ in 0..50 {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        if btc_feed.get_price() > Decimal::ZERO {
            break;
        }
    }
    info!("BTC price: ${}", btc_feed.get_price().round_dp(2));

    let market_monitor = MarketMonitor::new(config.clone());
    let orderbook_manager = Arc::new(OrderbookManager::new());

    // Stats tracking
    let mut total_entries = 0u32;
    let mut total_wins = 0u32;
    let mut total_profit = Decimal::ZERO;

    // Main loop
    loop {
        info!("═══════════════════════════════════════════════════");
        info!("Searching for active BTC 15-min market...");

        let market = market_monitor.wait_for_next_market().await;
        info!("Found: {} (ends {})", market.title, market.end_time);

        // Start WebSocket
        let _ws_rx = spawn_websocket_with_orderbook(
            config.clone(),
            vec![market.up_token_id.clone(), market.down_token_id.clone()],
            orderbook_manager.clone(),
        );

        // Mark open price
        btc_feed.mark_market_open();
        let open_price = btc_feed.get_price();
        info!("Market open BTC: ${}", open_price.round_dp(2));

        // Run timing session
        let result = run_timing_session(
            &config,
            &btc_feed,
            &market,
            orderbook_manager.clone(),
        ).await;

        if let Ok((entered, won, profit)) = result {
            if entered {
                total_entries += 1;
                if won { total_wins += 1; }
                total_profit += profit;

                info!("Session result: {} ${:.2}", if won { "WIN" } else { "LOSS" }, profit);
                info!("Total: {}/{} wins, ${:.2} profit", total_wins, total_entries, total_profit);
            }
        }

        btc_feed.clear_market_open();
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
    }
}

async fn run_timing_session(
    config: &Config,
    btc_feed: &Arc<btc_arb_bot::btc_price::BtcPriceFeed>,
    market: &BtcMarket,
    orderbook_manager: Arc<OrderbookManager>,
) -> Result<(bool, bool, Decimal)> {
    let end_time = market.end_time;
    let mut entered = false;
    let mut entry_price = Decimal::ZERO;
    let mut predicted_up = false;
    let mut shares = Decimal::ZERO;

    info!("Waiting for timing window (minute 14.8-15.0)...");

    loop {
        let now = chrono::Utc::now();
        let seconds_to_end = (end_time - now).num_seconds();

        if seconds_to_end <= 0 {
            break;
        }

        let minute_of_period = 15.0 - (seconds_to_end as f64 / 60.0);

        // Only act in final 12 seconds (minute 14.8-15.0)
        if minute_of_period >= 14.8 && !entered {
            let btc_price = btc_feed.get_price();
            let btc_change_pct = btc_feed.get_price_change_pct().unwrap_or(Decimal::ZERO);
            let is_up = btc_feed.get_predicted_outcome();

            // Get orderbook spread
            if let Some(spread) = orderbook_manager.get_combined_spread(
                &market.up_token_id, &market.down_token_id
            ) {
                let (outcome, best_ask, _token_id) = if is_up == Some(true) {
                    ("UP", spread.up_best_ask, &market.up_token_id)
                } else {
                    ("DOWN", spread.down_best_ask, &market.down_token_id)
                };

                // SHARKY STRATEGY: Buy even at 99¢, profit from 1¢ spread
                // He makes $100K/month doing this at high volume
                if best_ask >= dec!(0.95) && best_ask <= dec!(0.995) {
                    let potential_profit = dec!(1.0) - best_ask;
                    let position_size = config.max_position_usd;
                    shares = position_size / best_ask;
                    let expected_profit = shares * potential_profit;

                    info!("╔═══════════════════════════════════════════════════╗");
                    info!("║   TIMING ENTRY SIGNAL!                            ║");
                    info!("╚═══════════════════════════════════════════════════╝");
                    info!("  Minute: {:.2}", minute_of_period);
                    info!("  BTC: ${} ({:+.4}%)", btc_price.round_dp(2), btc_change_pct);
                    info!("  Direction: {}", outcome);
                    info!("  Entry price: {}¢", (best_ask * dec!(100)).round_dp(1));
                    info!("  Potential profit: {}¢/share", (potential_profit * dec!(100)).round_dp(1));
                    info!("  Expected profit: ${:.2}", expected_profit);

                    if config.dry_run {
                        info!("[DRY RUN] Would buy {} shares at {}¢", shares.round_dp(0), (best_ask * dec!(100)).round_dp(1));
                    }

                    entered = true;
                    entry_price = best_ask;
                    predicted_up = is_up == Some(true);
                } else if best_ask > dec!(0.995) {
                    debug!("Price too high: {}¢ (already at 100¢)", (best_ask * dec!(100)).round_dp(1));
                } else if best_ask < dec!(0.95) {
                    debug!("Price too low: {}¢ (need 95-99.5¢)", (best_ask * dec!(100)).round_dp(1));
                }
            }
        }

        // Log status every 2 seconds in final window
        if minute_of_period >= 14.5 && seconds_to_end % 2 == 0 {
            let btc_change_pct = btc_feed.get_price_change_pct().unwrap_or(Decimal::ZERO);
            let dir = match btc_feed.get_predicted_outcome() {
                Some(true) => "UP",
                Some(false) => "DOWN",
                None => "FLAT",
            };
            info!("Min {:.2}: BTC {:+.4}% = {} | Entered: {}",
                  minute_of_period, btc_change_pct, dir, entered);
        }

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    // Calculate result
    if entered {
        let btc_change = btc_feed.get_price_change().unwrap_or(Decimal::ZERO);
        let actual_up = btc_change > Decimal::ZERO;
        let won = predicted_up == actual_up;
        let profit = if won {
            shares * (dec!(1.0) - entry_price)
        } else {
            -(shares * entry_price)
        };

        info!("╔═══════════════════════════════════════════════════╗");
        info!("║   TIMING SESSION RESULT                           ║");
        info!("╚═══════════════════════════════════════════════════╝");
        info!("  Predicted: {} | Actual: {} | {}",
              if predicted_up { "UP" } else { "DOWN" },
              if actual_up { "UP" } else { "DOWN" },
              if won { "WIN!" } else { "LOSS" });
        info!("  Entry: {}¢ | Shares: {}", (entry_price * dec!(100)).round_dp(1), shares.round_dp(0));
        info!("  Profit: ${:.2}", profit);

        Ok((true, won, profit))
    } else {
        info!("No timing entry this session (price not in 95-99.5¢ range)");
        Ok((false, false, Decimal::ZERO))
    }
}
