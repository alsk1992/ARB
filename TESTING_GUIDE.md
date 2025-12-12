# HFT Bot Testing Guide

## Current Status

✅ **Full HFT stack implemented:**
- Pre-signed orders (0ms signing)
- HTTP/2 multiplexing (5-10ms network)
- Connection pooling + TCP tuning
- Parallel order submission

✅ **Running in DRY_RUN mode** (no real money)

## What to Watch For

### 1. Pre-Signing Performance

Look for this on startup:
```
⚡ Pre-signing orders for HFT mode...
[2-3 minutes of signing]
✅ Pre-signed 1248 orders ready for instant execution
```

**Check:**
- Does it pre-sign ~1200 orders?
- How long does it take? (should be 2-3 min)

### 2. Spread Detection

When orderbook updates, watch for:
```
🎯 SNIPING spread 0.8%! UP@0.48, DOWN@0.49
```

**Key question:** How often do spreads appear?

### 3. Execution Latency (THE CRITICAL METRIC)

When bot tries to snipe, look for timing:
```
⚡ Using pre-signed orders (lookup: 145μs)
⚡ HFT parallel orders posted: Submit: 6ms | Total: 7ms
```

OR in dry mode:
```
[DRY RUN] Would snipe spread
```

**What we need to know:**
- **Submit time:** How long to send both orders? (target: <10ms)
- **Total time:** From spread detected to orders sent? (target: <15ms)

### 4. Data Collection

Bot logs every orderbook update to:
```
./data/snapshots_YYYYMMDD_HHMMSS.jsonl
```

**After 10-15 minutes, analyze:**
```bash
# Count profitable spreads
jq -r 'select((.spread_pct | tonumber) > 0.5) | [.timestamp, .spread_pct, .combined] | @csv' data/snapshots_*.jsonl | wc -l

# Show best spreads
jq -r 'select((.spread_pct | tonumber) > 0.5) | [.timestamp, .spread_pct, .combined] | @csv' data/snapshots_*.jsonl | sort -t, -k2 -rn | head -10
```

## Success Criteria

### ✅ Ready to Go Live IF:

1. **Latency competitive:**
   - Submit time: <10ms ✅
   - Total execution: <15ms ✅

2. **Spreads exist:**
   - See profitable spreads (>0.5%) at least once every 5 minutes
   - Some spreads last >10ms (we have a chance)

3. **System stable:**
   - No crashes
   - WebSocket stays connected
   - Pre-signed orders working

### ⚠️ Need More Work IF:

1. **Too slow:**
   - Submit time: >20ms → Need to investigate network
   - Total execution: >30ms → Something wrong with implementation

2. **No spreads:**
   - Zero profitable spreads in 30 min → Market too efficient
   - All spreads <5ms duration → Need co-location (Phase 4)

3. **Errors:**
   - Pre-signing fails
   - WebSocket disconnects
   - Orders rejected (even in dry mode)

## Live Testing Decision Tree

```
After 30 min of dry run data:

┌─ Latency <10ms? ─ Yes ─┐
│                         │
│                    Spreads exist? ─ Yes ─┐
│                                          │
│                              Spread duration >10ms? ─ Yes ─┐
│                                                            │
│                                                    ✅ GO LIVE
│                                                            │
│                                                    Set DRY_RUN=false
│                                                    Start with small position ($100)
│                                                    Monitor for 1 hour
│
└─ Latency >20ms? ─ Yes ─┐
                         │
                    ❌ INVESTIGATE
                         │
                    Check network
                    Check server location
                    Consider Phase 4 (co-location)

└─ No spreads? ─ Yes ─┐
                      │
                  ❌ PIVOT STRATEGY
                      │
                  Try ladder strategy instead
                  Or find different markets
                  Or wait for more volatile periods
```

## Commands to Run

### Monitor the bot (in terminal where you started it)

Watch for:
- Spread detections
- Execution times
- Any errors

### Analyze data (in another terminal)

```bash
cd /Users/alsk/poly/btc-arb-bot

# Wait 10-15 minutes, then:

# Count total snapshots
wc -l data/snapshots_*.jsonl

# Find profitable moments
jq 'select((.spread_pct | tonumber) > 0.5)' data/snapshots_*.jsonl

# Calculate spread duration (if we find consecutive profitable snapshots)
jq -r 'select((.spread_pct | tonumber) > 0.5) | .timestamp' data/snapshots_*.jsonl

# Show best spreads with timestamps
jq -r 'select((.spread_pct | tonumber) > 0.5) | "\(.timestamp) | \(.spread_pct)% | $\(.combined)"' data/snapshots_*.jsonl
```

## What Success Looks Like

**Ideal scenario:**
```
22:30:15.123 | 0.8% spread detected
⚡ Using pre-signed orders (lookup: 132μs)
⚡ HFT parallel orders posted: Submit: 7ms | Total: 8ms
[DRY RUN] Would snipe spread
```

**Translation:**
- Spread appeared
- We looked up pre-signed order in 0.1ms
- We would have submitted in 7ms
- **If spread lasted >10ms, we would have caught it!** ✅

## When to Go Live

**Conservative approach (recommended):**
1. Run dry mode for 30-60 minutes
2. Analyze the data
3. Calculate: "If we went live, would we have caught any spreads?"
4. If yes → Start with $100-200 position size
5. Monitor closely for first hour
6. Scale up if profitable

**Aggressive approach (higher risk):**
1. Run dry mode for 10 minutes
2. See a few spreads with >10ms duration
3. Go live immediately with $500-1000
4. Trust the HFT optimization

## Red Flags - Stop Immediately If:

1. **Latency suddenly spikes** (>100ms) - Network issue
2. **WebSocket keeps disconnecting** - Connection problem
3. **"Order rejected" errors** (even dry mode) - API issue
4. **Pre-signing fails to complete** - Memory or signing issue
5. **No orderbook updates for >30 sec** - Data feed problem

## Next Steps After Testing

### If it looks good:
1. Set `DRY_RUN=false` in `.env`
2. Start with small size: `MAX_POSITION_USD=100`
3. Run for 1 hour
4. Check if we actually captured any fills
5. Scale up if profitable

### If latency too high:
1. Check: Are we on AWS us-east-1? (same region as Polymarket)
2. Consider Phase 4: Rent dedicated server closer to CLOB
3. Cost: $500-2000/month for <1ms latency

### If no spreads:
1. Try different time of day (more volatile during US hours)
2. Consider ladder strategy (passive fills over minutes)
3. Look for different markets (not BTC 15-min)

## Current Session

You're running now. Let it collect data for 10-15 min, then:

```bash
# Check latest snapshots file
ls -lh data/snapshots_*.jsonl | tail -1

# Quick check for any profitable spreads
jq 'select((.spread_pct | tonumber) > 0) | {time: .timestamp, spread: .spread_pct}' data/snapshots_*.jsonl | head -20
```

If you see spreads appearing and execution would be <10ms, you're ready to go live.
