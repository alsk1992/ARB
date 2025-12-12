# Quick Start - Test ETH and SOL Markets

## Super Simple 3-Step Process

### Step 1: Setup and Test ETH Market

**SSH into server:**
```bash
ssh -i ~/.ssh/btc-bot-key.pem ubuntu@3.237.6.125
cd /home/ubuntu/ARB
```

**Start ETH test:**
```bash
# Find next market end time (check current time)
date -u

# If it's 00:05 UTC now, next market ends at 00:15:00
./test_market.sh eth "2025-12-12T00:15:00Z"

# This will:
# - Stop current bot
# - Update .env to ETH market
# - Start bot for ETH
# - Create monitor_eth.sh script
```

**Start monitoring:**
```bash
nohup ./monitor_eth.sh > monitor_eth.log 2>&1 &
tail -f monitor_eth.log
```

**Wait 15 minutes** for results!

### Step 2: Test SOL Market

**After ETH completes, run:**
```bash
# Next market is 15 minutes later
./test_market.sh sol "2025-12-12T00:30:00Z"

# Start monitoring
nohup ./monitor_sol.sh > monitor_sol.log 2>&1 &
tail -f monitor_sol.log
```

### Step 3: Compare Results

**View all results:**
```bash
# ETH results
cat monitor_eth.log

# SOL results
cat monitor_sol.log

# BTC results (already done)
cat monitor.log
```

**Compare the key numbers:**

| Metric | BTC | ETH | SOL | Winner |
|--------|-----|-----|-----|--------|
| Spreads Found | 3 | ? | ? | ? |
| Avg Duration | 0.16ms | ?ms | ?ms | ? |
| Avg Size | 1.01% | ?% | ?% | ? |
| Catchable? | ❌ No | ? | ? | ? |

## What You're Looking For

**BEST CASE (Go Live!):**
```
Spreads Found: 5-10
Avg Duration: 15-50ms
Avg Size: 1-2%
Catchable? ✅ YES
```

**If you find this:** Set `DRY_RUN=false` and go live on that market!

**WORST CASE (Like BTC):**
```
Spreads Found: 0-3
Avg Duration: <1ms
Avg Size: 1%
Catchable? ❌ NO (need co-location)
```

**If you find this:** Skip the market.

## One-Liner Testing Schedule

**Test all 3 markets in 45 minutes:**

```bash
# 00:00 - Test BTC (already done)
# Results: 3 spreads, 0.16ms avg, NOT catchable

# 00:15 - Test ETH
./test_market.sh eth "2025-12-12T00:30:00Z"
nohup ./monitor_eth.sh > monitor_eth.log 2>&1 &

# 00:30 - Test SOL
./test_market.sh sol "2025-12-12T00:45:00Z"
nohup ./monitor_sol.sh > monitor_sol.log 2>&1 &

# 00:45 - Review all results
cat monitor_eth.log | grep "ANALYSIS COMPLETE" -B 30
cat monitor_sol.log | grep "ANALYSIS COMPLETE" -B 30
```

## Quick Commands Reference

**Check what's running:**
```bash
ps aux | grep btc-arb
```

**Kill bot:**
```bash
pkill -f btc-arb-bot
```

**View live bot logs:**
```bash
tail -f bot_eth.log   # ETH bot
tail -f bot_sol.log   # SOL bot
```

**View monitor progress:**
```bash
tail -f monitor_eth.log
tail -f monitor_sol.log
```

**Manual analysis (if needed):**
```bash
# Find latest data file
ls -lht data/snapshots_*.jsonl | head -1

# Analyze manually
./analyze_spreads.sh data/snapshots_20251212_001234.jsonl "Ethereum Up or Down"
```

## Market End Time Calculator

**Markets end at :00, :15, :30, :45 past each hour in UTC**

**Current time:**
```bash
date -u
# Thu Dec 12 00:07:30 UTC 2025
```

**Next end times:**
- If current minute is 00-14: next end is :15
- If current minute is 15-29: next end is :30
- If current minute is 30-44: next end is :45
- If current minute is 45-59: next end is :00 (next hour)

**Format for script:**
```bash
# If it's Dec 12 00:07 UTC
# Next market ends at 00:15
./test_market.sh eth "2025-12-12T00:15:00Z"

# Next after that is 00:30
./test_market.sh sol "2025-12-12T00:30:00Z"
```

## Expected Results Analysis

After running both tests, you'll get output like:

```
============================================
SPREAD ANALYSIS FOR: Ethereum Up or Down
============================================

[1/5] Extracting market data...
Total snapshots: 3200

[2/5] Finding profitable spreads (>0.5%)...
Profitable spreads found: 7

[3/5] Profitable spread details:
  [timestamps and prices]

[4/5] Calculating spread durations...
  Spread at 00:18:22.123456789Z (1.5%):
    → Duration appears to be ~25ms

[5/5] Summary statistics:
  Average spread: -1.2%
  Max spread: 1.5%
  Profitable snapshots: 7 / 3200 (0.22%)
```

**Interpretation:**
- 7 spreads in 15 min = Good frequency ✅
- ~25ms duration = Catchable with your code ✅
- 1.5% profit = Worth trading ✅
- **Decision: GO LIVE on ETH!** ✅

## Next Steps After Testing

**If you find a good market (10ms+ spreads):**

1. **Update .env:**
```bash
nano .env
# Set DRY_RUN=false
# Set MAX_POSITION_USD=100
# Set MARKET_SLUG to the winner (eth-up-or-down)
```

2. **Go live:**
```bash
./target/release/btc-arb-bot
```

3. **Monitor for 1 hour:**
```bash
tail -f data/snapshots_*.jsonl | grep "SNIP"
```

4. **Check if you caught any:**
- Look for "✅ Orders submitted" in logs
- Check Polymarket for your fills
- Calculate P&L

5. **Scale up if profitable:**
- Increase MAX_POSITION_USD to $500
- Run for 24 hours
- Monitor profitability

**If all markets are like BTC (sub-ms spreads):**

1. **Accept HFT not viable without co-location**
2. **Pivot to ladder strategy** (place limit orders, wait for fills)
3. **Or** spend $500-2k/month on co-location (Equinix Ashburn)
4. **Or** move on to different opportunities

## Troubleshooting

**"Market not found":**
- Check MARKET_SLUG in Polymarket URL
- Verify market exists at that time
- Try different asset

**"No spreads found":**
- Normal! Some markets very efficient
- Try different time of day
- Try during high volatility (US market hours)

**"Bot not collecting data":**
- Check bot logs: `tail -f bot_eth.log`
- Verify WebSocket connected
- Restart bot if needed

**"Monitor script not running":**
- Check market end time is in future
- Verify date format: `YYYY-MM-DDTHH:MM:SSZ`
- Run manually: `./monitor_eth.sh`

## Summary

You have 3 scripts now:

1. **test_market.sh** - Quick setup for any market (btc/eth/sol)
2. **monitor_XXX.sh** - Auto-generated monitor for each market
3. **analyze_spreads.sh** - Manual analysis tool

**To test ETH and SOL:**
```bash
# Test ETH (15 min)
./test_market.sh eth "2025-12-12T00:15:00Z"
nohup ./monitor_eth.sh > monitor_eth.log 2>&1 &

# Test SOL (15 min)
./test_market.sh sol "2025-12-12T00:30:00Z"
nohup ./monitor_sol.sh > monitor_sol.log 2>&1 &

# Wait 30 minutes total, then review logs
```

**Goal:** Find a market with 10ms+ spread durations, then go live and print money!
