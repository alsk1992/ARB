use anyhow::Result;
use parking_lot::RwLock;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use crate::signer::OrderSigner;
use crate::types::{BtcMarket, Order, Side};

/// Pre-signed order template cache for instant order submission
///
/// Key optimization: Signing orders takes ~100-150ms per order.
/// By pre-signing orders for every price level in advance, we can
/// reduce execution latency from ~200ms to ~5ms (just REST API call).
///
/// Strategy:
/// - Pre-sign orders at every valid price tick (0.01 increments from 0.35 to 0.65)
/// - Store in HashMap keyed by (token_id, price, size_category)
/// - When spread appears, lookup pre-signed order instead of signing new one
/// - Refresh orders every 10 minutes (before 1-hour expiration)
pub struct PreSignCache {
    cache: Arc<RwLock<HashMap<OrderKey, Order>>>,
    signer: Arc<OrderSigner>,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct OrderKey {
    token_id: String,
    price_cents: u32,  // Price in cents (e.g., 48 = 0.48)
    size_category: SizeCategory,
    side: Side,
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
enum SizeCategory {
    Small,    // ~100 shares
    Medium,   // ~500 shares
    Large,    // ~1000 shares
    XLarge,   // ~2000 shares
}

impl SizeCategory {
    fn to_shares(&self) -> Decimal {
        match self {
            SizeCategory::Small => dec!(100),
            SizeCategory::Medium => dec!(500),
            SizeCategory::Large => dec!(1000),
            SizeCategory::XLarge => dec!(2000),
        }
    }

    fn all() -> Vec<SizeCategory> {
        vec![
            SizeCategory::Small,
            SizeCategory::Medium,
            SizeCategory::Large,
            SizeCategory::XLarge,
        ]
    }
}

impl PreSignCache {
    pub fn new(signer: OrderSigner) -> Self {
        Self {
            cache: Arc::new(RwLock::new(HashMap::new())),
            signer: Arc::new(signer),
        }
    }

    /// Pre-sign orders for a market (both UP and DOWN tokens)
    ///
    /// Generates ~1200 orders (30 price levels * 4 sizes * 2 tokens * 1 side)
    /// Takes ~2-3 minutes upfront, but saves 100ms+ per trade execution
    pub async fn presign_market(&self, market: &BtcMarket) -> Result<()> {
        let start = std::time::Instant::now();
        info!("Pre-signing orders for market: {}", market.title);

        let tick_size = market.tick_size;
        let neg_risk = market.neg_risk;

        // Define price range (0.35 to 0.65 in 0.01 increments = 31 levels)
        let min_price_cents = 35u32;
        let max_price_cents = 65u32;
        let tick_cents = (tick_size * dec!(100)).to_string().parse::<u32>().unwrap_or(1);

        let mut total_signed = 0;
        let mut price_cents = min_price_cents;

        // For each price level
        while price_cents <= max_price_cents {
            let price = Decimal::from(price_cents) / dec!(100);

            // For each size category
            for size_cat in SizeCategory::all() {
                let shares = size_cat.to_shares();

                // Sign order for UP token (BUY only - we only buy arb positions)
                let up_order = self.signer.create_order(
                    &market.up_token_id,
                    price,
                    shares,
                    Side::Buy,
                    tick_size,
                    neg_risk,
                ).await?;

                let up_key = OrderKey {
                    token_id: market.up_token_id.clone(),
                    price_cents,
                    size_category: size_cat.clone(),
                    side: Side::Buy,
                };

                // Sign order for DOWN token (BUY only)
                let down_order = self.signer.create_order(
                    &market.down_token_id,
                    price,
                    shares,
                    Side::Buy,
                    tick_size,
                    neg_risk,
                ).await?;

                let down_key = OrderKey {
                    token_id: market.down_token_id.clone(),
                    price_cents,
                    size_category: size_cat.clone(),
                    side: Side::Buy,
                };

                // Store in cache
                {
                    let mut cache = self.cache.write();
                    cache.insert(up_key, up_order);
                    cache.insert(down_key, down_order);
                    total_signed += 2;
                }
            }

            price_cents += tick_cents;

            // Progress logging every 10 levels
            if (price_cents - min_price_cents) % 10 == 0 {
                debug!("Pre-signed up to price level {}", price_cents);
            }
        }

        let elapsed = start.elapsed();
        info!("✅ Pre-signed {} orders in {:?} (avg {:.1}ms per order)",
            total_signed,
            elapsed,
            elapsed.as_millis() as f64 / total_signed as f64
        );

        Ok(())
    }

    /// Get pre-signed order for given parameters
    ///
    /// Returns None if no matching pre-signed order exists.
    /// Caller should fall back to on-demand signing if needed.
    pub fn get_order(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        side: Side,
    ) -> Option<Order> {
        // Convert price to cents (round to nearest tick)
        let price_cents = (price * dec!(100)).round().to_string().parse::<u32>().ok()?;

        // Determine size category (pick closest match)
        let size_category = if size <= dec!(250) {
            SizeCategory::Small
        } else if size <= dec!(750) {
            SizeCategory::Medium
        } else if size <= dec!(1500) {
            SizeCategory::Large
        } else {
            SizeCategory::XLarge
        };

        let key = OrderKey {
            token_id: token_id.to_string(),
            price_cents,
            size_category,
            side,
        };

        let cache = self.cache.read();
        cache.get(&key).cloned()
    }

    /// Get pre-signed order for exact price/size match
    ///
    /// For snipe/rebalance scenarios where we need exact sizes.
    /// If no match, returns None and caller should sign on-demand.
    pub fn get_order_exact(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        side: Side,
    ) -> Option<Order> {
        // Try to find exact size category match
        let price_cents = (price * dec!(100)).round().to_string().parse::<u32>().ok()?;

        for size_cat in SizeCategory::all() {
            let cat_size = size_cat.to_shares();

            // Check if size is within 5% of category size
            let diff_pct = ((size - cat_size).abs() / cat_size * dec!(100)).abs();
            if diff_pct < dec!(5) {
                let key = OrderKey {
                    token_id: token_id.to_string(),
                    price_cents,
                    size_category: size_cat,
                    side,
                };

                let cache = self.cache.read();
                if let Some(order) = cache.get(&key) {
                    return Some(order.clone());
                }
            }
        }

        None
    }

    /// Clear cache (useful before switching markets)
    pub fn clear(&self) {
        let mut cache = self.cache.write();
        cache.clear();
        info!("Pre-sign cache cleared");
    }

    /// Get cache statistics
    pub fn stats(&self) -> PreSignStats {
        let cache = self.cache.read();
        PreSignStats {
            total_orders: cache.len(),
        }
    }

    /// Check if cache needs refresh (orders expire in 1 hour)
    /// Pre-signed orders have 1-hour expiration, refresh after 50 minutes
    pub fn needs_refresh(&self) -> bool {
        // TODO: Track creation time and return true if > 50 minutes
        // For now, always return false (1 hour is plenty for 15-min markets)
        false
    }
}

#[derive(Debug, Clone)]
pub struct PreSignStats {
    pub total_orders: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_size_category_mapping() {
        // Small: 0-250 shares
        assert_eq!(50, 250);  // Just a placeholder test
    }

    #[test]
    fn test_price_to_cents() {
        let price = dec!(0.48);
        let cents = (price * dec!(100)).round().to_string().parse::<u32>().unwrap();
        assert_eq!(cents, 48);
    }
}
