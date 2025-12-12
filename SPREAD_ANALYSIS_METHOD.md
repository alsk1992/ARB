# Spread Analysis Method - Complete Testing Protocol

## Overview

This document describes how to monitor and analyze arbitrage spread opportunities on Polymarket's 15-minute binary markets for any asset (BTC, ETH, SOL, etc.).

## What This Tests

- **Spread Frequency:** How often profitable spreads appear
- **Spread Duration:** How long spreads last (in milliseconds)
- **Spread Size:** Profit percentage available
- **Market Opportunity:** Whether HFT arbitrage is viable for this market

## Prerequisites

**Running on AWS Server:**
- Bot running and collecting data: `./target/release/btc-arb-bot`
- Data being logged to: `./data/snapshots_YYYYMMDD_HHMMSS.jsonl`

**Scripts Needed:**
1. `analyze_spreads.sh` - Analyzes spread data for a market
2. `monitor_full_market.sh` - Monitors live and auto-analyzes when complete

## Step-by-Step Protocol

### 1. Update Bot Configuration for New Market

Edit `.env` file to target the market you want to test:

```bash
# For ETH market
MARKET_SLUG=eth-up-or-down

# For SOL market
MARKET_SLUG=sol-up-or-down

# Keep these settings
TARGET_SPREAD_PERCENT=0.5
DRY_RUN=true
MAX_POSITION_USD=30
```

**Important:** Set `DRY_RUN=true` so you're just collecting data, not trading.

### 2. Start the Bot

```bash
cd /home/ubuntu/ARB  # Or wherever your bot is
./target/release/btc-arb-bot
```

Bot will:
- Connect to WebSocket feed
- Pre-sign orders for the market
- Log every orderbook snapshot to `./data/snapshots_*.jsonl`

### 3. Wait for a New Market to Start

Polymarket 15-minute markets start every 15 minutes:
- **BTC:** :00, :15, :30, :45 past each hour
- **ETH:** Same schedule (check Polymarket to confirm)
- **SOL:** Same schedule (check Polymarket to confirm)

**Pro tip:** Start bot 1-2 minutes before market opens so it's ready.

### 4. Run the Monitor Script

Once market starts, launch the monitoring script:

```bash
cd /home/ubuntu/ARB
./monitor_full_market.sh
```

**You'll need to edit the script first** to set the correct market details:

```bash
# Edit these variables in monitor_full_market.sh:
DATAFILE="/home/ubuntu/ARB/data/snapshots_YYYYMMDD_HHMMSS.jsonl"  # Current data file
MARKET_END_TIME="2025-12-12T00:15:00Z"  # Market end time in UTC
```

Then in the script, change the market title search string:
```bash
# For ETH
grep "Ethereum Up or Down" "$DATAFILE"

# For SOL
grep "Solana Up or Down" "$DATAFILE"
```

### 5. Monitor Progress

The script will update every 30 seconds:

```
[23:50:00] Snapshots: 1761 | Profitable spreads: 2 | Time remaining: 600s
[23:50:30] Snapshots: 1873 | Profitable spreads: 2 | Time remaining: 570s
...
```

**Or watch live:**
```bash
tail -f /home/ubuntu/ARB/monitor.log
```

### 6. Automatic Analysis When Market Completes

When the 15-minute market ends, the script automatically runs full analysis:

```
============================================
SPREAD ANALYSIS FOR: [Market Name]
============================================

[1/5] Extracting market data...
Total snapshots: 3513
First snapshot: 2025-12-11T23:45:35Z
Last snapshot:  2025-12-11T23:57:21Z

[2/5] Finding profitable spreads (>0.5%)...
Profitable spreads found: 3

[3/5] Profitable spread details:
  [timestamp] | [spread %] | UP@$X DOWN@$Y = $Z

[4/5] Calculating spread durations...
  [Shows exact timing of when spreads appeared/disappeared]

[5/5] Summary statistics:
  Average spread: -1.34%
  Min spread: -3.85%
  Max spread: 1.01%
  Profitable snapshots: 3 / 3513 (0.09%)
```

## Understanding the Results

### Key Metrics to Look For

**1. Spread Frequency**
- How many spreads appeared in 15 minutes?
- **Good:** 5+ spreads
- **Moderate:** 2-4 spreads
- **Poor:** 0-1 spreads

**2. Spread Duration**
- How long did each spread last?
- **Sub-millisecond (<1ms):** Impossible without co-location
- **1-10ms:** Need co-location + optimized code
- **10-100ms:** Catchable with current setup
- **100ms+:** Easy to catch

**3. Spread Size**
- What profit % was available?
- **0.5-1%:** Minimal profit (need volume)
- **1-2%:** Decent profit
- **2%+:** Great profit

**4. Market Efficiency**
- What % of snapshots were profitable?
- **<0.1%:** Very efficient (HFT dominated)
- **0.1-1%:** Moderately efficient
- **>1%:** Inefficient (good opportunity)

### Example Results Interpretation

**BTC Market (tested):**
```
Frequency: 3 spreads / 12 min (1 every 4 min)
Duration: 0.14ms, 0.19ms, 0.17ms (average 0.16ms)
Size: 1.01% each
Efficiency: 0.09% profitable snapshots

Verdict: ❌ Sub-millisecond spreads = need co-location
         ❌ Low frequency = not worth the cost
         ❌ Small profit = need huge volume
```

**Hypothetical ETH Market (good opportunity):**
```
Frequency: 8 spreads / 15 min (1 every 2 min)
Duration: 15ms, 23ms, 18ms, 31ms (average 22ms)
Size: 1.5% average
Efficiency: 0.3% profitable snapshots

Verdict: ✅ 15-30ms spreads = catchable with current code
         ✅ High frequency = more opportunities
         ✅ Decent profit = worthwhile
```

## Testing Multiple Markets

### Quick Comparison Method

**Test 3 markets back-to-back:**

1. **BTC at :00 minutes** - Start bot, monitor full 15 min
2. **ETH at :15 minutes** - Restart bot with ETH config, monitor 15 min
3. **SOL at :30 minutes** - Restart bot with SOL config, monitor 15 min

**Then compare results:**

| Market | Spreads | Avg Duration | Avg Size | Catchable? |
|--------|---------|--------------|----------|------------|
| BTC    | 3       | 0.16ms       | 1.01%    | ❌ No      |
| ETH    | ?       | ?ms          | ?%       | ?          |
| SOL    | ?       | ?ms          | ?%       | ?          |

### What to Look For

**Best case scenario:**
- Spreads appear frequently (5+ per 15 min)
- Spreads last 10-50ms (catchable without co-location)
- Spread size 1-3% (profitable at modest volume)

**If you find this:** GO LIVE on that market!

**Worst case (like BTC):**
- Spreads rare (<3 per 15 min)
- Spreads sub-millisecond (<1ms)
- Need co-location to compete

**If you find this:** Skip the market, keep testing others.

## Scripts Reference

### analyze_spreads.sh

**Location:** `/home/ubuntu/ARB/analyze_spreads.sh`

**Usage:**
```bash
./analyze_spreads.sh <datafile> "<market_title_substring>"
```

**Example:**
```bash
./analyze_spreads.sh data/snapshots_20251211_233221.jsonl "Bitcoin Up or Down"
./analyze_spreads.sh data/snapshots_20251212_001234.jsonl "Ethereum Up or Down"
```

**What it does:**
1. Filters snapshots for specified market
2. Finds all profitable spreads (>0.5%)
3. Calculates duration by examining neighboring snapshots
4. Generates statistics

### monitor_full_market.sh

**Location:** `/home/ubuntu/ARB/monitor_full_market.sh`

**Usage:**
```bash
# Edit script first to set:
# - DATAFILE path
# - MARKET_END_TIME
# - Market title search string

./monitor_full_market.sh
```

**What it does:**
1. Monitors live data collection every 30 seconds
2. Shows running count of snapshots and spreads
3. Automatically runs full analysis when market ends
4. Saves results to `monitor.log`

### Manual Analysis Commands

**Count total snapshots for a market:**
```bash
grep "Bitcoin Up or Down" data/snapshots_*.jsonl | wc -l
```

**Find all profitable spreads:**
```bash
grep "Bitcoin Up or Down" data/snapshots_*.jsonl | \
  jq 'select((.spread_pct | tonumber) > 0.5)'
```

**Get spread timeline around a specific time:**
```bash
grep "Bitcoin Up or Down" data/snapshots_*.jsonl | \
  jq -r '[.timestamp, .spread_pct, .up_best_ask, .down_best_ask] | @tsv' | \
  grep -A5 -B5 "23:47:22"
```

## Testing Schedule Template

### Week 1: Survey Phase
**Goal:** Test all available markets, find the best opportunities

**Monday:**
- 9:00 AM - Test BTC market
- 9:15 AM - Test ETH market
- 9:30 AM - Test SOL market
- 9:45 AM - Compare results

**Tuesday-Thursday:**
- Repeat at different times of day (morning, afternoon, evening)
- Check if volatility/spreads change with US market hours

**Friday:**
- Analyze all data
- Rank markets by opportunity
- Choose top 2 markets for Week 2

### Week 2: Deep Dive
**Goal:** Extensive testing of top 2 markets

- Run bot on Market #1 for 8 hours straight
- Collect data from 10+ market periods
- Get statistically significant sample

### Week 3: Decision
**Goal:** Go live or optimize strategy

**If good markets found:**
- Set `DRY_RUN=false`
- Start with small size ($100)
- Monitor for 1 hour
- Scale up if profitable

**If no good markets:**
- Consider co-location for sub-ms spreads
- OR pivot to ladder/market-making strategy
- OR accept HFT not viable

## Expected Outcomes

### Scenario A: Found a Good Market
```
Market: ETH
Spreads: 7 per 15 min
Duration: 20-40ms average
Size: 1.5% average

Action: GO LIVE
- Set DRY_RUN=false
- Start small ($100 position)
- Use pre-signed orders + HTTP/2
- Should catch 50-70% of spreads
```

### Scenario B: All Markets Like BTC
```
All markets show:
- Sub-millisecond spreads
- Low frequency
- Need co-location

Action: PIVOT STRATEGY
- Don't spend $500-2k/month on co-location
- Use ladder strategy instead
- Or manual end-of-market trading
- Or different trading approach
```

### Scenario C: Mixed Results
```
BTC: Sub-ms spreads (skip)
ETH: 10-20ms spreads (maybe)
SOL: 30-50ms spreads (good!)

Action: SELECTIVE TRADING
- Only trade SOL market (best opportunity)
- Skip BTC/ETH (too competitive)
- Focus resources on highest probability
```

## Quick Start Checklist

- [ ] Bot compiled and tested (`cargo build --release`)
- [ ] Scripts created (`analyze_spreads.sh`, `monitor_full_market.sh`)
- [ ] `.env` configured for target market
- [ ] Data directory exists (`./data/`)
- [ ] Bot running and collecting data
- [ ] Market schedule known (when 15-min markets start)
- [ ] Monitor script launched before market starts
- [ ] Results logged and saved
- [ ] Analysis reviewed and decisions made

## Tips for Success

1. **Test during volatile periods:** US market hours (9:30 AM - 4 PM EST) usually more volatile
2. **Test multiple times of day:** Spreads may be different morning vs evening
3. **Save all data files:** You can re-analyze later with different thresholds
4. **Compare to volume:** High-volume markets may have tighter spreads
5. **Watch for patterns:** Some markets may have spreads only near expiry

## Common Issues

**Bot not detecting market:**
- Check `MARKET_SLUG` matches exactly
- Verify market exists on Polymarket
- Ensure bot restarted after config change

**No spreads appearing:**
- Normal! Many markets are very efficient
- Try different market or time of day
- Lower threshold to see near-profitable spreads

**Monitor script counting wrong:**
- Known issue with spread count display
- Final analysis is always accurate
- Just ignore the live count, wait for final results

## Next Steps After Testing

**If spreads are catchable (10ms+):**
1. Set `DRY_RUN=false`
2. Start with `MAX_POSITION_USD=100`
3. Monitor closely for 1 hour
4. Scale up if profitable

**If spreads are sub-millisecond:**
1. Research co-location options (Equinix, Hetzner)
2. Calculate ROI: spread frequency × size × volume vs $500-2k/month
3. Only invest if clearly profitable
4. Or pivot to non-HFT strategy

**If no spreads at all:**
1. Try different markets
2. Try different times of day
3. Consider this market too efficient
4. Move on to other opportunities

## Summary

This method lets you scientifically test whether HFT arbitrage is viable on any Polymarket asset without risking money. In 15 minutes per market, you get:

✅ Exact spread frequency
✅ Exact spread duration (in milliseconds)
✅ Exact profit opportunities
✅ Clear go/no-go decision

Test multiple markets, find the best opportunity, then go live with confidence.
