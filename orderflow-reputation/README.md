# Order Flow Reputation Calculator

Calculates wallet reputation scores (0-10) based on trading performance.

## Scoring Algorithm

**5 weighted factors:**

1. **Win Rate (40%)** - % of trades that were profitable
   - 70%+ win rate = top score
   - 50% win rate = neutral
   - <30% win rate = degen

2. **Profit Factor (30%)** - Average profit per trade
   - 20-30% avg profit = excellent
   - 10-15% avg profit = good
   - <5% avg profit = poor

3. **Consistency (15%)** - Low variance in results
   - Steady wins = high score
   - Erratic results = low score

4. **Volume (10%)** - Number of trades (confidence metric)
   - 1000+ trades = max confidence
   - 100 trades = medium confidence
   - <10 trades = low confidence

5. **Timing (5%)** - Early entry indicates conviction
   - Entering at minute 0-3 = high score
   - Entering at minute 13-15 = low score

## Trader Tiers

- **WHALE** (8-10): Follow their trades
- **SMART** (6-8): Solid traders, worth watching
- **AVERAGE** (4-6): Neutral, no signal
- **NOVICE** (2-4): Learning, avoid following
- **DEGEN** (0-2): Fade their trades (bet opposite)

## How It Works

```
Every hour (configurable):
    1. Query all wallets with trades in last 30 days
    2. For each wallet:
        - Calculate 5 scoring factors
        - Compute weighted reputation score (0-10)
        - Determine confidence level (0-1)
        - Assign trader tier (WHALE/SMART/AVERAGE/NOVICE/DEGEN)
        - Save to orderflow_wallet_stats
        - Save to orderflow_reputation_history
    3. Log progress every 100 wallets
    4. Sleep until next calculation
```

## Setup

1. **Set environment variables**:
   ```bash
   cp .env.example .env
   # Edit .env with your DATABASE_URL
   ```

2. **Run locally**:
   ```bash
   cargo run
   ```

3. **Deploy to Railway**:
   - Create new service from GitHub repo
   - Set root directory to `/orderflow-reputation`
   - Add environment variables
   - Railway will auto-build using Dockerfile

## Configuration

**CALCULATION_INTERVAL_SECONDS** - How often to recalculate scores
- Default: 3600 (1 hour)
- For testing: 300 (5 minutes)
- Production: 3600-7200 (1-2 hours)

## Monitoring

**Logs to watch for**:
- `✅ Connected to PostgreSQL` - Database connected
- `📊 Calculating reputation for X wallets` - Starting calculation
- `Progress: 100/500 wallets updated` - Progress counter
- `✅ Updated reputation for X wallets` - Calculation complete

**Expected metrics**:
- ~100-500 wallets per calculation
- ~1-5 seconds per wallet
- Total calculation time: 2-10 minutes
- Memory usage: 100-200 MB
- CPU usage: 10-30% during calculation, idle between

## Database Tables

**orderflow_wallet_stats** - Current reputation
- Updated every calculation cycle
- Used by signal generator to determine follow/fade

**orderflow_reputation_history** - Historical scores
- Snapshot of each calculation
- Used to track reputation changes over time
- Useful for analyzing "hot streaks" vs "cold streaks"

## Troubleshooting

**No wallets found**:
- Check that orderflow-listener is running and collecting trades
- Verify orderflow_trades table has data

**Low confidence scores**:
- Normal for new wallets with <20 trades
- Confidence increases logarithmically with trade count

**Scores seem wrong**:
- Check that market outcomes are being populated
- Verify orderflow_market_outcomes table has resolved markets
- Most markets resolve within 15 minutes of end time

## Future Improvements

- Add category-specific scores (crypto vs politics vs sports)
- Weight recent performance higher than old trades
- Detect "hot streaks" and boost scores temporarily
- Add Sharpe ratio and max drawdown to scoring
