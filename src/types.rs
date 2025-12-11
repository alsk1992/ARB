use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Market information from Gamma API
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Market {
    pub condition_id: String,
    pub question: String,
    pub tokens: Vec<Token>,
    pub outcome_prices: Option<String>, // JSON string like "[\"0.5\", \"0.5\"]"
    pub end_date_iso: Option<String>,
    pub active: bool,
    pub closed: bool,
    pub neg_risk: Option<bool>,
    pub tick_size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub token_id: String,
    pub outcome: String,
}

/// Event from Gamma API
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    pub id: String,
    pub slug: String,
    pub title: String,
    pub markets: Vec<Market>,
    pub active: bool,
    pub closed: bool,
}

/// Orderbook from CLOB
#[derive(Debug, Clone, Deserialize)]
pub struct Orderbook {
    pub market: String,
    pub asset_id: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub hash: String,
    pub timestamp: Option<String>,
    pub min_order_size: Option<String>,
    pub tick_size: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PriceLevel {
    pub price: String,
    pub size: String,
}

/// Order to submit to CLOB
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Order {
    pub order: SignedOrder,
    pub owner: String,
    pub order_type: OrderType,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedOrder {
    pub salt: String,
    pub maker: String,
    pub signer: String,
    pub taker: String,
    pub token_id: String,
    pub maker_amount: String,
    pub taker_amount: String,
    pub expiration: String,
    pub nonce: String,
    pub fee_rate_bps: String,
    // CLOB API expects integer side: 0=BUY, 1=SELL (not string)
    #[serde(serialize_with = "serialize_side_as_int")]
    pub side: Side,
    pub signature_type: u8,
    pub signature: String,
}

fn serialize_side_as_int<S>(side: &Side, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match side {
        Side::Buy => serializer.serialize_u8(0),
        Side::Sell => serializer.serialize_u8(1),
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum Side {
    Buy,
    Sell,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum OrderType {
    Gtc, // Good til cancelled
    Gtd, // Good til date
    Fok, // Fill or kill
}

/// WebSocket message types
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type")]
pub enum WsMessage {
    #[serde(rename = "subscribed")]
    Subscribed { channel: String },

    #[serde(rename = "error")]
    Error { message: String },

    #[serde(other)]
    Unknown,
}

/// WebSocket orderbook update
#[derive(Debug, Clone, Deserialize)]
pub struct WsOrderbookUpdate {
    pub asset_id: String,
    pub market: String,
    pub bids: Vec<PriceLevel>,
    pub asks: Vec<PriceLevel>,
    pub hash: String,
    pub timestamp: String,
}

/// Position tracking
#[derive(Debug, Clone, Default)]
pub struct Position {
    pub up_shares: Decimal,
    pub up_cost: Decimal,
    pub down_shares: Decimal,
    pub down_cost: Decimal,
}

impl Position {
    pub fn total_cost(&self) -> Decimal {
        self.up_cost + self.down_cost
    }

    pub fn min_shares(&self) -> Decimal {
        self.up_shares.min(self.down_shares)
    }

    pub fn guaranteed_payout(&self) -> Decimal {
        self.min_shares() // Each share pays $1
    }

    pub fn locked_profit(&self) -> Decimal {
        self.guaranteed_payout() - self.total_cost()
    }

    pub fn is_balanced(&self) -> bool {
        let diff = (self.up_shares - self.down_shares).abs();
        let avg = (self.up_shares + self.down_shares) / Decimal::from(2);
        if avg.is_zero() {
            return true;
        }
        diff / avg < Decimal::from_str_exact("0.1").unwrap() // Within 10%
    }
}

/// Trade fill notification
#[derive(Debug, Clone, Deserialize)]
pub struct TradeFill {
    pub asset_id: String,
    pub market: String,
    pub side: Side,
    pub price: String,
    pub size: String,
    pub order_id: String,
    pub status: String,
}

/// BTC 15-min market info
#[derive(Debug, Clone)]
pub struct BtcMarket {
    pub event_slug: String,
    pub condition_id: String,
    pub title: String,
    pub up_token_id: String,
    pub down_token_id: String,
    pub end_time: chrono::DateTime<chrono::Utc>,
    pub tick_size: Decimal,
    pub neg_risk: bool,
}
