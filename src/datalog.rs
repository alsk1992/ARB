use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

/// Data logger for saving market data, orders, and results for ML analysis
pub struct DataLogger {
    log_dir: String,
    session_id: String,
}

/// Price level with price and size
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
    pub price: Decimal,
    pub size: Decimal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketSnapshot {
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub market_title: String,
    pub end_time: DateTime<Utc>,
    pub up_token_id: String,
    pub down_token_id: String,
    pub up_best_bid: Option<Decimal>,
    pub up_best_ask: Option<Decimal>,
    pub down_best_bid: Option<Decimal>,
    pub down_best_ask: Option<Decimal>,
    pub combined_ask: Option<Decimal>,
    pub spread_pct: Option<Decimal>,
    // Orderbook depth - top 5 levels for each side
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub up_asks: Vec<PriceLevel>,    // UP token ask levels (price, size)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub up_bids: Vec<PriceLevel>,    // UP token bid levels
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub down_asks: Vec<PriceLevel>,  // DOWN token ask levels
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub down_bids: Vec<PriceLevel>,  // DOWN token bid levels
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderLog {
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub side: String,        // "UP" or "DOWN"
    pub action: String,      // "BUY" or "SELL"
    pub price: Decimal,
    pub size: Decimal,
    pub order_id: Option<String>,
    pub is_dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FillLog {
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub side: String,
    pub price: Decimal,
    pub size: Decimal,
    pub order_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub market_id: String,
    pub market_title: String,
    pub total_up_shares: Decimal,
    pub total_down_shares: Decimal,
    pub total_up_cost: Decimal,
    pub total_down_cost: Decimal,
    pub total_cost: Decimal,
    pub min_shares: Decimal,
    pub guaranteed_payout: Decimal,
    pub locked_profit: Decimal,
    pub profit_pct: Decimal,
    pub is_dry_run: bool,
    pub orders_placed: u32,
    pub fills_received: u32,
}

impl DataLogger {
    pub fn new(log_dir: &str) -> Result<Self> {
        let session_id = Utc::now().format("%Y%m%d_%H%M%S").to_string();

        // Create log directory if it doesn't exist
        fs::create_dir_all(log_dir)?;

        Ok(Self {
            log_dir: log_dir.to_string(),
            session_id,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Log a market snapshot (orderbook state)
    pub fn log_market_snapshot(&self, snapshot: &MarketSnapshot) -> Result<()> {
        let file_path = format!("{}/snapshots_{}.jsonl", self.log_dir, self.session_id);
        self.append_json(&file_path, snapshot)
    }

    /// Log an order (placed or would-be-placed in dry run)
    pub fn log_order(&self, order: &OrderLog) -> Result<()> {
        let file_path = format!("{}/orders_{}.jsonl", self.log_dir, self.session_id);
        self.append_json(&file_path, order)
    }

    /// Log a fill
    pub fn log_fill(&self, fill: &FillLog) -> Result<()> {
        let file_path = format!("{}/fills_{}.jsonl", self.log_dir, self.session_id);
        self.append_json(&file_path, fill)
    }

    /// Log session summary
    pub fn log_session_summary(&self, summary: &SessionSummary) -> Result<()> {
        let file_path = format!("{}/summaries.jsonl", self.log_dir);
        self.append_json(&file_path, summary)
    }

    /// Append JSON line to file
    fn append_json<T: Serialize>(&self, file_path: &str, data: &T) -> Result<()> {
        let json = serde_json::to_string(data)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;
        writeln!(file, "{}", json)?;
        Ok(())
    }

    /// Get all snapshots from a session
    pub fn read_snapshots(log_dir: &str, session_id: &str) -> Result<Vec<MarketSnapshot>> {
        let file_path = format!("{}/snapshots_{}.jsonl", log_dir, session_id);
        Self::read_jsonl(&file_path)
    }

    /// Get all orders from a session
    pub fn read_orders(log_dir: &str, session_id: &str) -> Result<Vec<OrderLog>> {
        let file_path = format!("{}/orders_{}.jsonl", log_dir, session_id);
        Self::read_jsonl(&file_path)
    }

    /// Get all fills from a session
    pub fn read_fills(log_dir: &str, session_id: &str) -> Result<Vec<FillLog>> {
        let file_path = format!("{}/fills_{}.jsonl", log_dir, session_id);
        Self::read_jsonl(&file_path)
    }

    /// Get all session summaries
    pub fn read_summaries(log_dir: &str) -> Result<Vec<SessionSummary>> {
        let file_path = format!("{}/summaries.jsonl", log_dir);
        Self::read_jsonl(&file_path)
    }

    /// Read JSONL file into vector
    fn read_jsonl<T: for<'de> Deserialize<'de>>(file_path: &str) -> Result<Vec<T>> {
        if !Path::new(file_path).exists() {
            return Ok(vec![]);
        }

        let content = fs::read_to_string(file_path)?;
        let mut results = Vec::new();

        for line in content.lines() {
            if !line.trim().is_empty() {
                let item: T = serde_json::from_str(line)?;
                results.push(item);
            }
        }

        Ok(results)
    }

    /// List all session IDs in log directory
    pub fn list_sessions(log_dir: &str) -> Result<Vec<String>> {
        let mut sessions = Vec::new();

        if let Ok(entries) = fs::read_dir(log_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("snapshots_") && name.ends_with(".jsonl") {
                    let session_id = name
                        .trim_start_matches("snapshots_")
                        .trim_end_matches(".jsonl")
                        .to_string();
                    sessions.push(session_id);
                }
            }
        }

        sessions.sort();
        Ok(sessions)
    }
}
