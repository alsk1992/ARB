use anyhow::{Context, Result};
use rust_decimal::Decimal;
use std::env;

#[derive(Debug, Clone)]
pub struct Config {
    // API Credentials
    pub api_key: String,
    pub api_secret: String,
    pub api_passphrase: String,

    // Wallet
    pub address: String,
    pub private_key: String,

    // Trading Parameters
    pub max_position_usd: Decimal,
    pub account_balance: Decimal,  // For dynamic position sizing
    pub target_spread_percent: Decimal,
    pub min_spread_percent: Decimal,

    // Ladder Settings
    pub ladder_levels: u32,
    pub order_size_per_level: Decimal,

    // Mode
    pub dry_run: bool,
    pub log_level: String,

    // Alerts
    pub discord_webhook: Option<String>,

    // Endpoints
    pub clob_url: String,
    pub ws_url: String,
    pub gamma_url: String,

    // Lambda proxy for bypassing Cloudflare (optional)
    pub lambda_proxy_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Config {
            // API Credentials
            api_key: env::var("POLY_API_KEY").context("POLY_API_KEY not set")?,
            api_secret: env::var("POLY_API_SECRET").context("POLY_API_SECRET not set")?,
            api_passphrase: env::var("POLY_API_PASSPHRASE").context("POLY_API_PASSPHRASE not set")?,

            // Wallet
            address: env::var("POLY_ADDRESS").context("POLY_ADDRESS not set")?,
            private_key: env::var("PRIVATE_KEY").context("PRIVATE_KEY not set")?,

            // Trading Parameters
            max_position_usd: env::var("MAX_POSITION_USD")
                .unwrap_or_else(|_| "1200".to_string())
                .parse()
                .context("Invalid MAX_POSITION_USD")?,
            account_balance: env::var("ACCOUNT_BALANCE")
                .unwrap_or_else(|_| "38".to_string())  // Default $38
                .parse()
                .context("Invalid ACCOUNT_BALANCE")?,
            target_spread_percent: env::var("TARGET_SPREAD_PERCENT")
                .unwrap_or_else(|_| "4".to_string())
                .parse()
                .context("Invalid TARGET_SPREAD_PERCENT")?,
            min_spread_percent: env::var("MIN_SPREAD_PERCENT")
                .unwrap_or_else(|_| "2".to_string())
                .parse()
                .context("Invalid MIN_SPREAD_PERCENT")?,

            // Ladder Settings (30 levels like the pros)
            ladder_levels: env::var("LADDER_LEVELS")
                .unwrap_or_else(|_| "30".to_string())
                .parse()
                .context("Invalid LADDER_LEVELS")?,
            order_size_per_level: env::var("ORDER_SIZE_PER_LEVEL")
                .unwrap_or_else(|_| "75".to_string())
                .parse()
                .context("Invalid ORDER_SIZE_PER_LEVEL")?,

            // Mode
            dry_run: env::var("DRY_RUN")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
            log_level: env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string()),

            // Alerts
            discord_webhook: env::var("DISCORD_WEBHOOK").ok(),

            // Endpoints
            clob_url: "https://clob.polymarket.com".to_string(),
            ws_url: "wss://ws-subscriptions-clob.polymarket.com/ws/market".to_string(),
            gamma_url: "https://gamma-api.polymarket.com".to_string(),

            // Lambda proxy URL (set LAMBDA_PROXY_URL to enable)
            lambda_proxy_url: env::var("LAMBDA_PROXY_URL").ok(),
        })
    }
}
