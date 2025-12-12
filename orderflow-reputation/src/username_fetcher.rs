use anyhow::Result;
use regex::Regex;
use sqlx::PgPool;
use tracing::{debug, info, warn};

pub struct UsernameFetcher {
    db: PgPool,
    client: reqwest::Client,
    username_regex: Regex,
}

impl UsernameFetcher {
    pub fn new(db: PgPool) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent("Mozilla/5.0 (compatible; OrderflowBot/1.0)")
            .build()
            .expect("Failed to create HTTP client");

        let username_regex = Regex::new(r#""username":"([^"]+)""#)
            .expect("Failed to compile username regex");

        Self {
            db,
            client,
            username_regex,
        }
    }

    /// Fetch usernames for wallets missing them, prioritizing SMART/WHALE tier
    pub async fn fetch_missing_usernames(&self, limit: usize) -> Result<usize> {
        // Get wallets without usernames, prioritizing high-tier traders
        let wallets: Vec<String> = sqlx::query_scalar(
            r#"
            SELECT wallet_address
            FROM orderflow_wallet_stats
            WHERE polymarket_username IS NULL
            AND trader_tier IN ('SMART', 'WHALE', 'AVERAGE')
            ORDER BY
                CASE trader_tier
                    WHEN 'WHALE' THEN 1
                    WHEN 'SMART' THEN 2
                    WHEN 'AVERAGE' THEN 3
                    ELSE 4
                END,
                reputation_score DESC
            LIMIT $1
            "#
        )
        .bind(limit as i32)
        .fetch_all(&self.db)
        .await?;

        if wallets.is_empty() {
            debug!("No wallets missing usernames");
            return Ok(0);
        }

        info!("🔍 Fetching usernames for {} wallets", wallets.len());

        let mut updated_count = 0;

        for wallet in &wallets {
            match self.fetch_username(wallet).await {
                Ok(Some(username)) => {
                    if let Err(e) = self.save_username(wallet, &username).await {
                        warn!("Failed to save username for {}: {}", wallet, e);
                    } else {
                        updated_count += 1;
                        debug!("✅ {} -> {}", wallet, username);
                    }
                }
                Ok(None) => {
                    debug!("⚠️  No username found for {}", wallet);
                }
                Err(e) => {
                    warn!("Failed to fetch username for {}: {}", wallet, e);
                }
            }

            // Rate limit: 2 seconds between requests
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }

        info!("✅ Fetched {} usernames", updated_count);
        Ok(updated_count)
    }

    /// Fetch username for a single wallet from Polymarket profile page
    async fn fetch_username(&self, wallet: &str) -> Result<Option<String>> {
        let url = format!("https://polymarket.com/profile/{}", wallet);

        let response = self.client
            .get(&url)
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(None);
        }

        let html = response.text().await?;

        // Extract username from embedded JSON data
        if let Some(captures) = self.username_regex.captures(&html) {
            if let Some(username_match) = captures.get(1) {
                let username = username_match.as_str().to_string();
                return Ok(Some(username));
            }
        }

        Ok(None)
    }

    /// Save username to database
    async fn save_username(&self, wallet: &str, username: &str) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE orderflow_wallet_stats
            SET
                polymarket_username = $1,
                username_fetched_at = NOW()
            WHERE wallet_address = $2
            "#
        )
        .bind(username)
        .bind(wallet)
        .execute(&self.db)
        .await?;

        Ok(())
    }
}
