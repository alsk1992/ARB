use anyhow::{Context, Result};
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::json;
use std::collections::HashMap;
use std::time::Instant;
use tracing::{debug, info, warn, trace};

use crate::auth::generate_headers;
use crate::config::Config;
use crate::types::{Orderbook, Order, SignedOrder, OrderType, Side};

pub struct ClobClient {
    client: Client,
    proxy_client: Option<Client>,  // Client with residential proxy
    config: Config,
    scrapeless_token: Option<String>,
}

impl ClobClient {
    pub fn new(config: Config) -> Result<Self> {
        // HTTP client (direct)
        let client = Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(std::time::Duration::from_secs(90))
            .timeout(std::time::Duration::from_secs(30))
            .connect_timeout(std::time::Duration::from_secs(10))
            .user_agent("py_clob_client")
            .build()?;

        // Build proxy client if PROXY_URL is set (residential proxy for Cloudflare bypass)
        let proxy_client = if let Ok(proxy_url) = std::env::var("PROXY_URL") {
            match reqwest::Proxy::all(&proxy_url) {
                Ok(proxy) => {
                    match Client::builder()
                        .proxy(proxy)
                        .tcp_nodelay(true)
                        .timeout(std::time::Duration::from_secs(30))
                        .connect_timeout(std::time::Duration::from_secs(10))
                        .user_agent("py_clob_client")
                        .build()
                    {
                        Ok(c) => {
                            info!("Residential proxy enabled: {}", proxy_url.split('@').last().unwrap_or("configured"));
                            Some(c)
                        }
                        Err(e) => {
                            warn!("Failed to build proxy client: {}", e);
                            None
                        }
                    }
                }
                Err(e) => {
                    warn!("Invalid PROXY_URL: {}", e);
                    None
                }
            }
        } else {
            None
        };

        // Check for Scrapeless API token (for Cloudflare bypass)
        let scrapeless_token = std::env::var("SCRAPELESS_TOKEN").ok();
        if scrapeless_token.is_some() {
            info!("Scrapeless proxy also enabled for Cloudflare bypass");
        }

        Ok(Self { client, proxy_client, config, scrapeless_token })
    }

    /// Make a POST request through residential proxy (bypasses Cloudflare)
    async fn post_via_proxy(
        &self,
        path: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<serde_json::Value> {
        let proxy_client = self.proxy_client.as_ref()
            .ok_or_else(|| anyhow::anyhow!("PROXY_URL not configured"))?;

        let url = format!("{}{}", self.config.clob_url, path);

        let mut request = proxy_client.post(&url);
        for (key, value) in headers {
            request = request.header(key, value);
        }

        let response = request
            .body(body.to_string())
            .send()
            .await
            .context("Failed to POST via residential proxy")?;

        let status = response.status();
        let result: serde_json::Value = response.json().await
            .context("Failed to parse response from proxy")?;

        // Check for Cloudflare block
        if status.as_u16() == 403 {
            anyhow::bail!("Cloudflare blocked request (403 via proxy)");
        }

        // Check for Polymarket errors
        if let Some(error) = result.get("error").and_then(|e| e.as_str()) {
            if error.contains("Unauthorized") {
                anyhow::bail!("Polymarket auth error: {}", error);
            }
        }

        if !status.is_success() {
            anyhow::bail!("Proxy POST failed: {} - {:?}", status, result);
        }

        Ok(result)
    }

    /// Make a POST request through Scrapeless proxy (bypasses Cloudflare)
    /// Uses German residential IP which is not blocked
    async fn post_via_scrapeless(
        &self,
        path: &str,
        body: &str,
        headers: &[(String, String)],
    ) -> Result<serde_json::Value> {
        let token = self.scrapeless_token.as_ref()
            .ok_or_else(|| anyhow::anyhow!("SCRAPELESS_TOKEN not configured"))?;

        // Build header object for Scrapeless
        let mut header_obj = serde_json::Map::new();
        header_obj.insert("Content-Type".to_string(), json!("application/json"));
        for (key, value) in headers {
            header_obj.insert(key.clone(), json!(value));
        }

        let url = format!("https://clob.polymarket.com{}", path);

        let scrapeless_request = json!({
            "actor": "unlocker.webunlocker",
            "proxy": {
                "country": "DE"  // German IPs bypass Cloudflare
            },
            "input": {
                "url": url,
                "method": "POST",
                "redirect": false,
                "header": header_obj,
                "body": body
            }
        });

        let response = self.client
            .post("https://api.scrapeless.com/api/v1/unlocker/request")
            .header("x-api-token", token)
            .header("Content-Type", "application/json")
            .json(&scrapeless_request)
            .send()
            .await
            .context("Failed to send request via Scrapeless")?;

        let result: serde_json::Value = response.json().await
            .context("Failed to parse Scrapeless response")?;

        // Check Scrapeless response code
        let code = result.get("code").and_then(|c| c.as_u64()).unwrap_or(0);
        if code != 200 {
            let msg = result.get("message").and_then(|m| m.as_str()).unwrap_or("Unknown error");
            anyhow::bail!("Scrapeless error {}: {}", code, msg);
        }

        // Parse the data field (which contains the actual response)
        let data_str = result.get("data").and_then(|d| d.as_str())
            .ok_or_else(|| anyhow::anyhow!("No data in Scrapeless response"))?;

        // Check if it's an HTML error page
        if data_str.contains("<!DOCTYPE") || data_str.contains("<html") {
            anyhow::bail!("Cloudflare blocked request (HTML response)");
        }

        let data: serde_json::Value = serde_json::from_str(data_str)
            .context("Failed to parse Polymarket response from Scrapeless")?;

        // Check for Polymarket errors
        if let Some(error) = data.get("error").and_then(|e| e.as_str()) {
            anyhow::bail!("Polymarket error: {}", error);
        }

        Ok(data)
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
    /// Priority: 1) Residential proxy, 2) Scrapeless, 3) Lambda proxy, 4) Direct
    pub async fn post_order(&self, order: &Order) -> Result<serde_json::Value> {
        let total_start = Instant::now();

        let path = "/order";
        let body = serde_json::to_string(order)?;
        let json_time = total_start.elapsed();

        let auth_start = Instant::now();
        let headers = generate_headers(&self.config, "POST", path, &body)?;
        let auth_time = auth_start.elapsed();

        // Try residential proxy first (fastest, most reliable)
        if self.proxy_client.is_some() {
            let http_start = Instant::now();
            match self.post_via_proxy(path, &body, &headers).await {
                Ok(result) => {
                    let http_time = http_start.elapsed();
                    let total_time = total_start.elapsed();
                    info!("CLOB POST via residential proxy: json={:?} auth={:?} http={:?} TOTAL={:?}",
                        json_time, auth_time, http_time, total_time);
                    info!("Order posted successfully via proxy: {:?}", result);
                    return Ok(result);
                }
                Err(e) => {
                    warn!("Residential proxy failed: {}, trying Scrapeless...", e);
                }
            }
        }

        // Try Scrapeless proxy second
        if self.scrapeless_token.is_some() {
            let http_start = Instant::now();
            match self.post_via_scrapeless(path, &body, &headers).await {
                Ok(result) => {
                    let http_time = http_start.elapsed();
                    let total_time = total_start.elapsed();
                    info!("CLOB POST via Scrapeless: json={:?} auth={:?} http={:?} TOTAL={:?}",
                        json_time, auth_time, http_time, total_time);
                    info!("Order posted successfully via Scrapeless: {:?}", result);
                    return Ok(result);
                }
                Err(e) => {
                    warn!("Scrapeless proxy failed: {}, trying fallback...", e);
                }
            }
        }

        // Fallback to Lambda proxy if configured
        let (url, use_proxy) = if let Some(lambda_url) = &self.config.lambda_proxy_url {
            (lambda_url.clone(), true)
        } else {
            (format!("{}{}", self.config.clob_url, path), false)
        };

        let http_start = Instant::now();
        let response = if use_proxy {
            // Lambda proxy: wrap request in JSON envelope
            let proxy_request = json!({
                "path": path,
                "method": "POST",
                "headers": headers.iter().cloned().collect::<std::collections::HashMap<String, String>>(),
                "body": body
            });
            info!("Using Lambda proxy for order submission");
            self.client
                .post(&url)
                .json(&proxy_request)
                .send()
                .await
                .context("Failed to post order via Lambda")?
        } else {
            // Direct: add headers individually
            let mut request = self.client.post(&url);
            for (key, value) in headers {
                request = request.header(&key, &value);
            }
            request
                .body(body)
                .send()
                .await
                .context("Failed to post order")?
        };
        let http_time = http_start.elapsed();

        let parse_start = Instant::now();
        let status = response.status();
        let result: serde_json::Value = response.json().await?;
        let parse_time = parse_start.elapsed();

        let total_time = total_start.elapsed();

        info!("CLOB POST timing: json={:?} auth={:?} http={:?} parse={:?} TOTAL={:?} (proxy={})",
            json_time, auth_time, http_time, parse_time, total_time, use_proxy);

        // Handle Lambda response (unwrap body field if present)
        let final_result = if use_proxy {
            if let Some(body_str) = result.get("body").and_then(|b| b.as_str()) {
                serde_json::from_str(body_str).unwrap_or(result)
            } else {
                result
            }
        } else {
            result
        };

        if !status.is_success() && !use_proxy {
            anyhow::bail!("Order failed: {} - {:?}", status, final_result);
        }

        // Check Lambda response status
        if use_proxy {
            if let Some(status_code) = final_result.get("statusCode").and_then(|s| s.as_u64()) {
                if status_code >= 400 {
                    anyhow::bail!("Order failed via Lambda: {:?}", final_result);
                }
            }
        }

        info!("Order posted successfully: {:?}", final_result);
        Ok(final_result)
    }

    /// Post multiple orders in parallel
    /// Priority: 1) Residential proxy, 2) Scrapeless, 3) Lambda proxy, 4) Direct
    pub async fn post_orders(&self, orders: &[Order]) -> Result<Vec<serde_json::Value>> {
        let total_start = Instant::now();

        let path = "/orders";
        let body = serde_json::to_string(orders)?;
        let json_time = total_start.elapsed();

        let auth_start = Instant::now();
        let headers = generate_headers(&self.config, "POST", path, &body)?;
        let auth_time = auth_start.elapsed();

        // Try residential proxy first (fastest, most reliable)
        if self.proxy_client.is_some() {
            let http_start = Instant::now();
            match self.post_via_proxy(path, &body, &headers).await {
                Ok(result) => {
                    let http_time = http_start.elapsed();
                    let total_time = total_start.elapsed();
                    info!("CLOB BATCH POST via residential proxy: {} orders, json={:?} auth={:?} http={:?} TOTAL={:?}",
                        orders.len(), json_time, auth_time, http_time, total_time);
                    info!("Orders posted successfully via proxy: {:?}", result);
                    return match result {
                        serde_json::Value::Array(arr) => Ok(arr),
                        _ => Ok(vec![result]),
                    };
                }
                Err(e) => {
                    warn!("Residential proxy failed for batch: {}, trying Scrapeless...", e);
                }
            }
        }

        // Try Scrapeless proxy second
        if self.scrapeless_token.is_some() {
            let http_start = Instant::now();
            match self.post_via_scrapeless(path, &body, &headers).await {
                Ok(result) => {
                    let http_time = http_start.elapsed();
                    let total_time = total_start.elapsed();
                    info!("CLOB BATCH POST via Scrapeless: {} orders, json={:?} auth={:?} http={:?} TOTAL={:?}",
                        orders.len(), json_time, auth_time, http_time, total_time);
                    info!("Orders posted successfully via Scrapeless: {:?}", result);
                    return match result {
                        serde_json::Value::Array(arr) => Ok(arr),
                        _ => Ok(vec![result]),
                    };
                }
                Err(e) => {
                    warn!("Scrapeless proxy failed for batch orders: {}, trying fallback...", e);
                }
            }
        }

        // Fallback to Lambda proxy if configured
        let (url, use_proxy) = if let Some(lambda_url) = &self.config.lambda_proxy_url {
            (lambda_url.clone(), true)
        } else {
            (format!("{}{}", self.config.clob_url, path), false)
        };

        let http_start = Instant::now();
        let response = if use_proxy {
            // Lambda proxy: wrap request in JSON envelope
            let proxy_request = json!({
                "path": path,
                "method": "POST",
                "headers": headers.iter().cloned().collect::<std::collections::HashMap<String, String>>(),
                "body": body
            });
            info!("Using Lambda proxy for batch order submission");
            self.client
                .post(&url)
                .json(&proxy_request)
                .send()
                .await
                .context("Failed to post orders via Lambda")?
        } else {
            let mut request = self.client.post(&url);
            for (key, value) in headers {
                request = request.header(&key, &value);
            }
            request
                .body(body)
                .send()
                .await
                .context("Failed to post orders")?
        };
        let http_time = http_start.elapsed();

        let parse_start = Instant::now();
        let status = response.status();
        let result: serde_json::Value = response.json().await?;
        let parse_time = parse_start.elapsed();

        let total_time = total_start.elapsed();

        info!("CLOB BATCH POST timing: {} orders, json={:?} auth={:?} http={:?} parse={:?} TOTAL={:?} (proxy={})",
            orders.len(), json_time, auth_time, http_time, parse_time, total_time, use_proxy);

        // Handle Lambda response
        let final_result = if use_proxy {
            if let Some(body_str) = result.get("body").and_then(|b| b.as_str()) {
                serde_json::from_str(body_str).unwrap_or(result)
            } else {
                result
            }
        } else {
            result
        };

        if !status.is_success() && !use_proxy {
            anyhow::bail!("Orders failed: {} - {:?}", status, final_result);
        }

        info!("Orders posted successfully: {:?}", final_result);

        // Return as array
        match final_result {
            serde_json::Value::Array(arr) => Ok(arr),
            _ => Ok(vec![final_result]),
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
