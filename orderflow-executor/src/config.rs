use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ExecutorConfig {
    // API credentials (for auto-execution)
    pub poly_api_key: Option<String>,
    pub poly_api_secret: Option<String>,
    pub poly_api_passphrase: Option<String>,

    // Trading wallet
    pub private_key: Option<String>,

    // Risk management
    pub max_position_usd: f64,
    pub min_signal_confidence: f64,
    pub max_daily_loss: f64,
    pub max_open_positions: i32,

    // Signal thresholds
    pub min_whale_score: f64,  // Follow wallets above this score
    pub max_fade_score: f64,   // Fade wallets below this score

    // Feature flags
    pub enable_paper_trading: bool,
    pub enable_whale_following: bool,
    pub enable_degen_fading: bool,

    // Execution
    pub kelly_fraction: f64,  // Fraction of Kelly criterion to use (0.25 = quarter Kelly)
}

impl ExecutorConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            poly_api_key: std::env::var("POLY_API_KEY").ok(),
            poly_api_secret: std::env::var("POLY_API_SECRET").ok(),
            poly_api_passphrase: std::env::var("POLY_API_PASSPHRASE").ok(),

            private_key: std::env::var("PRIVATE_KEY").ok(),

            max_position_usd: std::env::var("MAX_POSITION_USD")
                .unwrap_or_else(|_| "1000".to_string())
                .parse()?,

            min_signal_confidence: std::env::var("MIN_SIGNAL_CONFIDENCE")
                .unwrap_or_else(|_| "0.7".to_string())
                .parse()?,

            max_daily_loss: std::env::var("MAX_DAILY_LOSS")
                .unwrap_or_else(|_| "500".to_string())
                .parse()?,

            max_open_positions: std::env::var("MAX_OPEN_POSITIONS")
                .unwrap_or_else(|_| "5".to_string())
                .parse()?,

            min_whale_score: std::env::var("MIN_WHALE_SCORE")
                .unwrap_or_else(|_| "7.0".to_string())
                .parse()?,

            max_fade_score: std::env::var("MAX_FADE_SCORE")
                .unwrap_or_else(|_| "3.0".to_string())
                .parse()?,

            enable_paper_trading: std::env::var("ENABLE_PAPER_TRADING")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),

            enable_whale_following: std::env::var("ENABLE_WHALE_FOLLOWING")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),

            enable_degen_fading: std::env::var("ENABLE_DEGEN_FADING")
                .unwrap_or_else(|_| "false".to_string())
                .parse()
                .unwrap_or(false),

            kelly_fraction: std::env::var("KELLY_FRACTION")
                .unwrap_or_else(|_| "0.25".to_string())
                .parse()
                .unwrap_or(0.25),
        })
    }
}
