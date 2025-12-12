use anyhow::Result;
use dashmap::DashMap;
use futures_util::future::join_all;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal_macros::dec;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

use crate::signer::OrderSigner;
use crate::types::{Order, Side};

/// Key for looking up pre-signed orders
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct OrderKey {
    pub token_id: String,
    pub side: Side,
    pub price_cents: u32, // Price in cents (e.g., 45 = $0.45)
    pub size_bucket: u32, // Size bucket for approximate matching
}

impl OrderKey {
    pub fn new(token_id: &str, side: Side, price: Decimal, size: Decimal) -> Self {
        // Round price to cents
        let price_cents = (price * dec!(100)).to_u32().unwrap_or(50);
        // Size bucket (round to nearest 10 shares)
        let size_bucket = (size / dec!(10)).to_u32().unwrap_or(1) * 10;

        Self {
            token_id: token_id.to_string(),
            side,
            price_cents,
            size_bucket,
        }
    }

    pub fn from_params(token_id: &str, side: Side, price_cents: u32, size_bucket: u32) -> Self {
        Self {
            token_id: token_id.to_string(),
            side,
            price_cents,
            size_bucket,
        }
    }
}

/// Pre-signed order cache for instant order submission
///
/// Pre-computes orders at common price points and sizes,
/// allowing sub-millisecond lookup instead of ~20-40ms signing.
pub struct PresignedCache {
    cache: DashMap<OrderKey, CachedOrder>,
    signer: Arc<OrderSigner>,
    cache_ttl: Duration,
    last_warm: Instant,
}

#[derive(Clone)]
pub struct CachedOrder {
    pub order: Order,
    pub created_at: Instant,
}

impl PresignedCache {
    pub fn new(signer: Arc<OrderSigner>) -> Self {
        Self {
            cache: DashMap::new(),
            signer,
            cache_ttl: Duration::from_secs(300), // 5 minute TTL
            last_warm: Instant::now(),
        }
    }

    /// Warm the cache with pre-signed orders for a market
    ///
    /// Generates orders at price points from 35 to 65 cents (step 1 cent)
    /// for multiple size buckets.
    pub async fn warm_cache(
        &self,
        token_id: &str,
        tick_size: Decimal,
        neg_risk: bool,
        base_size: Decimal,
    ) -> Result<usize> {
        let start = Instant::now();
        info!("Warming pre-signed cache for {}...", token_id);

        // Price range: 35 to 65 cents (step 1 cent = 31 prices)
        let prices: Vec<u32> = (35..=65).collect();

        // Size buckets: base_size * [0.5, 1.0, 1.5, 2.0]
        let size_multipliers = vec![dec!(0.5), dec!(1.0), dec!(1.5), dec!(2.0)];

        let mut futures = Vec::new();

        for &price_cents in &prices {
            let price = Decimal::new(price_cents as i64, 2); // Convert cents to decimal

            for &mult in &size_multipliers {
                let size = base_size * mult;
                let size_bucket = (size / dec!(10)).to_u32().unwrap_or(1) * 10;

                // Create order key
                let key = OrderKey::from_params(token_id, Side::Buy, price_cents, size_bucket);

                // Only create if not already cached and valid
                if let Some(cached) = self.cache.get(&key) {
                    if cached.created_at.elapsed() < self.cache_ttl {
                        continue; // Already cached and valid
                    }
                }

                // Create order future
                let signer = self.signer.clone();
                let token = token_id.to_string();
                let key_clone = key.clone();

                futures.push(async move {
                    match signer.create_order(
                        &token, price, size, Side::Buy, tick_size, neg_risk
                    ).await {
                        Ok(order) => Some((key_clone, CachedOrder {
                            order,
                            created_at: Instant::now(),
                        })),
                        Err(e) => {
                            warn!("Failed to pre-sign order: {}", e);
                            None
                        }
                    }
                });
            }
        }

        let total_orders = futures.len();
        info!("Pre-signing {} orders in parallel...", total_orders);

        // Execute all signing in parallel
        let results = join_all(futures).await;

        // Insert into cache
        let mut cached_count = 0;
        for result in results.into_iter().flatten() {
            self.cache.insert(result.0, result.1);
            cached_count += 1;
        }

        let elapsed = start.elapsed();
        info!(
            "Cache warmed: {} orders in {:?} ({:.1}ms per order)",
            cached_count,
            elapsed,
            elapsed.as_millis() as f64 / cached_count.max(1) as f64
        );

        Ok(cached_count)
    }

    /// Get a pre-signed order from cache (sub-millisecond)
    pub fn get_order(&self, token_id: &str, side: Side, price: Decimal, size: Decimal) -> Option<Order> {
        let start = Instant::now();
        let key = OrderKey::new(token_id, side, price, size);

        let result = self.cache.get(&key).and_then(|cached| {
            // Check TTL
            if cached.created_at.elapsed() < self.cache_ttl {
                Some(cached.order.clone())
            } else {
                None // Expired
            }
        });

        let lookup_time = start.elapsed();
        debug!("Cache lookup: {:?} (hit: {})", lookup_time, result.is_some());

        result
    }

    /// Get order or sign fresh (fallback)
    pub async fn get_or_sign(
        &self,
        token_id: &str,
        price: Decimal,
        size: Decimal,
        side: Side,
        tick_size: Decimal,
        neg_risk: bool,
    ) -> Result<Order> {
        // Try cache first (fast path)
        if let Some(order) = self.get_order(token_id, side, price, size) {
            debug!("Cache HIT for {} @ {}", token_id, price);
            return Ok(order);
        }

        // Sign fresh (slow path)
        debug!("Cache MISS for {} @ {}, signing fresh", token_id, price);
        self.signer.create_order(token_id, price, size, side, tick_size, neg_risk).await
    }

    /// Clear expired entries
    pub fn cleanup(&self) {
        let before = self.cache.len();
        self.cache.retain(|_, v| v.created_at.elapsed() < self.cache_ttl);
        let after = self.cache.len();
        if before != after {
            info!("Cache cleanup: removed {} expired orders", before - after);
        }
    }

    /// Get cache stats
    pub fn stats(&self) -> CacheStats {
        let total = self.cache.len();
        let valid = self.cache.iter()
            .filter(|e| e.value().created_at.elapsed() < self.cache_ttl)
            .count();

        CacheStats {
            total_entries: total,
            valid_entries: valid,
            expired_entries: total - valid,
            ttl_seconds: self.cache_ttl.as_secs(),
        }
    }
}

#[derive(Debug)]
pub struct CacheStats {
    pub total_entries: usize,
    pub valid_entries: usize,
    pub expired_entries: usize,
    pub ttl_seconds: u64,
}

impl std::fmt::Display for CacheStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cache: {} valid, {} expired (TTL: {}s)",
            self.valid_entries, self.expired_entries, self.ttl_seconds
        )
    }
}
