use parking_lot::RwLock;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::BTreeMap;
use std::sync::Arc;
use tracing::debug;

/// Local orderbook mirror for fast access
#[derive(Debug, Clone)]
pub struct LocalOrderbook {
    pub asset_id: String,
    pub bids: BTreeMap<Decimal, Decimal>, // price -> size (sorted desc)
    pub asks: BTreeMap<Decimal, Decimal>, // price -> size (sorted asc)
    pub last_update: std::time::Instant,
}

impl LocalOrderbook {
    pub fn new(asset_id: &str) -> Self {
        Self {
            asset_id: asset_id.to_string(),
            bids: BTreeMap::new(),
            asks: BTreeMap::new(),
            last_update: std::time::Instant::now(),
        }
    }

    /// Update from WebSocket snapshot
    pub fn update_from_snapshot(&mut self, bids: &[(String, String)], asks: &[(String, String)]) {
        self.bids.clear();
        self.asks.clear();

        for (price, size) in bids {
            if let (Ok(p), Ok(s)) = (price.parse::<Decimal>(), size.parse::<Decimal>()) {
                if s > dec!(0) {
                    self.bids.insert(p, s);
                }
            }
        }

        for (price, size) in asks {
            if let (Ok(p), Ok(s)) = (price.parse::<Decimal>(), size.parse::<Decimal>()) {
                if s > dec!(0) {
                    self.asks.insert(p, s);
                }
            }
        }

        self.last_update = std::time::Instant::now();
    }

    /// Update single price level
    pub fn update_level(&mut self, is_bid: bool, price: Decimal, size: Decimal) {
        let book = if is_bid { &mut self.bids } else { &mut self.asks };

        if size > dec!(0) {
            book.insert(price, size);
        } else {
            book.remove(&price);
        }

        self.last_update = std::time::Instant::now();
    }

    /// Get best bid price
    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.keys().next_back().copied()
    }

    /// Get best ask price
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.keys().next().copied()
    }

    /// Get mid price
    pub fn mid_price(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some((bid + ask) / dec!(2)),
            _ => None,
        }
    }

    /// Get spread
    pub fn spread(&self) -> Option<Decimal> {
        match (self.best_bid(), self.best_ask()) {
            (Some(bid), Some(ask)) => Some(ask - bid),
            _ => None,
        }
    }

    /// Get total size at or below price (for buys)
    pub fn size_at_price(&self, price: Decimal, is_bid: bool) -> Decimal {
        let book = if is_bid { &self.bids } else { &self.asks };

        book.iter()
            .filter(|(p, _)| if is_bid { **p >= price } else { **p <= price })
            .map(|(_, s)| *s)
            .sum()
    }

    /// Check if orderbook is stale
    pub fn is_stale(&self, max_age_ms: u64) -> bool {
        self.last_update.elapsed().as_millis() > max_age_ms as u128
    }
}

/// Thread-safe orderbook manager for multiple tokens
pub struct OrderbookManager {
    books: RwLock<std::collections::HashMap<String, LocalOrderbook>>,
}

impl OrderbookManager {
    pub fn new() -> Self {
        Self {
            books: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// Get or create orderbook for token
    pub fn get_or_create(&self, asset_id: &str) -> LocalOrderbook {
        {
            let books = self.books.read();
            if let Some(book) = books.get(asset_id) {
                return book.clone();
            }
        }

        let mut books = self.books.write();
        books
            .entry(asset_id.to_string())
            .or_insert_with(|| LocalOrderbook::new(asset_id))
            .clone()
    }

    /// Update orderbook from WebSocket
    pub fn update(&self, asset_id: &str, bids: &[(String, String)], asks: &[(String, String)]) {
        let mut books = self.books.write();
        let book = books
            .entry(asset_id.to_string())
            .or_insert_with(|| LocalOrderbook::new(asset_id));
        book.update_from_snapshot(bids, asks);
    }

    /// Get combined spread for two tokens (Up + Down)
    pub fn get_combined_spread(&self, up_token: &str, down_token: &str) -> Option<CombinedSpread> {
        let books = self.books.read();

        let up_book = books.get(up_token)?;
        let down_book = books.get(down_token)?;

        let up_ask = up_book.best_ask()?;
        let down_ask = down_book.best_ask()?;

        let combined_cost = up_ask + down_ask;
        let spread_pct = (dec!(1) - combined_cost) / combined_cost * dec!(100);

        Some(CombinedSpread {
            up_best_ask: up_ask,
            down_best_ask: down_ask,
            combined_cost,
            spread_pct,
            is_profitable: combined_cost < dec!(1),
        })
    }
}

#[derive(Debug, Clone)]
pub struct CombinedSpread {
    pub up_best_ask: Decimal,
    pub down_best_ask: Decimal,
    pub combined_cost: Decimal,
    pub spread_pct: Decimal,
    pub is_profitable: bool,
}

impl CombinedSpread {
    /// Check if spread meets minimum threshold
    pub fn meets_threshold(&self, min_spread_pct: Decimal) -> bool {
        self.is_profitable && self.spread_pct >= min_spread_pct
    }
}
