use anyhow::{Context, Result};
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{debug, error, info};

use crate::config::Config;
use crate::orderbook::OrderbookManager;
use crate::types::{TradeFill, Side};

/// WebSocket event types
#[derive(Debug, Clone)]
pub enum WsEvent {
    /// New market created (for instant detection)
    MarketCreated {
        condition_id: String,
        asset_ids: Vec<String>,
        tick_size: String,
    },
    /// Orderbook snapshot/update
    OrderbookUpdate {
        asset_id: String,
        bids: Vec<(String, String)>,
        asks: Vec<(String, String)>,
    },
    /// Price change event
    PriceChange {
        asset_id: String,
        best_bid: String,
        best_ask: String,
    },
    /// Trade fill (your order got hit)
    TradeFill(TradeFill),
    /// Connection established
    Connected,
    /// Connection lost
    Disconnected,
    /// Error
    Error(String),
}

/// High-performance WebSocket client
pub struct WebSocketClient {
    config: Config,
    event_tx: mpsc::Sender<WsEvent>,
    orderbook_manager: Arc<OrderbookManager>,
    reconnect_count: Arc<RwLock<u32>>,
}

impl WebSocketClient {
    pub fn new(
        config: Config,
        event_tx: mpsc::Sender<WsEvent>,
        orderbook_manager: Arc<OrderbookManager>,
    ) -> Self {
        Self {
            config,
            event_tx,
            orderbook_manager,
            reconnect_count: Arc::new(RwLock::new(0)),
        }
    }

    /// Connect with auto-reconnect
    pub async fn run(&self, token_ids: Vec<String>) -> Result<()> {
        loop {
            match self.connect_and_subscribe(&token_ids).await {
                Ok(_) => {
                    info!("WebSocket connection closed normally");
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    let _ = self.event_tx.send(WsEvent::Error(e.to_string())).await;
                }
            }

            // Increment reconnect counter
            let should_backoff = {
                let mut count = self.reconnect_count.write();
                *count += 1;
                if *count > 10 {
                    *count = 0;
                    true
                } else {
                    false
                }
            };

            if should_backoff {
                error!("Too many reconnects, backing off...");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }

            let _ = self.event_tx.send(WsEvent::Disconnected).await;

            // Exponential backoff for reconnect
            let delay = Duration::from_millis(500);
            info!("Reconnecting in {:?}...", delay);
            tokio::time::sleep(delay).await;
        }
    }

    /// Connect and subscribe to all channels
    async fn connect_and_subscribe(&self, token_ids: &[String]) -> Result<()> {
        // Polymarket uses separate URLs for market vs user channels
        // wss://ws-subscriptions-clob.polymarket.com/ws/market
        // wss://ws-subscriptions-clob.polymarket.com/ws/user
        let url = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
        info!("Connecting to WebSocket: {}", url);

        // Connect with timeout
        let connect_future = connect_async(url);
        let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_future)
            .await
            .context("WebSocket connection timeout")?
            .context("Failed to connect to WebSocket")?;

        info!("WebSocket connected");
        let _ = self.event_tx.send(WsEvent::Connected).await;

        // Reset reconnect counter on successful connect
        *self.reconnect_count.write() = 0;

        let (mut write, mut read) = ws_stream.split();

        // Subscribe to market channel with asset IDs
        // Format: {"assets_ids": ["token_id_1", "token_id_2"], "type": "market"}
        if !token_ids.is_empty() {
            let market_subscribe = json!({
                "assets_ids": token_ids,
                "type": "market"
            });

            write
                .send(Message::Text(market_subscribe.to_string()))
                .await
                .context("Failed to subscribe to market")?;

            info!("Subscribed to market channel for {} tokens", token_ids.len());
        } else {
            // If no token IDs, still need to send something to keep connection
            let market_subscribe = json!({
                "assets_ids": [],
                "type": "market"
            });

            write
                .send(Message::Text(market_subscribe.to_string()))
                .await
                .context("Failed to subscribe to market")?;

            info!("Subscribed to market channel (no tokens yet)");
        }

        // Note: clob_user subscriptions require a separate WebSocket connection to /ws/user
        // The /ws/market endpoint only handles market data (orderbooks, prices)
        // For now, we'll poll for fills via REST API instead

        info!("Subscribed to all channels");

        // Spawn ping task to keep connection alive (every 10 seconds as per docs)
        let ping_write = Arc::new(tokio::sync::Mutex::new(write));
        let ping_writer = ping_write.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            loop {
                interval.tick().await;
                let mut w = ping_writer.lock().await;
                // Polymarket expects text "PING" not binary ping frames
                if w.send(Message::Text("PING".to_string())).await.is_err() {
                    break;
                }
            }
        });

        // Process incoming messages
        while let Some(msg) = read.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Err(e) = self.handle_message(&text).await {
                        debug!("Failed to handle message: {}", e);
                    }
                }
                Ok(Message::Binary(data)) => {
                    // Some messages might come as binary
                    if let Ok(text) = String::from_utf8(data) {
                        if let Err(e) = self.handle_message(&text).await {
                            debug!("Failed to handle binary message: {}", e);
                        }
                    }
                }
                Ok(Message::Ping(data)) => {
                    let mut w = ping_write.lock().await;
                    let _ = w.send(Message::Pong(data)).await;
                }
                Ok(Message::Pong(_)) => {
                    // Connection is alive
                }
                Ok(Message::Close(_)) => {
                    info!("WebSocket closed by server");
                    break;
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                    break;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Handle incoming WebSocket message
    async fn handle_message(&self, text: &str) -> Result<()> {
        let msg: serde_json::Value = serde_json::from_str(text)?;

        // Debug log for unknown messages
        debug!("WS message: {}", &text[..std::cmp::min(200, text.len())]);

        // Handle subscription confirmations
        if let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) {
            match msg_type {
                "subscribed" => {
                    debug!("Subscription confirmed");
                    return Ok(());
                }
                "error" => {
                    let error_msg = msg
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("Unknown error");
                    error!("WebSocket error: {}", error_msg);
                    let _ = self.event_tx.send(WsEvent::Error(error_msg.to_string())).await;
                    return Ok(());
                }
                _ => {}
            }
        }

        // Handle market_created events
        if msg.get("market").is_some() && msg.get("asset_ids").is_some() {
            let condition_id = msg["market"].as_str().unwrap_or("").to_string();
            let asset_ids: Vec<String> = msg["asset_ids"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let tick_size = msg["tick_size"].as_str().unwrap_or("0.01").to_string();

            info!("New market created: {} with {} tokens", condition_id, asset_ids.len());

            let _ = self
                .event_tx
                .send(WsEvent::MarketCreated {
                    condition_id,
                    asset_ids,
                    tick_size,
                })
                .await;

            return Ok(());
        }

        // Handle orderbook updates (agg_orderbook)
        if let Some(asset_id) = msg.get("asset_id").and_then(|a| a.as_str()) {
            // Check if this is an orderbook snapshot
            if msg.get("bids").is_some() && msg.get("asks").is_some() {
                let bids: Vec<(String, String)> = msg["bids"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                let price = v.get("price")?.as_str()?.to_string();
                                let size = v.get("size")?.as_str()?.to_string();
                                Some((price, size))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let asks: Vec<(String, String)> = msg["asks"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|v| {
                                let price = v.get("price")?.as_str()?.to_string();
                                let size = v.get("size")?.as_str()?.to_string();
                                Some((price, size))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                // Update local orderbook
                self.orderbook_manager.update(asset_id, &bids, &asks);

                let _ = self
                    .event_tx
                    .send(WsEvent::OrderbookUpdate {
                        asset_id: asset_id.to_string(),
                        bids,
                        asks,
                    })
                    .await;

                return Ok(());
            }
        }

        // Handle price_change events
        if let Some(pc) = msg.get("pc").and_then(|p| p.as_array()) {
            for change in pc {
                if let (Some(asset_id), Some(best_bid), Some(best_ask)) = (
                    change.get("a").and_then(|a| a.as_str()),
                    change.get("bb").and_then(|b| b.as_str()),
                    change.get("ba").and_then(|a| a.as_str()),
                ) {
                    let _ = self
                        .event_tx
                        .send(WsEvent::PriceChange {
                            asset_id: asset_id.to_string(),
                            best_bid: best_bid.to_string(),
                            best_ask: best_ask.to_string(),
                        })
                        .await;
                }
            }
            return Ok(());
        }

        // Handle trade fills
        if let Some(order_id) = msg.get("order_id").and_then(|o| o.as_str()) {
            if let Some(status) = msg.get("status").and_then(|s| s.as_str()) {
                if status == "FILLED" || status == "MATCHED" {
                    let fill = TradeFill {
                        asset_id: msg
                            .get("asset_id")
                            .and_then(|a| a.as_str())
                            .unwrap_or("")
                            .to_string(),
                        market: msg
                            .get("market")
                            .and_then(|m| m.as_str())
                            .unwrap_or("")
                            .to_string(),
                        side: if msg.get("side").and_then(|s| s.as_str()) == Some("BUY") {
                            Side::Buy
                        } else {
                            Side::Sell
                        },
                        price: msg
                            .get("price")
                            .and_then(|p| p.as_str())
                            .unwrap_or("0")
                            .to_string(),
                        size: msg
                            .get("size")
                            .and_then(|s| s.as_str())
                            .unwrap_or("0")
                            .to_string(),
                        order_id: order_id.to_string(),
                        status: status.to_string(),
                    };

                    info!("Fill received: {} @ {}", fill.size, fill.price);
                    let _ = self.event_tx.send(WsEvent::TradeFill(fill)).await;
                }
            }
        }

        Ok(())
    }
}

/// Spawn WebSocket client in background with orderbook manager
pub fn spawn_websocket_with_orderbook(
    config: Config,
    token_ids: Vec<String>,
    orderbook_manager: Arc<OrderbookManager>,
) -> mpsc::Receiver<WsEvent> {
    let (tx, rx) = mpsc::channel(10000); // Large buffer for high-frequency updates

    // Use std::sync::Arc for the reconnect counter to be Send
    let reconnect_count = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));

    tokio::spawn(async move {
        loop {
            let (event_tx, orderbook_mgr, tokens) = (tx.clone(), orderbook_manager.clone(), token_ids.clone());

            match run_websocket_connection(config.clone(), event_tx, orderbook_mgr, tokens).await {
                Ok(_) => {
                    info!("WebSocket connection closed normally");
                }
                Err(e) => {
                    error!("WebSocket error: {}", e);
                }
            }

            // Reconnect counter
            let count = reconnect_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if count > 10 {
                reconnect_count.store(0, std::sync::atomic::Ordering::Relaxed);
                error!("Too many reconnects, backing off...");
                tokio::time::sleep(Duration::from_secs(30)).await;
            }

            let _ = tx.send(WsEvent::Disconnected).await;

            info!("Reconnecting in 500ms...");
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    });

    rx
}

/// Run a single WebSocket connection
async fn run_websocket_connection(
    _config: Config,
    event_tx: mpsc::Sender<WsEvent>,
    orderbook_manager: Arc<OrderbookManager>,
    token_ids: Vec<String>,
) -> Result<()> {
    // Polymarket WebSocket endpoints:
    // /ws/market - for market data (orderbooks, prices) - public
    // /ws/user - for user data (orders, trades) - requires auth
    let url = "wss://ws-subscriptions-clob.polymarket.com/ws/market";
    info!("Connecting to WebSocket: {}", url);

    // Connect with timeout
    let connect_future = connect_async(url);
    let (ws_stream, _) = tokio::time::timeout(Duration::from_secs(10), connect_future)
        .await
        .context("WebSocket connection timeout")?
        .context("Failed to connect to WebSocket")?;

    info!("WebSocket connected");
    let _ = event_tx.send(WsEvent::Connected).await;

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to market channel with asset IDs
    // Correct format per Polymarket docs: {"assets_ids": [...], "type": "market"}
    let market_subscribe = json!({
        "assets_ids": token_ids,
        "type": "market"
    });

    write
        .send(Message::Text(market_subscribe.to_string()))
        .await
        .context("Failed to subscribe to market")?;

    info!("Subscribed to market channel for {} tokens", token_ids.len());

    // Note: User channel (orders/trades) requires separate connection to /ws/user with auth

    // Keep-alive ping task (every 10 seconds per Polymarket docs)
    let ping_write = Arc::new(tokio::sync::Mutex::new(write));
    let ping_writer = ping_write.clone();

    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(10));
        loop {
            interval.tick().await;
            let mut w = ping_writer.lock().await;
            // Polymarket expects text "PING" not binary ping frames
            if w.send(Message::Text("PING".to_string())).await.is_err() {
                break;
            }
        }
    });

    // Process messages
    while let Some(msg) = read.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                if let Err(e) = handle_ws_message(&text, &event_tx, &orderbook_manager).await {
                    debug!("Failed to handle message: {}", e);
                }
            }
            Ok(Message::Binary(data)) => {
                if let Ok(text) = String::from_utf8(data) {
                    let _ = handle_ws_message(&text, &event_tx, &orderbook_manager).await;
                }
            }
            Ok(Message::Ping(data)) => {
                let mut w = ping_write.lock().await;
                let _ = w.send(Message::Pong(data)).await;
            }
            Ok(Message::Close(_)) => {
                info!("WebSocket closed by server");
                break;
            }
            Err(e) => {
                error!("WebSocket error: {}", e);
                break;
            }
            _ => {}
        }
    }

    Ok(())
}

/// Handle a WebSocket message
async fn handle_ws_message(
    text: &str,
    event_tx: &mpsc::Sender<WsEvent>,
    orderbook_manager: &OrderbookManager,
) -> Result<()> {
    let msg: serde_json::Value = serde_json::from_str(text)?;

    // Handle subscription confirmations
    if let Some(msg_type) = msg.get("type").and_then(|t| t.as_str()) {
        match msg_type {
            "subscribed" => return Ok(()),
            "error" => {
                let error_msg = msg.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
                error!("WebSocket error: {}", error_msg);
                let _ = event_tx.send(WsEvent::Error(error_msg.to_string())).await;
                return Ok(());
            }
            _ => {}
        }
    }

    // Handle market_created
    if msg.get("market").is_some() && msg.get("asset_ids").is_some() {
        let condition_id = msg["market"].as_str().unwrap_or("").to_string();
        let asset_ids: Vec<String> = msg["asset_ids"]
            .as_array()
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let tick_size = msg["tick_size"].as_str().unwrap_or("0.01").to_string();

        info!("New market created: {}", condition_id);
        let _ = event_tx.send(WsEvent::MarketCreated { condition_id, asset_ids, tick_size }).await;
        return Ok(());
    }

    // Handle orderbook updates
    if let Some(asset_id) = msg.get("asset_id").and_then(|a| a.as_str()) {
        if msg.get("bids").is_some() && msg.get("asks").is_some() {
            let bids: Vec<(String, String)> = msg["bids"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            let price = v.get("price")?.as_str()?.to_string();
                            let size = v.get("size")?.as_str()?.to_string();
                            Some((price, size))
                        })
                        .collect()
                })
                .unwrap_or_default();

            let asks: Vec<(String, String)> = msg["asks"]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| {
                            let price = v.get("price")?.as_str()?.to_string();
                            let size = v.get("size")?.as_str()?.to_string();
                            Some((price, size))
                        })
                        .collect()
                })
                .unwrap_or_default();

            orderbook_manager.update(asset_id, &bids, &asks);
            let _ = event_tx.send(WsEvent::OrderbookUpdate { asset_id: asset_id.to_string(), bids, asks }).await;
            return Ok(());
        }
    }

    // Handle price_change
    if let Some(pc) = msg.get("pc").and_then(|p| p.as_array()) {
        for change in pc {
            if let (Some(asset_id), Some(best_bid), Some(best_ask)) = (
                change.get("a").and_then(|a| a.as_str()),
                change.get("bb").and_then(|b| b.as_str()),
                change.get("ba").and_then(|a| a.as_str()),
            ) {
                let _ = event_tx.send(WsEvent::PriceChange {
                    asset_id: asset_id.to_string(),
                    best_bid: best_bid.to_string(),
                    best_ask: best_ask.to_string(),
                }).await;
            }
        }
        return Ok(());
    }

    // Handle trade fills
    if let Some(order_id) = msg.get("order_id").and_then(|o| o.as_str()) {
        if let Some(status) = msg.get("status").and_then(|s| s.as_str()) {
            if status == "FILLED" || status == "MATCHED" {
                let fill = TradeFill {
                    asset_id: msg.get("asset_id").and_then(|a| a.as_str()).unwrap_or("").to_string(),
                    market: msg.get("market").and_then(|m| m.as_str()).unwrap_or("").to_string(),
                    side: if msg.get("side").and_then(|s| s.as_str()) == Some("BUY") { Side::Buy } else { Side::Sell },
                    price: msg.get("price").and_then(|p| p.as_str()).unwrap_or("0").to_string(),
                    size: msg.get("size").and_then(|s| s.as_str()).unwrap_or("0").to_string(),
                    order_id: order_id.to_string(),
                    status: status.to_string(),
                };
                info!("Fill received: {} @ {}", fill.size, fill.price);
                let _ = event_tx.send(WsEvent::TradeFill(fill)).await;
            }
        }
    }

    Ok(())
}

/// Spawn WebSocket client (backward compatible)
pub fn spawn_websocket(config: Config, token_ids: Vec<String>) -> mpsc::Receiver<WsEvent> {
    let orderbook_manager = Arc::new(OrderbookManager::new());
    spawn_websocket_with_orderbook(config, token_ids, orderbook_manager)
}
