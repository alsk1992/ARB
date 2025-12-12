use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::json;
use tracing::{info, warn};

/// Alert client supporting Discord and Telegram
pub struct AlertClient {
    client: Client,
    discord_url: Option<String>,
    telegram_token: Option<String>,
    telegram_chat_id: Option<String>,
    enabled: bool,
}

impl AlertClient {
    pub fn new(webhook_url: Option<String>) -> Self {
        // Check for Telegram config in env
        let telegram_token = std::env::var("TELEGRAM_BOT_TOKEN").ok();
        let telegram_chat_id = std::env::var("TELEGRAM_CHAT_ID").ok();

        let has_discord = webhook_url.is_some() && !webhook_url.as_ref().unwrap().is_empty();
        let has_telegram = telegram_token.is_some() && telegram_chat_id.is_some();
        let enabled = has_discord || has_telegram;

        if has_telegram {
            info!("Telegram alerts enabled");
        }
        if has_discord {
            info!("Discord alerts enabled");
        }

        Self {
            client: Client::new(),
            discord_url: webhook_url,
            telegram_token,
            telegram_chat_id,
            enabled,
        }
    }

    /// Send alert to all configured channels
    async fn send(&self, content: &str, _color: u32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Try Telegram first (more reliable)
        if let (Some(token), Some(chat_id)) = (&self.telegram_token, &self.telegram_chat_id) {
            let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
            let payload = json!({
                "chat_id": chat_id,
                "text": content,
                "parse_mode": "HTML"
            });

            match self.client.post(&url).json(&payload).send().await {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        warn!("Telegram alert failed: {}", resp.status());
                    }
                }
                Err(e) => {
                    warn!("Telegram alert error: {}", e);
                }
            }
        }

        // Also try Discord if configured
        if let Some(url) = &self.discord_url {
            if !url.is_empty() {
                let payload = json!({
                    "embeds": [{
                        "description": content,
                        "color": _color
                    }]
                });

                if let Err(e) = self.client.post(url).json(&payload).send().await {
                    warn!("Discord alert error: {}", e);
                }
            }
        }

        Ok(())
    }

    /// Alert: Bot started
    pub async fn bot_started(&self, dry_run: bool) {
        let mode = if dry_run { "🔵 DRY RUN" } else { "🟢 LIVE" };
        let msg = format!("🤖 <b>BTC 15m Bot Started</b>\n{}", mode);
        let _ = self.send(&msg, 0x00FF00).await;
    }

    /// Alert: New market found
    pub async fn market_found(&self, title: &str, _end_time: &str) {
        // Extract time from title (e.g., "11:30PM-11:45PM ET")
        let time_part = title.split(" - ").last().unwrap_or("");
        let msg = format!("📊 <b>Market</b>: {}", time_part);
        let _ = self.send(&msg, 0x0099FF).await;
    }

    /// Alert: Orders submitted
    pub async fn orders_submitted(&self, _up_count: usize, _down_count: usize, total_usd: Decimal) {
        let msg = format!(
            "📝 <b>Order</b>: ${:.0}",
            total_usd
        );
        let _ = self.send(&msg, 0x0099FF).await;
    }

    /// Alert: Fill received
    pub async fn fill_received(&self, side: &str, _shares: &str, price: &str) {
        let msg = format!(
            "✅ <b>Fill</b>: {} @ {}¢",
            side, (price.parse::<f64>().unwrap_or(0.0) * 100.0) as i32
        );
        let _ = self.send(&msg, 0x00FF00).await;
    }

    /// Alert: Position update
    pub async fn position_update(
        &self,
        up_shares: Decimal,
        down_shares: Decimal,
        locked_profit: Decimal,
    ) {
        let msg = format!(
            "💰 **Position Update**\nUP: {} shares\nDOWN: {} shares\nLocked Profit: ${}",
            up_shares, down_shares, locked_profit
        );
        let _ = self.send(&msg, 0xFFFF00).await; // Yellow
    }

    /// Alert: Market resolved
    pub async fn market_resolved(&self, _title: &str, profit: Decimal) {
        let (emoji, result) = if profit > Decimal::ZERO {
            ("🟢", "WIN")
        } else if profit < Decimal::ZERO {
            ("🔴", "LOSS")
        } else {
            ("⚪", "FLAT")
        };
        let color = if profit > Decimal::ZERO { 0x00FF00 } else { 0xFF0000 };
        let msg = format!(
            "{} <b>{}</b>: ${:.2}",
            emoji, result, profit
        );
        let _ = self.send(&msg, color).await;
    }

    /// Alert: Error occurred
    pub async fn error(&self, context: &str, error: &str) {
        let msg = format!("❌ **Error**\n{}\n```{}```", context, error);
        let _ = self.send(&msg, 0xFF0000).await; // Red
    }

    /// Alert: Warning
    pub async fn warning(&self, message: &str) {
        let msg = format!("⚠️ **Warning**\n{}", message);
        let _ = self.send(&msg, 0xFFA500).await; // Orange
    }

    /// Alert: Position imbalance
    pub async fn position_imbalance(&self, up_shares: Decimal, down_shares: Decimal) {
        let msg = format!(
            "⚖️ **Position Imbalance**\nUP: {} shares\nDOWN: {} shares\nDifference: {}",
            up_shares,
            down_shares,
            (up_shares - down_shares).abs()
        );
        let _ = self.send(&msg, 0xFFA500).await; // Orange
    }

    /// Alert: Market skipped (no entry)
    pub async fn market_skipped(&self, market_time: &str, reason: &str, btc_change: Decimal) {
        let direction = if btc_change > Decimal::ZERO { "📈" } else { "📉" };
        let msg = format!(
            "⏭️ <b>SKIP</b>: {}\n{} BTC: {:+.3}%\n💡 {}",
            market_time, direction, btc_change, reason
        );
        let _ = self.send(&msg, 0xFFA500).await; // Orange
    }

    /// Alert: Market summary (entry taken)
    pub async fn market_entry(&self, market_time: &str, direction: &str, entry_price: Decimal, shares: Decimal, btc_change: Decimal) {
        let emoji = if direction == "UP" { "🟢" } else { "🔴" };
        let msg = format!(
            "{} <b>ENTRY</b>: {}\n{} {} @ {}¢\nBTC: {:+.3}%",
            emoji, market_time, shares.round_dp(0), direction, (entry_price * Decimal::from(100)).round_dp(1), btc_change
        );
        let _ = self.send(&msg, 0x00FF00).await;
    }
}
