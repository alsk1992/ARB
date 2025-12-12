use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info, trace};

use crate::auth::generate_headers;
use crate::config::Config;
use crate::types::{Orderbook, Order, SignedOrder, OrderType, Side};

pub struct ClobClient {
    client: Client,
    config: Config,
}

impl ClobClient {
    pub fn new(config: Config) -> Result<Self> {
        // Optimized HTTP client for low latency
        let client = Client::builder()
            .tcp_nodelay(true)                     // Disable Nagle's algorithm
            .pool_max_idle_per_host(10)            // Keep connections warm
            .pool_idle_timeout(std::time::Duration::from_secs(90)) // Keep alive longer
            .timeout(std::time::Duration::from_secs(10))  // Overall timeout
            .connect_timeout(std::time::Duration::from_secs(5)) // Fast connect timeout
            .build()?;

        Ok(Self { client, config })
    }

    /// Get orderbook for a token
    pub async fn get_orderbook(&self, token_id: &str) -> Result<Orderbook> {
        let start = Instant::now();

        let path = format!("/book?token_id={}", token_id);
        let url = format!("{}{}", self.config.clob_url, path);

        let http_start = Instant::now();
        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch orderbook")?;
        let http_time = http_start.elapsed();

        let parse_start = Instant::now();
        let orderbook: Orderbook = response
            .json()
            .await
            .context("Failed to parse orderbook")?;
        let parse_time = parse_start.elapsed();

        trace!("Orderbook fetch: http={:?} parse={:?} TOTAL={:?}",
            http_time, parse_time, start.elapsed());

        Ok(orderbook)
    }

    /// Get orderbooks for multiple tokens
    pub async fn get_orderbooks(&self, token_ids: &[&str]) -> Result<Vec<Orderbook>> {
        let ids = token_ids.join(",");
        let path = format!("/books?token_ids={}", ids);
        let url = format!("{}{}", self.config.clob_url, path);

        let response = self.client
            .get(&url)
            .send()
            .await
            .context("Failed to fetch orderbooks")?;

        let orderbooks: Vec<Orderbook> = response
            .json()
            .await
            .context("Failed to parse orderbooks")?;

        Ok(orderbooks)
    }

    /// Get best prices for a token
    pub async fn get_price(&self, token_id: &str) -> Result<(Decimal, Decimal)> {
        let path = format!("/price?token_id={}&side=BUY", token_id);
        let url = format!("{}{}", self.config.clob_url, path);

        let response: serde_json::Value = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        let price: Decimal = response["price"]
            .as_str()
            .unwrap_or("0")
            .parse()
            .unwrap_or_default();

        Ok((price, Decimal::ZERO)) // TODO: Get ask price too
    }

    /// Post a signed order to CLOB
    pub async fn post_order(&self, order: &Order) -> Result<serde_json::Value> {
        let total_start = Instant::now();

        let path = "/order";
        let body = serde_json::to_string(order)?;
        let json_time = total_start.elapsed();

        let auth_start = Instant::now();
        let headers = generate_headers(&self.config, "POST", path, &body)?;
        let auth_time = auth_start.elapsed();

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = self.client.post(&url);
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        let http_start = Instant::now();
        let response = request
            .body(body)
            .send()
            .await
            .context("Failed to post order")?;
        let http_time = http_start.elapsed();

        let parse_start = Instant::now();
        let status = response.status();
        let result: serde_json::Value = response.json().await?;
        let parse_time = parse_start.elapsed();

        let total_time = total_start.elapsed();

        info!("CLOB POST timing: json={:?} auth={:?} http={:?} parse={:?} TOTAL={:?}",
            json_time, auth_time, http_time, parse_time, total_time);

        if !status.is_success() {
            anyhow::bail!("Order failed: {} - {:?}", status, result);
        }

        info!("Order posted successfully: {:?}", result);
        Ok(result)
    }

    /// Post multiple orders in parallel
    pub async fn post_orders(&self, orders: &[Order]) -> Result<Vec<serde_json::Value>> {
        let total_start = Instant::now();

        let path = "/orders";
        let body = serde_json::to_string(orders)?;
        let json_time = total_start.elapsed();

        let auth_start = Instant::now();
        let headers = generate_headers(&self.config, "POST", path, &body)?;
        let auth_time = auth_start.elapsed();

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = self.client.post(&url);
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        let http_start = Instant::now();
        let response = request
            .body(body)
            .send()
            .await
            .context("Failed to post orders")?;
        let http_time = http_start.elapsed();

        let parse_start = Instant::now();
        let status = response.status();
        let result: serde_json::Value = response.json().await?;
        let parse_time = parse_start.elapsed();

        let total_time = total_start.elapsed();

        info!("CLOB BATCH POST timing: {} orders, json={:?} auth={:?} http={:?} parse={:?} TOTAL={:?}",
            orders.len(), json_time, auth_time, http_time, parse_time, total_time);

        if !status.is_success() {
            anyhow::bail!("Orders failed: {} - {:?}", status, result);
        }

        info!("Orders posted successfully: {:?}", result);

        // Return as array
        match result {
            serde_json::Value::Array(arr) => Ok(arr),
            _ => Ok(vec![result]),
        }
    }

    /// Cancel an order
    pub async fn cancel_order(&self, order_id: &str) -> Result<()> {
        let path = "/order";
        let body = json!({ "orderID": order_id }).to_string();

        let headers = generate_headers(&self.config, "DELETE", path, &body)?;

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = self.client.delete(&url);
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        let response = request
            .body(body)
            .send()
            .await
            .context("Failed to cancel order")?;

        if !response.status().is_success() {
            anyhow::bail!("Cancel failed: {}", response.status());
        }

        debug!("Order {} cancelled", order_id);
        Ok(())
    }

    /// Cancel all orders for a market
    pub async fn cancel_market_orders(&self, condition_id: &str) -> Result<()> {
        let path = "/cancel-market-orders";
        let body = json!({ "market": condition_id }).to_string();

        let headers = generate_headers(&self.config, "DELETE", path, &body)?;

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = self.client.delete(&url);
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        let response = request
            .body(body)
            .send()
            .await
            .context("Failed to cancel market orders")?;

        if !response.status().is_success() {
            anyhow::bail!("Cancel all failed: {}", response.status());
        }

        info!("All orders cancelled for market {}", condition_id);
        Ok(())
    }

    /// Get open orders
    pub async fn get_open_orders(&self) -> Result<Vec<serde_json::Value>> {
        let path = "/data/orders";
        let headers = generate_headers(&self.config, "GET", path, "")?;

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = self.client.get(&url);
        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        let response = request
            .send()
            .await
            .context("Failed to get open orders")?;

        let orders: Vec<serde_json::Value> = response.json().await?;
        Ok(orders)
    }

    /// Get tick size for a market
    pub async fn get_tick_size(&self, token_id: &str) -> Result<Decimal> {
        let path = format!("/tick-size?token_id={}", token_id);
        let url = format!("{}{}", self.config.clob_url, path);

        let response: serde_json::Value = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        let tick_size: Decimal = response
            .as_str()
            .unwrap_or("0.01")
            .parse()
            .unwrap_or(Decimal::from_str_exact("0.01").unwrap());

        Ok(tick_size)
    }

    /// Check if market is neg risk
    pub async fn get_neg_risk(&self, token_id: &str) -> Result<bool> {
        let path = format!("/neg-risk?token_id={}", token_id);
        let url = format!("{}{}", self.config.clob_url, path);

        let response: serde_json::Value = self.client
            .get(&url)
            .send()
            .await?
            .json()
            .await?;

        Ok(response.as_bool().unwrap_or(false))
    }
}
