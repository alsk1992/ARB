use anyhow::Result;
use reqwest::Client;
use rust_decimal::Decimal;
use serde_json::json;
use tracing::{error, info};

/// Discord webhook client for alerts
pub struct AlertClient {
    client: Client,
    webhook_url: Option<String>,
    enabled: bool,
}

impl AlertClient {
    pub fn new(webhook_url: Option<String>) -> Self {
        let enabled = webhook_url.is_some();
        Self {
            client: Client::new(),
            webhook_url,
            enabled,
        }
    }

    /// Send a Discord message
    async fn send(&self, content: &str, color: u32) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let url = match &self.webhook_url {
            Some(u) => u,
            None => return Ok(()),
        };

        let payload = json!({
            "embeds": [{
                "description": content,
                "color": color
            }]
        });

        match self.client.post(url).json(&payload).send().await {
            Ok(_) => Ok(()),
            Err(e) => {
                error!("Failed to send Discord alert: {}", e);
                Ok(()) // Don't fail the bot over alerts
            }
        }
    }

    /// Alert: Bot started
    pub async fn bot_started(&self, dry_run: bool) {
        let mode = if dry_run { "DRY RUN" } else { "LIVE" };
        let msg = format!("🤖 **BTC Arb Bot Started**\nMode: {}", mode);
        let _ = self.send(&msg, 0x00FF00).await; // Green
    }

    /// Alert: New market found
    pub async fn market_found(&self, title: &str, end_time: &str) {
        let msg = format!(
            "📊 **New Market Found**\n{}\nEnds: {}",
            title, end_time
        );
        let _ = self.send(&msg, 0x0099FF).await; // Blue
    }

    /// Alert: Orders submitted
    pub async fn orders_submitted(&self, up_count: usize, down_count: usize, total_usd: Decimal) {
        let msg = format!(
            "📝 **Orders Submitted**\nUP: {} orders\nDOWN: {} orders\nTotal: ${}",
            up_count, down_count, total_usd
        );
        let _ = self.send(&msg, 0x0099FF).await; // Blue
    }

    /// Alert: Fill received
    pub async fn fill_received(&self, side: &str, shares: &str, price: &str) {
        let msg = format!(
            "✅ **Fill Received**\n{}: {} shares @ ${}",
            side, shares, price
        );
        let _ = self.send(&msg, 0x00FF00).await; // Green
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
    pub async fn market_resolved(&self, title: &str, profit: Decimal) {
        let emoji = if profit > Decimal::ZERO { "🎉" } else { "😢" };
        let color = if profit > Decimal::ZERO { 0x00FF00 } else { 0xFF0000 };
        let msg = format!(
            "{} **Market Resolved**\n{}\nP&L: ${}",
            emoji, title, profit
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
}
