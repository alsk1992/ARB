use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
    pub tx_hash: String,
    pub block_number: i64,
    pub timestamp: DateTime<Utc>,
    pub wallet_address: String,
    pub is_maker: bool,
    pub market_id: String,
    pub market_title: Option<String>,
    pub token_id: String,
    pub outcome: Option<String>,
    pub side: String, // BUY or SELL
    pub price: f64,
    pub size: f64,
    pub value_usd: Option<f64>,
    pub order_hash: Option<String>,
    pub fee_paid: Option<f64>,
    pub gas_price: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketMetadata {
    pub condition_id: String,
    pub question: String,
    pub tokens: Vec<TokenInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenInfo {
    pub token_id: String,
    pub outcome: String, // UP/DOWN or YES/NO
}

impl Trade {
    pub fn new(
        tx_hash: String,
        block_number: u64,
        timestamp: DateTime<Utc>,
        wallet_address: String,
        is_maker: bool,
    ) -> Self {
        Self {
            tx_hash,
            block_number: block_number as i64,
            timestamp,
            wallet_address,
            is_maker,
            market_id: String::new(),
            market_title: None,
            token_id: String::new(),
            outcome: None,
            side: String::new(),
            price: 0.0,
            size: 0.0,
            value_usd: None,
            order_hash: None,
            fee_paid: None,
            gas_price: None,
        }
    }
}
