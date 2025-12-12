//! Latency Benchmark Tool
//!
//! Comprehensive latency testing for the arbitrage bot.
//! Run with: cargo run --bin latency_bench --release

use anyhow::Result;
use reqwest::Client;
use std::time::{Duration, Instant};

const CLOB_URL: &str = "https://clob.polymarket.com";
const TEST_TOKEN_ID: &str = "21742633143463906290569050155826241533067272736897614950488156847949938836455";

#[tokio::main]
async fn main() -> Result<()> {
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║           LATENCY BENCHMARK - Arbitrage Bot               ║");
    println!("╚═══════════════════════════════════════════════════════════╝\n");

    // Create optimized HTTP client (same as production)
    let client = Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(5))
        .build()?;

    // Warm up connection pool
    println!("Warming up connection pool...");
    let _ = client.get(CLOB_URL).send().await;
    println!("Connection pool warmed.\n");

    // Test 1: Orderbook fetch latency
    println!("═══ TEST 1: Orderbook Fetch (50 requests) ═══");
    let mut fetch_times: Vec<Duration> = Vec::new();

    for i in 1..=50 {
        let start = Instant::now();
        let url = format!("{}/book?token_id={}", CLOB_URL, TEST_TOKEN_ID);
        let resp = client.get(&url).send().await?;
        let _ = resp.bytes().await?;
        let elapsed = start.elapsed();
        fetch_times.push(elapsed);

        if i % 10 == 0 {
            print!(".");
            use std::io::Write;
            std::io::stdout().flush().ok();
        }
    }
    println!();

    fetch_times.sort();
    let p50 = fetch_times[fetch_times.len() / 2];
    let p95 = fetch_times[(fetch_times.len() as f64 * 0.95) as usize];
    let p99 = fetch_times[(fetch_times.len() as f64 * 0.99) as usize];
    let avg: Duration = fetch_times.iter().sum::<Duration>() / fetch_times.len() as u32;
    let min = fetch_times.first().unwrap();
    let max = fetch_times.last().unwrap();

    println!("Orderbook Fetch Results:");
    println!("  Min:  {:>8.2?}", min);
    println!("  Avg:  {:>8.2?}", avg);
    println!("  P50:  {:>8.2?}", p50);
    println!("  P95:  {:>8.2?}", p95);
    println!("  P99:  {:>8.2?}", p99);
    println!("  Max:  {:>8.2?}", max);

    let target = Duration::from_millis(100);
    if p95 < target {
        println!("  ✅ PASS: P95 < 100ms target");
    } else {
        println!("  ⚠️  WARN: P95 > 100ms target");
    }
    println!();

    // Test 2: POST round-trip (will fail auth but measures network)
    println!("═══ TEST 2: POST Round-trip (10 requests) ═══");
    let mut post_times: Vec<Duration> = Vec::new();

    for _ in 1..=10 {
        let start = Instant::now();
        let url = format!("{}/order", CLOB_URL);
        let _ = client
            .post(&url)
            .header("Content-Type", "application/json")
            .body(r#"{"test": true}"#)
            .send()
            .await;
        let elapsed = start.elapsed();
        post_times.push(elapsed);
        print!(".");
        use std::io::Write;
        std::io::stdout().flush().ok();
    }
    println!();

    post_times.sort();
    let post_p50 = post_times[post_times.len() / 2];
    let post_avg: Duration = post_times.iter().sum::<Duration>() / post_times.len() as u32;
    let post_min = post_times.first().unwrap();
    let post_max = post_times.last().unwrap();

    println!("POST Round-trip Results:");
    println!("  Min:  {:>8.2?}", post_min);
    println!("  Avg:  {:>8.2?}", post_avg);
    println!("  P50:  {:>8.2?}", post_p50);
    println!("  Max:  {:>8.2?}", post_max);

    let post_target = Duration::from_millis(150);
    if post_p50 < post_target {
        println!("  ✅ PASS: P50 < 150ms target");
    } else {
        println!("  ⚠️  WARN: P50 > 150ms target");
    }
    println!();

    // Test 3: Signing benchmark (CPU-only)
    println!("═══ TEST 3: Order Signing (simulated) ═══");
    let sign_start = Instant::now();
    let iterations = 1000;

    // Simulate HMAC-SHA256 signing workload
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    for i in 0..iterations {
        let mut hasher = DefaultHasher::new();
        format!("order_{}_{}", i, TEST_TOKEN_ID).hash(&mut hasher);
        let _ = hasher.finish();
    }

    let sign_elapsed = sign_start.elapsed();
    let per_sign = sign_elapsed / iterations;

    println!("Signing Simulation Results:");
    println!("  {} iterations in {:?}", iterations, sign_elapsed);
    println!("  Per-sign avg: {:?}", per_sign);
    println!("  Note: Real EIP-712 signing is ~0.1ms/order (from logs)");
    println!();

    // Summary
    println!("╔═══════════════════════════════════════════════════════════╗");
    println!("║                    SUMMARY                                ║");
    println!("╠═══════════════════════════════════════════════════════════╣");
    println!("║ Orderbook Fetch P95:  {:>8.2?} (target: <100ms)         ║", p95);
    println!("║ POST Round-trip P50:  {:>8.2?} (target: <150ms)         ║", post_p50);
    println!("║ Order Signing:        ~0.1ms/order (from production)     ║");
    println!("╠═══════════════════════════════════════════════════════════╣");

    let total_estimate = p95 + post_p50 + Duration::from_micros(100);
    println!("║ Estimated End-to-End: {:>8.2?}                          ║", total_estimate);

    if total_estimate < Duration::from_millis(300) {
        println!("║ ✅ OVERALL: Speed is SUFFICIENT for arbitrage            ║");
    } else {
        println!("║ ⚠️  OVERALL: Speed may need optimization                  ║");
    }

    println!("╚═══════════════════════════════════════════════════════════╝");

    Ok(())
}
