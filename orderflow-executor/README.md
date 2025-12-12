# Order Flow Executor

Generates trading signals from whale activity and executes trades automatically.

## How It Works

**Signal Generation (every 3 seconds):**
1. Query orderflow_trades for whale trades in last 10 seconds
2. If whale (score 7+) bought → Generate FOLLOW_WHALE signal
3. If 5+ degens (score <3) panic sold → Generate FADE_DEGEN signal
4. Calculate position size using Kelly criterion
5. Save signal to orderflow_signals table

**Signal Execution:**
1. Fetch pending signals with confidence > threshold
2. Check risk limits (max positions, daily loss)
3. Execute trade (paper or real)
4. Update signal status
5. Create position record

## Signal Types

### FOLLOW_WHALE
- Triggered when: Wallet with score 7+ buys
- Action: Copy their trade (BUY same outcome)
- Confidence: wallet_score / 10
- Example: Whale buys YES @ 0.65 → We buy YES @ ≤0.68 (5% slippage)

### FADE_DEGEN
- Triggered when: 5+ wallets with score <3 panic sell same market
- Action: Buy what they're selling
- Confidence: Fixed 0.7
- Example: 7 degens sell NO @ 0.40 → We buy NO @ ≤0.42

## Position Sizing

**Kelly Criterion:**
```
f = (bp - q) / b

where:
  b = odds (profit if win) = (1 - price) / price
  p = confidence (from wallet score)
  q = 1 - confidence

We use quarter Kelly (0.25 fraction) for safety
```

**Example:**
- Whale (score 9.0) buys YES @ 0.70
- Confidence: 0.9
- Kelly: ((0.43 * 0.9) - 0.1) / 0.43 = 0.67
- Quarter Kelly: 0.67 * 0.25 = 0.168 (16.8% of capital)
- If MAX_POSITION_USD=1000 → Position size = $168

## Risk Management

**Per-trade limits:**
- Max position: MAX_POSITION_USD (default: $1000)
- Max slippage: 5% above whale's price
- Min confidence: MIN_SIGNAL_CONFIDENCE (default: 0.7)

**Daily limits:**
- Max open positions: MAX_OPEN_POSITIONS (default: 5)
- Daily loss limit: MAX_DAILY_LOSS (default: $500)
- If loss limit hit → Stop trading for the day

## Paper Trading Mode

**ENABLE_PAPER_TRADING=true (RECOMMENDED FOR TESTING)**

When enabled:
- Signals are generated normally
- Execution is simulated (logged but not submitted)
- Positions tracked in database
- Can backtest strategy without risk
- Use this for 1-2 weeks before going live

## Real Trading Mode

**ENABLE_PAPER_TRADING=false**

Requirements:
- POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE (from Polymarket)
- PRIVATE_KEY (wallet with USDC balance)

What happens:
- Fetch current orderbook price from CLOB
- Build EIP-712 signed order
- Submit to Polymarket CLOB API
- Track order fill status
- Update positions in real-time

## Configuration

**Risk Settings:**
```bash
MAX_POSITION_USD=1000        # Max $ per trade
MIN_SIGNAL_CONFIDENCE=0.7    # Only execute signals > 70% confidence
MAX_DAILY_LOSS=500          # Stop trading if down $500 today
MAX_OPEN_POSITIONS=5        # Max concurrent positions
```

**Signal Thresholds:**
```bash
MIN_WHALE_SCORE=7.0         # Follow wallets with score ≥ 7
MAX_FADE_SCORE=3.0          # Fade wallets with score ≤ 3
```

**Feature Flags:**
```bash
ENABLE_PAPER_TRADING=true        # Simulate trades (no real $)
ENABLE_WHALE_FOLLOWING=true      # Follow high-score wallets
ENABLE_DEGEN_FADING=false        # Fade low-score wallets (risky!)
```

**Position Sizing:**
```bash
KELLY_FRACTION=0.25         # Use quarter Kelly (0.25 = conservative)
```

## Setup

1. **Set environment variables**:
   ```bash
   cp .env.example .env
   # Edit .env with your settings
   ```

2. **Start with paper trading**:
   ```bash
   ENABLE_PAPER_TRADING=true cargo run
   ```

3. **Monitor signals**:
   ```sql
   SELECT * FROM orderflow_signals ORDER BY created_at DESC LIMIT 10;
   ```

4. **Deploy to Railway**:
   - Create new service from GitHub repo
   - Set root directory to `/orderflow-executor`
   - Add environment variables
   - Railway will auto-build using Dockerfile

## Monitoring

**Logs to watch for**:
- `🐋 WHALE SIGNAL: ...` - New whale trade detected
- `🚨 PANIC SIGNAL: ...` - Degen panic sell detected
- `✅ Executed signal #...` - Trade executed
- `⚠️ Risk limits reached` - Stopped trading (hit limits)

**Expected metrics**:
- 5-10 signals per day (whale following)
- 1-3 signals per day (degen fading)
- 70%+ execution rate
- Memory usage: 100-200 MB
- CPU usage: <5%

## Database Tables

**orderflow_signals** - Generated signals
- Created when whale trades or degens panic
- Status: PENDING → EXECUTED/SKIPPED/EXPIRED
- Tracks P&L after market resolution

**orderflow_positions** - Open positions
- Created when signal executed
- Tracks unrealized P&L
- Closed when market resolves

## Example Workflow

**1. Whale trades:**
```
2025-01-15 14:32:15 - 🐋 WHALE SIGNAL
  Whale: 0x1234...5678 (score: 8.5)
  Market: BTC-UPDOWN-15M-1736953200
  Action: BUY YES @ 0.72
  Confidence: 85%
  Position size: $212 (Kelly 0.21)
```

**2. Signal execution:**
```
2025-01-15 14:32:18 - ✅ Executed signal #847
  Bought YES @ 0.74 for $212
  Slippage: 2.7%
  Position #123 opened
```

**3. Market resolves:**
```
2025-01-15 14:47:00 - 🎯 WIN
  Position #123 closed
  Entry: 0.74, Exit: 1.00
  Profit: $75 (+35%)
  Updated wallet stats
```

## Troubleshooting

**No signals generated:**
- Check that orderflow-listener is running
- Verify whale trades exist: `SELECT * FROM orderflow_trades WHERE timestamp > NOW() - INTERVAL '1 hour' LIMIT 10`
- Check MIN_WHALE_SCORE threshold (maybe too high)

**Signals not executing:**
- Check MIN_SIGNAL_CONFIDENCE (maybe too high)
- Verify risk limits not hit
- Check ENABLE_PAPER_TRADING setting

**High slippage:**
- Markets move fast in 15-minute windows
- Consider tighter max_price limits
- Execute faster (reduce polling interval)

## Next Steps

1. **Week 1**: Paper trade with ENABLE_PAPER_TRADING=true
2. **Week 2**: If 70%+ win rate, go live with $100 positions
3. **Week 3**: If profitable, increase to $500 positions
4. **Week 4**: If still profitable, increase to $1000-5000 positions
