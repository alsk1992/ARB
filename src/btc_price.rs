//! BTC Price Feed from Multiple Exchanges
//!
//! Provides real-time BTC price tracking from Coinbase + Binance.
//! This is the KEY difference from our old strategy:
//! - Old: Track Polymarket prices (circular logic)
//! - New: Track actual BTC price (what determines the outcome!)
//!
//! Multi-exchange benefits:
//! - Cross-validation: Both exchanges must agree on direction
//! - Higher confidence: Reduces false signals from single-exchange lag
//! - Redundancy: If one exchange fails, use the other

use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Deserialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info, warn};

/// BTC price update event
#[derive(Debug, Clone)]
pub struct BtcPriceUpdate {
    pub price: Decimal,
    pub timestamp: u64,
}

/// BTC price feed state with momentum tracking
#[derive(Debug)]
pub struct BtcPriceState {
    /// Current BTC price (from Coinbase - primary)
    pub current_price: Decimal,
    /// Price at market open (set when new 15-min period starts)
    pub market_open_price: Option<Decimal>,
    /// Timestamp of last Coinbase update
    pub last_update: Instant,
    /// Coinbase connection status
    pub connected: bool,
    /// Price history for momentum calculation (last 60 prices ~1 min)
    pub price_history: Vec<Decimal>,
    /// Binance price (secondary source for cross-validation)
    pub binance_price: Option<Decimal>,
    /// Binance connection status
    pub binance_connected: bool,
    /// Timestamp of last Binance update
    pub last_binance_update: Instant,
    /// Max history size
    max_history: usize,
}

impl Default for BtcPriceState {
    fn default() -> Self {
        Self {
            current_price: Decimal::ZERO,
            market_open_price: None,
            last_update: Instant::now(),
            connected: false,
            price_history: Vec::with_capacity(60),
            binance_price: None,
            binance_connected: false,
            last_binance_update: Instant::now(),
            max_history: 60,
        }
    }
}

impl BtcPriceState {
    /// Add price to history
    pub fn add_price(&mut self, price: Decimal) {
        self.price_history.push(price);
        if self.price_history.len() > self.max_history {
            self.price_history.remove(0);
        }
    }

    /// Get rate of change (momentum) - last N prices
    pub fn get_roc(&self, periods: usize) -> Option<Decimal> {
        if self.price_history.len() < periods + 1 {
            return None;
        }
        let current = *self.price_history.last()?;
        let past = self.price_history[self.price_history.len() - periods - 1];
        if past == Decimal::ZERO {
            return None;
        }
        Some((current - past) / past * dec!(100))
    }

    /// Get simple moving average of last N prices
    pub fn get_sma(&self, periods: usize) -> Option<Decimal> {
        if self.price_history.len() < periods {
            return None;
        }
        let sum: Decimal = self.price_history.iter()
            .rev()
            .take(periods)
            .copied()
            .sum();
        Some(sum / Decimal::from(periods as i64))
    }

    /// Check if price is trending (above SMA = bullish, below = bearish)
    pub fn is_trending_up(&self) -> Option<bool> {
        let sma = self.get_sma(20)?;
        Some(self.current_price > sma)
    }

    /// Get momentum strength (0-100)
    pub fn get_momentum_strength(&self) -> Option<Decimal> {
        let roc_10 = self.get_roc(10)?;
        let roc_30 = self.get_roc(30)?;

        // Combine short and long term momentum
        let combined = (roc_10.abs() + roc_30.abs()) * dec!(50);
        Some(combined.min(dec!(100)))
    }
}

/// BTC Price Feed - connects to Binance WebSocket
pub struct BtcPriceFeed {
    state: Arc<RwLock<BtcPriceState>>,
}

impl BtcPriceFeed {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(BtcPriceState::default())),
        }
    }

    /// Get shared state handle
    pub fn state(&self) -> Arc<RwLock<BtcPriceState>> {
        self.state.clone()
    }

    /// Get current BTC price
    pub fn get_price(&self) -> Decimal {
        self.state.read().current_price
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.state.read().connected
    }

    /// Mark the current price as the market open price
    /// Call this when a new 15-minute market period starts
    pub fn mark_market_open(&self) {
        let mut state = self.state.write();
        state.market_open_price = Some(state.current_price);
        info!(
            "Marked market open BTC price: ${}",
            state.current_price.round_dp(2)
        );
    }

    /// Clear market open price (call when market resolves)
    pub fn clear_market_open(&self) {
        self.state.write().market_open_price = None;
    }

    /// Get the predicted outcome based on BTC price movement
    /// Returns Some(true) for UP, Some(false) for DOWN, None if no movement
    pub fn get_predicted_outcome(&self) -> Option<bool> {
        let state = self.state.read();
        let open = state.market_open_price?;
        let now = state.current_price;

        if now > open {
            Some(true) // UP
        } else if now < open {
            Some(false) // DOWN
        } else {
            None // No movement
        }
    }

    /// Get price change since market open (in dollars)
    pub fn get_price_change(&self) -> Option<Decimal> {
        let state = self.state.read();
        let open = state.market_open_price?;
        Some(state.current_price - open)
    }

    /// Get price change percentage since market open
    pub fn get_price_change_pct(&self) -> Option<Decimal> {
        let state = self.state.read();
        let open = state.market_open_price?;
        if open == Decimal::ZERO {
            return None;
        }
        Some((state.current_price - open) / open * dec!(100))
    }

    /// Get confidence level (0-100) based on magnitude of price change
    /// Higher confidence = larger BTC move = more certain outcome
    pub fn get_confidence(&self) -> Decimal {
        let pct_change = self.get_price_change_pct().unwrap_or(Decimal::ZERO).abs();
        // Scale: 0.01% = low confidence, 0.1% = high confidence
        // Cap at 100
        (pct_change * dec!(1000)).min(dec!(100))
    }

    /// Get momentum-enhanced confidence (combines price change + momentum)
    pub fn get_momentum_confidence(&self) -> Decimal {
        let base_conf = self.get_confidence();
        let state = self.state.read();

        if let Some(momentum) = state.get_momentum_strength() {
            // Boost confidence if momentum confirms direction
            let roc = state.get_roc(10).unwrap_or(Decimal::ZERO);
            let price_dir_up = self.get_predicted_outcome().unwrap_or(true);
            let momentum_confirms = (roc > Decimal::ZERO && price_dir_up)
                                  || (roc < Decimal::ZERO && !price_dir_up);

            if momentum_confirms {
                (base_conf + momentum * dec!(0.5)).min(dec!(100))
            } else {
                // Momentum diverges - reduce confidence
                (base_conf - momentum * dec!(0.3)).max(Decimal::ZERO)
            }
        } else {
            base_conf
        }
    }

    /// Check if current move is supported by momentum
    pub fn is_momentum_aligned(&self) -> bool {
        let state = self.state.read();
        let roc = state.get_roc(10).unwrap_or(Decimal::ZERO);
        let sma_trend = state.is_trending_up();
        let price_dir = self.get_predicted_outcome();

        match (price_dir, sma_trend) {
            (Some(true), Some(true)) => roc > Decimal::ZERO,  // UP with bullish momentum
            (Some(false), Some(false)) => roc < Decimal::ZERO, // DOWN with bearish momentum
            _ => false
        }
    }

    /// Get Binance BTC price
    pub fn get_binance_price(&self) -> Option<Decimal> {
        self.state.read().binance_price
    }

    /// Check if both exchanges agree on direction (cross-validation)
    pub fn exchanges_agree(&self) -> bool {
        let state = self.state.read();
        let open = match state.market_open_price {
            Some(p) => p,
            None => return true, // No open yet, assume agree
        };

        let coinbase_up = state.current_price > open;
        let coinbase_down = state.current_price < open;

        if let Some(binance) = state.binance_price {
            let binance_up = binance > open;
            let binance_down = binance < open;

            // Both must agree on direction
            (coinbase_up && binance_up) || (coinbase_down && binance_down)
        } else {
            true // Binance not available, trust Coinbase
        }
    }

    /// Get exchange divergence (difference between Coinbase and Binance)
    pub fn get_exchange_divergence(&self) -> Option<Decimal> {
        let state = self.state.read();
        let binance = state.binance_price?;
        Some((state.current_price - binance).abs())
    }

    /// Check if Binance is connected
    pub fn is_binance_connected(&self) -> bool {
        self.state.read().binance_connected
    }

    /// Start the WebSocket connections to Coinbase + Binance
    pub async fn connect(&self) -> Result<()> {
        let state = self.state.clone();

        // Spawn Coinbase WebSocket
        let coinbase_state = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = run_coinbase_ws(coinbase_state.clone()).await {
                    error!("Coinbase WebSocket error: {}", e);
                    coinbase_state.write().connected = false;
                }

                warn!("Coinbase WebSocket disconnected, reconnecting in 1s...");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });

        // Spawn Binance WebSocket (secondary source)
        let binance_state = state.clone();
        tokio::spawn(async move {
            loop {
                if let Err(e) = run_binance_ws(binance_state.clone()).await {
                    error!("Binance WebSocket error: {}", e);
                    binance_state.write().binance_connected = false;
                }

                warn!("Binance WebSocket disconnected, reconnecting in 2s...");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });

        // Wait for first price update (from either exchange)
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let state_read = self.state.read();
            if state_read.current_price > Decimal::ZERO {
                let binance_str = match state_read.binance_price {
                    Some(p) => format!("${}", p.round_dp(2)),
                    None => "connecting...".to_string(),
                };
                drop(state_read);
                info!("BTC feeds: Coinbase ${}, Binance {}",
                      self.get_price().round_dp(2), binance_str);
                return Ok(());
            }
        }

        warn!("BTC price feed slow to connect, continuing anyway...");
        Ok(())
    }
}

impl Default for BtcPriceFeed {
    fn default() -> Self {
        Self::new()
    }
}

/// Coinbase ticker message
#[derive(Debug, Deserialize)]
struct CoinbaseTicker {
    #[serde(rename = "type")]
    msg_type: String,
    price: Option<String>,
    time: Option<String>,
}

/// Binance ticker message
#[derive(Debug, Deserialize)]
struct BinanceTicker {
    #[serde(rename = "e")]
    event_type: Option<String>,
    #[serde(rename = "c")]
    price: Option<String>,
}

/// Kraken ticker message
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum KrakenMessage {
    Ticker(Vec<serde_json::Value>),
    Status { status: String },
    Subscribed { channelName: Option<String> },
}

/// Run Binance WebSocket connection (or Kraken as fallback)
async fn run_binance_ws(state: Arc<RwLock<BtcPriceState>>) -> Result<()> {
    // Try Binance first, fall back to Kraken if geo-blocked
    if let Err(e) = run_binance_ws_inner(state.clone()).await {
        warn!("Binance failed (possibly geo-blocked): {}, trying Kraken...", e);
        return run_kraken_ws(state).await;
    }
    Ok(())
}

/// Binance WebSocket implementation
async fn run_binance_ws_inner(state: Arc<RwLock<BtcPriceState>>) -> Result<()> {
    let url = "wss://stream.binance.com:9443/ws/btcusdt@ticker";

    info!("Connecting to Binance WebSocket...");

    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(url))
        .await
        .context("Binance WebSocket connection timeout")?
        .context("Failed to connect to Binance WebSocket")?;

    info!("Binance WebSocket connected");
    state.write().binance_connected = true;

    let (_write, mut read) = ws_stream.split();

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(ticker) = serde_json::from_str::<BinanceTicker>(&text) {
                    if let Some(price_str) = ticker.price {
                        if let Ok(price) = price_str.parse::<Decimal>() {
                            let mut s = state.write();
                            s.binance_price = Some(price);
                            s.last_binance_update = Instant::now();
                        }
                    }
                }
            }
            Ok(Message::Ping(data)) => {
                // Binance sends pings
                debug!("Binance ping received");
            }
            Ok(Message::Close(_)) => {
                info!("Binance WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("Binance WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    state.write().binance_connected = false;
    Ok(())
}

/// Kraken WebSocket implementation (fallback for geo-blocked regions)
async fn run_kraken_ws(state: Arc<RwLock<BtcPriceState>>) -> Result<()> {
    let url = "wss://ws.kraken.com";

    info!("Connecting to Kraken WebSocket...");

    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(url))
        .await
        .context("Kraken WebSocket connection timeout")?
        .context("Failed to connect to Kraken WebSocket")?;

    info!("Kraken WebSocket connected");
    state.write().binance_connected = true; // Reuse field for secondary exchange

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to BTC/USD ticker
    let subscribe_msg = serde_json::json!({
        "event": "subscribe",
        "pair": ["XBT/USD"],
        "subscription": {"name": "ticker"}
    });

    write
        .send(Message::Text(subscribe_msg.to_string()))
        .await
        .context("Failed to send Kraken subscribe message")?;

    info!("Subscribed to Kraken XBT/USD ticker");

    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                // Kraken sends array format: [channelID, data, "ticker", "XBT/USD"]
                if let Ok(arr) = serde_json::from_str::<Vec<serde_json::Value>>(&text) {
                    if arr.len() >= 2 {
                        if let Some(data) = arr.get(1) {
                            // Get "c" field (last trade close price)
                            if let Some(c) = data.get("c") {
                                if let Some(arr) = c.as_array() {
                                    if let Some(price_val) = arr.first() {
                                        if let Some(price_str) = price_val.as_str() {
                                            if let Ok(price) = price_str.parse::<Decimal>() {
                                                let mut s = state.write();
                                                s.binance_price = Some(price);
                                                s.last_binance_update = Instant::now();
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Ok(Message::Ping(data)) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) => {
                info!("Kraken WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("Kraken WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    state.write().binance_connected = false;
    Ok(())
}

/// Run the Coinbase WebSocket connection
async fn run_coinbase_ws(state: Arc<RwLock<BtcPriceState>>) -> Result<()> {
    // Coinbase WebSocket for BTC-USD ticker
    let url = "wss://ws-feed.exchange.coinbase.com";

    info!("Connecting to Coinbase WebSocket: {}", url);

    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_async(url))
        .await
        .context("Coinbase WebSocket connection timeout")?
        .context("Failed to connect to Coinbase WebSocket")?;

    info!("Coinbase WebSocket connected");
    state.write().connected = true;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to BTC-USD ticker
    let subscribe_msg = serde_json::json!({
        "type": "subscribe",
        "product_ids": ["BTC-USD"],
        "channels": ["ticker"]
    });

    write
        .send(Message::Text(subscribe_msg.to_string()))
        .await
        .context("Failed to send subscribe message")?;

    info!("Subscribed to BTC-USD ticker");

    // Process messages
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Ok(ticker) = serde_json::from_str::<CoinbaseTicker>(&text) {
                    if ticker.msg_type == "ticker" {
                        if let Some(price_str) = ticker.price {
                            if let Ok(price) = price_str.parse::<Decimal>() {
                                let mut s = state.write();
                                s.current_price = price;
                                s.last_update = Instant::now();
                                s.add_price(price); // Track for momentum

                                // Log every ~100th update to avoid spam
                                static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
                                let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                if count % 100 == 0 {
                                    debug!("BTC price: ${}", price.round_dp(2));
                                }
                            }
                        }
                    }
                }
            }
            Ok(Message::Ping(data)) => {
                let _ = write.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) => {
                info!("Coinbase WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("Coinbase WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    state.write().connected = false;
    Ok(())
}

/// Spawn BTC price feed and return handle
pub fn spawn_btc_price_feed() -> Arc<BtcPriceFeed> {
    let feed = Arc::new(BtcPriceFeed::new());
    let feed_clone = feed.clone();

    tokio::spawn(async move {
        if let Err(e) = feed_clone.connect().await {
            error!("Failed to start BTC price feed: {}", e);
        }
    });

    feed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_predicted_outcome() {
        let feed = BtcPriceFeed::new();

        // Set initial price
        {
            let mut state = feed.state.write();
            state.current_price = dec!(100000);
            state.market_open_price = Some(dec!(100000));
        }

        // No movement
        assert_eq!(feed.get_predicted_outcome(), None);

        // Price went up
        feed.state.write().current_price = dec!(100100);
        assert_eq!(feed.get_predicted_outcome(), Some(true)); // UP

        // Price went down
        feed.state.write().current_price = dec!(99900);
        assert_eq!(feed.get_predicted_outcome(), Some(false)); // DOWN
    }

    #[test]
    fn test_confidence() {
        let feed = BtcPriceFeed::new();

        // Set initial price at $100,000
        {
            let mut state = feed.state.write();
            state.current_price = dec!(100000);
            state.market_open_price = Some(dec!(100000));
        }

        // No change = 0 confidence
        assert_eq!(feed.get_confidence(), dec!(0));

        // 0.1% change = high confidence
        feed.state.write().current_price = dec!(100100);
        let conf = feed.get_confidence();
        assert!(conf > dec!(50));
    }
}
