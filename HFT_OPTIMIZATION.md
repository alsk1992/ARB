# HFT Optimization Path for BTC Arb Bot

## Problem Statement

Analysis of live data shows profitable spreads exist but last only **4-13 milliseconds**:
- Spread appears: `22:47:11.840` (1.01%)
- Best spread: `22:47:11.844` (3.09% - combined cost $0.97)
- Spread gone: `22:47:11.853` (back to negative)
- **Total window: 13ms**

Current bot execution time: **~200-500ms**
- Result: **Missing 100% of profitable opportunities**

## Optimization Roadmap

### ✅ **Phase 1: Baseline (Completed)**
- **Latency:** ~200-500ms
- **Bottleneck:** Signing (150ms) + REST API (50ms)
- **Status:** Data collection working, spreads detected but too slow

### ✅ **Phase 2: Pre-Signed Orders (Completed)**
- **Target:** 200ms → 20ms
- **Implementation:** Pre-sign orders for all price levels upfront
- **Code:** `src/presign.rs`
- **How it works:**
  - Pre-sign ~1200 orders per market (31 price levels × 4 sizes × 2 tokens)
  - Takes 2-3 minutes upfront
  - Instant lookup from HashMap cache
  - Zero signing latency during execution
- **Result:** ~150ms saved (signing eliminated)
- **Current latency:** ~20ms (REST API only)

**Benchmark:**
```
Standard signing: 147ms
⚡ Using pre-signed orders (lookup: 145μs)
🎯 Snipe successful! Total latency: 21ms
```

### 🚧 **Phase 3: WebSocket Order Submission (In Progress)**
- **Target:** 20ms → 5ms
- **Bottleneck:** HTTP/TLS handshake overhead
- **Implementation:** `src/ws_clob.rs` (skeleton created)
- **Challenge:** Polymarket doesn't document WebSocket order submission
- **How it would work:**
  - Persistent WebSocket connection to CLOB
  - Submit orders as binary frames (no HTTP overhead)
  - Eliminate TLS handshake on every request
- **Blockers:**
  - Need to reverse-engineer Polymarket's private WS protocol
  - Alternative: Use their official REST API optimally
- **Expected gain:** ~15ms saved

### 📋 **Phase 4: Co-location (Future)**
- **Target:** 5ms → 1ms
- **Implementation:** Deploy on AWS in same datacenter as Polymarket
- **Requirements:**
  - Dedicated server ($500-2000/mo)
  - Same AWS region as Polymarket CLOB
  - Optimized network stack (kernel tuning)
- **Expected gain:** ~4ms saved

## Current Performance Summary

| Phase | Latency | Signing | Network | Status |
|-------|---------|---------|---------|--------|
| Baseline | 200ms | 150ms | 50ms | ✅ Complete |
| Pre-signing | 20ms | 0ms | 20ms | ✅ **ACTIVE** |
| WebSocket | 5ms | 0ms | 5ms | 🚧 Blocked on protocol |
| Co-location | 1ms | 0ms | 1ms | 📋 Planned |

## Realistic Assessment

### Can We Compete?

**With Phase 2 (20ms):**
- ❌ Cannot capture 13ms spreads
- ❌ Still too slow for HFT market
- ✅ Good for learning and data collection

**With Phase 3 (5ms):**
- ⚠️ Might catch occasional spreads
- ❌ Still slower than top bots (<3ms)
- 📊 Would need luck + perfect timing

**With Phase 4 (1ms):**
- ✅ Competitive with mid-tier HFT bots
- ⚠️ Top bots still faster (<500μs)
- 💰 Requires significant infrastructure cost

### Alternative Strategies

Instead of chasing milliseconds, consider:

1. **Different markets:** Find less competitive arb opportunities
2. **Slower strategies:** Ladder orders that capture gradual fills
3. **ML prediction:** Predict when spreads will appear (5-10 sec head start)
4. **MEV on-chain:** Front-run settlement transactions
5. **Market making:** Provide liquidity instead of arb

## Phase 3 Implementation Notes

### WebSocket Order Submission Challenges

Polymarket's CLOB API is documented for REST only:
- `POST https://clob.polymarket.com/order` - Standard endpoint
- WebSocket endpoint: **Unknown/Undocumented**

**Two paths forward:**

#### Option A: Reverse Engineer (Advanced)
1. Capture network traffic from official Polymarket frontend
2. Identify WebSocket order submission protocol
3. Replicate in bot
4. **Risk:** Protocol changes, ToS violation

#### Option B: Optimize REST (Practical)
1. HTTP/2 multiplexing
2. Connection pooling (already implemented)
3. TCP_NODELAY (already enabled)
4. Keep-alive connections
5. **Realistic gain:** 20ms → 10ms

### Current Code Status

- ✅ `ws_clob.rs` - Skeleton implementation
- ⚠️ WebSocket URL is placeholder
- ⚠️ Message format is guessed
- ❌ Not functional without protocol details

## Data Analysis Results

From `snapshots_20251211_224224.jsonl`:

- **Total snapshots:** 14,847
- **"Profitable" moments (combined ≤ $1.00):** 1,035
- **Actually profitable (>0.5% spread):** 6
- **Longest spread duration:** 13ms
- **Conclusion:** Even with 1ms latency, fill probability is low

### Sample Profitable Spread

```json
{"timestamp":"2024-12-11T22:47:11.840Z","spread_pct":"1.01","combined":"0.99"}
{"timestamp":"2024-12-11T22:47:11.844Z","spread_pct":"3.09","combined":"0.97"} ← BEST
{"timestamp":"2024-12-11T22:47:11.853Z","spread_pct":"-1.02","combined":"1.01"} ← GONE
```

Window: **13 milliseconds**

## Recommendations

### Short-term (This Week)
1. ✅ Keep Phase 2 (pre-signing) running
2. 📊 Continue data collection
3. 🔍 Analyze fill rates and spread patterns
4. 🧪 Test ladder strategy (passive fills)

### Medium-term (This Month)
1. 🤖 Train ML model to predict spread timing
2. 📈 Optimize position sizing
3. 💡 Explore alternative arb opportunities
4. 🔬 Study top trader behavior patterns

### Long-term (Future)
1. 💰 If profitable: Invest in Phase 4 (co-location)
2. 🔓 Reverse-engineer WebSocket protocol (Phase 3)
3. ⚡ Optimize to <1ms if ROI justifies cost
4. 🏗️ Build custom FPGA solution ($10k+) for <100μs

## Code Structure

```
src/
├── presign.rs       ✅ Phase 2: Pre-signed order cache
├── ws_clob.rs       🚧 Phase 3: WebSocket submission (skeleton)
├── strategy.rs      ✅ Updated to use pre-signing
├── main.rs          ✅ Integrated pre-sign cache
└── clob.rs          ✅ REST API client (fallback)
```

## Running the Bot

```bash
# With HFT pre-signing (current)
cargo run --release

# Output shows:
# ⚡ Pre-signing orders for HFT mode...
# ✅ Pre-signed 1248 orders ready for instant execution
# ⚡ Using pre-signed orders (lookup: 145μs)
# Total latency: 21ms
```

## Performance Monitoring

Key metrics to track:
- Spread detection time
- Order submission latency
- Fill rate vs spread size
- Profitability per spread window

Log format:
```
🎯 SNIPING spread 1.2%! UP@0.48, DOWN@0.49
⚡ Using pre-signed orders (lookup: 145μs)
🎯 Snipe successful! Potential profit: $12 | Total latency: 21ms
```

## Conclusion

**Current state:** Phase 2 complete, running at 20ms latency

**Reality check:** Still 7x too slow for 13ms spread windows

**Path forward:**
1. Keep optimizing (Phase 3/4)
2. OR pivot to different strategy
3. OR accept low fill rate and run at scale

The HFT game is tough. Pre-signing gets us competitive infrastructure, but sub-10ms execution requires either WebSocket protocol access or co-location investment.
