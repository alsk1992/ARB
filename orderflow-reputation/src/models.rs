use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletStats {
    pub wallet_address: String,
    pub first_trade_at: Option<DateTime<Utc>>,
    pub last_trade_at: Option<DateTime<Utc>>,
    pub total_trades: i32,
    pub total_volume_usd: f64,

    // Performance metrics
    pub winning_trades: i32,
    pub losing_trades: i32,
    pub win_rate: Option<f64>,
    pub total_pnl_usd: f64,
    pub avg_profit_per_trade_pct: Option<f64>,

    // Behavioral patterns
    pub avg_position_size_usd: Option<f64>,
    pub avg_entry_minute: Option<f64>,
    pub avg_hold_duration_minutes: Option<f64>,

    // Risk metrics
    pub sharpe_ratio: Option<f64>,
    pub max_drawdown_pct: Option<f64>,
    pub volatility: Option<f64>,

    // Reputation
    pub reputation_score: f64,
    pub confidence_level: f64,
    pub trader_tier: String,
}

#[derive(Debug, Clone, Copy)]
pub enum TraderTier {
    Whale,    // Score 8-10
    Smart,    // Score 6-8
    Average,  // Score 4-6
    Novice,   // Score 2-4
    Degen,    // Score 0-2
}

impl TraderTier {
    pub fn from_score(score: f64) -> Self {
        if score >= 8.0 {
            TraderTier::Whale
        } else if score >= 6.0 {
            TraderTier::Smart
        } else if score >= 4.0 {
            TraderTier::Average
        } else if score >= 2.0 {
            TraderTier::Novice
        } else {
            TraderTier::Degen
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            TraderTier::Whale => "WHALE",
            TraderTier::Smart => "SMART",
            TraderTier::Average => "AVERAGE",
            TraderTier::Novice => "NOVICE",
            TraderTier::Degen => "DEGEN",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ReputationScore {
    pub score: f64,           // 0.0 to 10.0
    pub confidence: f64,      // 0.0 to 1.0 (how confident we are)
    pub tier: TraderTier,
}

impl ReputationScore {
    pub fn new(score: f64, confidence: f64) -> Self {
        let score = score.max(0.0).min(10.0);
        let confidence = confidence.max(0.0).min(1.0);
        let tier = TraderTier::from_score(score);

        Self {
            score,
            confidence,
            tier,
        }
    }
}
