//! Trade Database - SQLite logging for all trades
//!
//! Logs every trade for analysis and performance tracking.

use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rusqlite::{Connection, params};
use std::path::Path;
use tracing::info;

/// Trade record for database
#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub timestamp: DateTime<Utc>,
    pub market_id: String,
    pub market_title: String,
    pub direction: String,        // "UP" or "DOWN"
    pub entry_price: Decimal,
    pub shares: Decimal,
    pub btc_open_price: Decimal,
    pub btc_entry_price: Decimal,
    pub btc_change_pct: Decimal,
    pub confidence_score: Decimal,
    pub minute_of_entry: f64,
    pub outcome: String,          // "WIN", "LOSS", "PENDING"
    pub profit: Decimal,
    pub is_dry_run: bool,
}

/// Trade database manager
pub struct TradeDb {
    conn: Connection,
}

impl TradeDb {
    /// Create new trade database
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(path)?;

        // Create trades table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS trades (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                market_id TEXT NOT NULL,
                market_title TEXT NOT NULL,
                direction TEXT NOT NULL,
                entry_price REAL NOT NULL,
                shares REAL NOT NULL,
                btc_open_price REAL NOT NULL,
                btc_entry_price REAL NOT NULL,
                btc_change_pct REAL NOT NULL,
                confidence_score REAL NOT NULL,
                minute_of_entry REAL NOT NULL,
                outcome TEXT NOT NULL,
                profit REAL NOT NULL,
                is_dry_run INTEGER NOT NULL
            )",
            [],
        )?;

        // Create index for faster queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON trades(timestamp)",
            [],
        )?;

        info!("Trade database initialized");
        Ok(Self { conn })
    }

    /// Insert a new trade record
    pub fn insert_trade(&self, trade: &TradeRecord) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO trades (
                timestamp, market_id, market_title, direction, entry_price,
                shares, btc_open_price, btc_entry_price, btc_change_pct,
                confidence_score, minute_of_entry, outcome, profit, is_dry_run
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                trade.timestamp.to_rfc3339(),
                trade.market_id,
                trade.market_title,
                trade.direction,
                trade.entry_price.to_string().parse::<f64>().unwrap_or(0.0),
                trade.shares.to_string().parse::<f64>().unwrap_or(0.0),
                trade.btc_open_price.to_string().parse::<f64>().unwrap_or(0.0),
                trade.btc_entry_price.to_string().parse::<f64>().unwrap_or(0.0),
                trade.btc_change_pct.to_string().parse::<f64>().unwrap_or(0.0),
                trade.confidence_score.to_string().parse::<f64>().unwrap_or(0.0),
                trade.minute_of_entry,
                trade.outcome,
                trade.profit.to_string().parse::<f64>().unwrap_or(0.0),
                trade.is_dry_run as i32,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    /// Update trade outcome
    pub fn update_outcome(&self, id: i64, outcome: &str, profit: Decimal) -> Result<()> {
        self.conn.execute(
            "UPDATE trades SET outcome = ?1, profit = ?2 WHERE id = ?3",
            params![
                outcome,
                profit.to_string().parse::<f64>().unwrap_or(0.0),
                id,
            ],
        )?;
        Ok(())
    }

    /// Get trade statistics
    pub fn get_stats(&self, dry_run_only: bool) -> Result<TradeStats> {
        let where_clause = if dry_run_only {
            "WHERE is_dry_run = 1"
        } else {
            ""
        };

        let total: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM trades {}", where_clause),
            [],
            |row| row.get(0),
        )?;

        let wins: i64 = self.conn.query_row(
            &format!("SELECT COUNT(*) FROM trades {} {} outcome = 'WIN'",
                where_clause,
                if dry_run_only { "AND" } else { "WHERE" }
            ),
            [],
            |row| row.get(0),
        ).unwrap_or(0);

        let total_profit: f64 = self.conn.query_row(
            &format!("SELECT COALESCE(SUM(profit), 0) FROM trades {}", where_clause),
            [],
            |row| row.get(0),
        ).unwrap_or(0.0);

        let avg_entry_minute: f64 = self.conn.query_row(
            &format!("SELECT COALESCE(AVG(minute_of_entry), 0) FROM trades {}", where_clause),
            [],
            |row| row.get(0),
        ).unwrap_or(0.0);

        Ok(TradeStats {
            total_trades: total as u32,
            wins: wins as u32,
            losses: (total - wins) as u32,
            win_rate: if total > 0 { wins as f64 / total as f64 * 100.0 } else { 0.0 },
            total_profit,
            avg_entry_minute,
        })
    }

    /// Get recent trades
    pub fn get_recent_trades(&self, limit: i32) -> Result<Vec<TradeRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT timestamp, market_id, market_title, direction, entry_price,
                    shares, btc_open_price, btc_entry_price, btc_change_pct,
                    confidence_score, minute_of_entry, outcome, profit, is_dry_run
             FROM trades ORDER BY timestamp DESC LIMIT ?"
        )?;

        let trades = stmt.query_map([limit], |row| {
            Ok(TradeRecord {
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(0)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                market_id: row.get(1)?,
                market_title: row.get(2)?,
                direction: row.get(3)?,
                entry_price: Decimal::from_str_exact(&row.get::<_, f64>(4)?.to_string())
                    .unwrap_or_default(),
                shares: Decimal::from_str_exact(&row.get::<_, f64>(5)?.to_string())
                    .unwrap_or_default(),
                btc_open_price: Decimal::from_str_exact(&row.get::<_, f64>(6)?.to_string())
                    .unwrap_or_default(),
                btc_entry_price: Decimal::from_str_exact(&row.get::<_, f64>(7)?.to_string())
                    .unwrap_or_default(),
                btc_change_pct: Decimal::from_str_exact(&row.get::<_, f64>(8)?.to_string())
                    .unwrap_or_default(),
                confidence_score: Decimal::from_str_exact(&row.get::<_, f64>(9)?.to_string())
                    .unwrap_or_default(),
                minute_of_entry: row.get(10)?,
                outcome: row.get(11)?,
                profit: Decimal::from_str_exact(&row.get::<_, f64>(12)?.to_string())
                    .unwrap_or_default(),
                is_dry_run: row.get::<_, i32>(13)? != 0,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(trades)
    }
}

/// Trade statistics
#[derive(Debug, Clone)]
pub struct TradeStats {
    pub total_trades: u32,
    pub wins: u32,
    pub losses: u32,
    pub win_rate: f64,
    pub total_profit: f64,
    pub avg_entry_minute: f64,
}

impl std::fmt::Display for TradeStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Trades: {} | Wins: {} | Losses: {} | Win Rate: {:.1}% | Profit: ${:.2} | Avg Entry: min {:.1}",
            self.total_trades, self.wins, self.losses, self.win_rate, self.total_profit, self.avg_entry_minute
        )
    }
}

trait DecimalExt {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error>;
}

impl DecimalExt for Decimal {
    fn from_str_exact(s: &str) -> Result<Decimal, rust_decimal::Error> {
        s.parse()
    }
}
