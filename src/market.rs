use anyhow::{Context, Result};
use chrono::{DateTime, Utc, TimeZone};
use reqwest::Client;
use rust_decimal::Decimal;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::types::{BtcMarket, Event, Market};

/// Find active BTC 15-minute markets
pub struct MarketMonitor {
    client: Client,
    config: Config,
}

impl MarketMonitor {
    pub fn new(config: Config) -> Self {
        let client = Client::builder()
            .tcp_nodelay(true)
            .build()
            .unwrap();

        Self { client, config }
    }

    /// Find the current active BTC 15-min market
    pub async fn find_active_btc_market(&self) -> Result<Option<BtcMarket>> {
        // Get current time and calculate current 15-min window timestamp
        let now = Utc::now();
        let current_ts = now.timestamp();

        // Round down to nearest 15 minutes
        let window_start = (current_ts / 900) * 900;

        // Try current and next windows
        for offset in [0, 900] {
            let ts = window_start + offset;
            let slug = format!("btc-updown-15m-{}", ts);

            if let Some(market) = self.fetch_market_by_slug(&slug).await? {
                if market.end_time > now {
                    info!("Found active market: {} (ends at {})", market.title, market.end_time);
                    return Ok(Some(market));
                }
            }
        }

        // Fallback: search for any active BTC 15-min market
        self.search_active_btc_markets().await
    }

    /// Fetch market by event slug
    async fn fetch_market_by_slug(&self, slug: &str) -> Result<Option<BtcMarket>> {
        let url = format!("{}/events?slug={}", self.config.gamma_url, slug);

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch event")?;

        let events: Vec<Event> = response
            .json()
            .await
            .context("Failed to parse events")?;

        if events.is_empty() {
            return Ok(None);
        }

        let event = &events[0];

        if event.markets.is_empty() {
            return Ok(None);
        }

        self.parse_btc_market(event).await
    }

    /// Search for active BTC 15-min markets
    async fn search_active_btc_markets(&self) -> Result<Option<BtcMarket>> {
        let url = format!(
            "{}/events?slug_contains=btc-updown-15m&active=true&closed=false&limit=5",
            self.config.gamma_url
        );

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to search markets")?;

        let events: Vec<Event> = response
            .json()
            .await
            .unwrap_or_default();

        for event in events {
            if let Ok(Some(market)) = self.parse_btc_market(&event).await {
                if market.end_time > Utc::now() {
                    return Ok(Some(market));
                }
            }
        }

        Ok(None)
    }

    /// Parse event into BtcMarket struct
    async fn parse_btc_market(&self, event: &Event) -> Result<Option<BtcMarket>> {
        if event.markets.is_empty() {
            return Ok(None);
        }

        let market = &event.markets[0];

        // Extract token IDs for Up and Down
        let mut up_token_id = String::new();
        let mut down_token_id = String::new();

        for token in &market.tokens {
            match token.outcome.to_lowercase().as_str() {
                "up" | "yes" => up_token_id = token.token_id.clone(),
                "down" | "no" => down_token_id = token.token_id.clone(),
                _ => {}
            }
        }

        if up_token_id.is_empty() || down_token_id.is_empty() {
            // Try parsing from clobTokenIds if tokens array is empty
            if let Some(prices_str) = &market.outcome_prices {
                // Parse outcome prices to determine which token is which
                debug!("Parsing from outcome_prices: {}", prices_str);
            }
            warn!("Could not find Up/Down token IDs for market: {}", event.slug);
            return Ok(None);
        }

        // Parse end time from slug (btc-updown-15m-{timestamp})
        let end_time = if let Some(ts_str) = event.slug.strip_prefix("btc-updown-15m-") {
            if let Ok(ts) = ts_str.parse::<i64>() {
                // Add 15 minutes to get end time
                Utc.timestamp_opt(ts + 900, 0).single().unwrap_or(Utc::now())
            } else {
                Utc::now()
            }
        } else {
            Utc::now()
        };

        // Get tick size
        let tick_size = market
            .tick_size
            .as_ref()
            .and_then(|t| t.parse().ok())
            .unwrap_or(Decimal::from_str_exact("0.01").unwrap());

        let neg_risk = market.neg_risk.unwrap_or(false);

        Ok(Some(BtcMarket {
            event_slug: event.slug.clone(),
            condition_id: market.condition_id.clone(),
            title: market.question.clone(),
            up_token_id,
            down_token_id,
            end_time,
            tick_size,
            neg_risk,
        }))
    }

    /// Get time remaining until market resolution
    pub fn time_until_resolution(market: &BtcMarket) -> chrono::Duration {
        market.end_time - Utc::now()
    }

    /// Check if it's too late to enter a market (less than 2 minutes remaining)
    pub fn is_too_late(market: &BtcMarket) -> bool {
        Self::time_until_resolution(market) < chrono::Duration::minutes(2)
    }

    /// Wait for next market window
    pub async fn wait_for_next_market(&self) -> BtcMarket {
        loop {
            match self.find_active_btc_market().await {
                Ok(Some(market)) => {
                    if !Self::is_too_late(&market) {
                        return market;
                    }
                    info!("Market too close to resolution, waiting for next...");
                }
                Ok(None) => {
                    debug!("No active market found, waiting...");
                }
                Err(e) => {
                    warn!("Error finding market: {}", e);
                }
            }

            // Wait 30 seconds before checking again
            tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        }
    }
}
