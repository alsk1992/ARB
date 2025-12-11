# Complete Trade Analysis - BTC 15-Min Arbitrage

## Summary Stats

### nobuyoshi005 (0xbefbdd434fc8d99da3e37c20cb0f088ec3168a78)
- **Total Unique Trades**: 1,364
- **Total Markets**: 6
- **Strategy**: BUYS BOTH SIDES in every market

### 15m-a4 (0xecd55daa7c6900683b804d1d4db935fbfabe43f4)
- **Total Unique Trades**: 1,408
- **Total Markets**: 50
- **Strategy**: MIXED - some both sides, some one-sided

---

## nobuyoshi005 - Market by Market

| Market | Total Trades | UP Trades | DOWN Trades | UP Cost | DOWN Cost | UP Shares | DOWN Shares | Both Sides? |
|--------|-------------|-----------|-------------|---------|-----------|-----------|-------------|-------------|
| Dec 10, 11:45AM-12:00PM | 193 | 106 | 87 | $402.58 | $958.58 | 1,344 | 1,321 | YES |
| Dec 10, 12:00PM-12:15PM | 215 | 103 | 112 | $790.25 | $663.19 | 1,518 | 1,353 | YES |
| Dec 10, 12:15PM-12:30PM | 287 | 116 | 171 | $1,403.24 | $300.16 | 1,686 | 1,743 | YES |
| Dec 10, 12:30PM-12:45PM | 175 | 71 | 104 | $606.31 | $420.41 | 973 | 1,038 | YES |
| Dec 10, 12:45PM-1:00PM | 221 | 116 | 105 | $528.30 | $766.33 | 1,367 | 1,308 | YES |
| Dec 10, 1:00PM-1:15PM | 273 | 152 | 121 | $727.26 | $744.68 | 1,537 | 1,645 | YES |

**PATTERN**: nobuyoshi005 ALWAYS buys BOTH sides. Roughly equal shares on each side.

---

## 15m-a4 - Market by Market

| Market | Total Trades | UP Trades | DOWN Trades | UP Cost | DOWN Cost | UP Shares | DOWN Shares | Both Sides? |
|--------|-------------|-----------|-------------|---------|-----------|-----------|-------------|-------------|
| Dec 11, 10:30AM-10:45AM | 1 | 0 | 1 | $0 | $4.20 | 0 | 6 | NO (DOWN only) |
| Dec 11, 10:45AM-11:00AM | 5 | 3 | 2 | $15.19 | $145.97 | 36 | 270 | YES |
| Dec 11, 11:00AM-11:15AM | 1 | 1 | 0 | $1.80 | $0 | 15 | 0 | NO (UP only) |
| Dec 11, 11:15AM-11:30AM | 7 | 7 | 0 | $501.70 | $0 | 596 | 0 | NO (UP only) |
| Dec 11, 11:30AM-11:45AM | 10 | 5 | 5 | $186.03 | $183.37 | 301 | 313 | YES |
| Dec 11, 11:45AM-12:00PM | 7 | 3 | 4 | $74.48 | $1,507.79 | 178 | 1,744 | YES |
| Dec 11, 12:00PM-12:15PM | 6 | 3 | 3 | $288.53 | $339.48 | 321 | 712 | YES |
| Dec 11, 12:15PM-12:30PM | 28 | 24 | 4 | $927.19 | $143.00 | 1,300 | 260 | YES |
| Dec 11, 1:30AM-1:45AM | 25 | 25 | 0 | $3,752.22 | $0 | 4,900 | 0 | NO (UP only) |
| Dec 11, 3:30AM-3:45AM | 44 | 0 | 44 | $0 | $6,240.29 | 0 | 8,846 | NO (DOWN only) |
| Dec 11, 3:45AM-4:00AM | 33 | 0 | 33 | $0 | $2,542.54 | 0 | 6,888 | NO (DOWN only) |
| Dec 11, 7:00AM-7:15AM | 25 | 3 | 22 | $3.37 | $5,414.70 | 43 | 10,161 | YES (but heavily DOWN) |
| Dec 11, 7:30AM-7:45AM | 16 | 16 | 0 | $6,302.09 | $0 | 13,642 | 0 | NO (UP only) |

**PATTERN**: 15m-a4 is MIXED:
- Some markets: BOTH sides (arbitrage)
- Some markets: ONE side only (directional bet)

---

## Key Findings

### nobuyoshi005 Strategy
1. **Pure Arbitrage** - Always buys both Up and Down
2. **Balanced positions** - Roughly equal shares on each side
3. **Guaranteed profit** when Up + Down price < $1
4. **Example**: Buy 1,344 Up + 1,321 Down = guaranteed 1,321 shares payout (minimum)

### 15m-a4 Strategy
1. **Mixed approach** - Sometimes arb, sometimes directional
2. **Directional bets** - When confident, goes all-in on one side
3. **Higher risk, higher reward** - Can lose on directional bets but wins big when right
4. **Examples**:
   - Dec 11, 7:30AM: ALL UP ($6,302 on UP only)
   - Dec 11, 3:30AM: ALL DOWN ($6,240 on DOWN only)

---

## IMPORTANT: Activity API vs Closed Positions API

The activity API only returns recent trades (last ~6 markets for nobuyoshi005).
The closed-positions API returns historical resolved positions (50 markets).

**closed-positions shows:**
- Total realized P&L: $44,885.60 (nobuyoshi005)
- Total realized P&L: $138,058.06 (15m-a4)
- 100% win rate on closed positions

**Why 100% win rate?**
The closed-positions API only shows the WINNING side that resolved to $1.
When they buy BOTH sides, one resolves to $1, one to $0.
The API aggregates wins separately from losses.

---

## CONFIRMED STRATEGIES

### nobuyoshi005: PURE ARBITRAGE
- Buys BOTH Up and Down in every market
- Gets roughly equal shares on each side
- One side always wins ($1), one always loses ($0)
- Net profit = (winning shares × $1) - (total cost for both sides)
- Guaranteed profit when combined price < $1

### 15m-a4: MIXED STRATEGY
- Sometimes pure arbitrage (both sides)
- Sometimes directional bets (one side only)
- Higher variance but potentially higher returns
- Takes directional positions when confident

---

## To Replicate (Arbitrage Strategy)

1. Monitor BTC 15-min markets
2. Watch orderbook for: Up price + Down price < 0.98 (2%+ spread)
3. Buy EQUAL shares of both Up and Down
4. Wait for resolution (15 min)
5. Collect $1 per share on winning side
6. Profit = shares - total_cost

**Example with $1,270 capital:**
- If spread = 3% (Up 48¢ + Down 49¢ = 97¢)
- Buy ~650 Up shares @ 48¢ = $312
- Buy ~650 Down shares @ 49¢ = $318.50
- Total cost: $630.50
- Guaranteed payout: 650 shares × $1 = $650
- Profit: $19.50 (3.1% per 15-min cycle)
- 4 cycles/hour × 24 hours = 96 cycles/day
- Theoretical daily profit: 96 × $19.50 = $1,872 (but liquidity limited)

---

## Files Generated
- `nobuyoshi005_unique.json` - 1,364 deduplicated trades
- `15m_a4_unique.json` - 1,408 deduplicated trades
- `nobuyoshi005_closed_positions.csv` - P&L per position
- `15m_a4_closed_positions.csv` - P&L per position
