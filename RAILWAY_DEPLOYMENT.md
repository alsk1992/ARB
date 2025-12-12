# Railway Deployment Guide - Order Flow Bot

## Architecture Decision: Add to PolyTrack Project ✅

**Why this is optimal:**
1. Share PostgreSQL database (whale data synergy!)
2. PolyTrack already tracks wallets → instant reputation data
3. Cross-reference: PolyTrack UI shows wallet, order flow shows their trades
4. Single Railway bill

## Railway Services Setup

### Service 1: Order Flow Listener (New)
**Purpose:** Stream Polygon blockchain, capture all Polymarket trades

```yaml
name: orderflow-listener
build:
  context: ./orderflow-bot
  dockerfile: Dockerfile.listener
environment:
  DATABASE_URL: ${{Postgres.DATABASE_URL}}
  POLYGON_RPC_URL: wss://polygon-mainnet.g.alchemy.com/v2/YOUR_KEY
  RUST_LOG: info
resources:
  memory: 512MB
  cpu: 0.5
restart: always
healthcheck:
  enabled: true
  path: /health
```

### Service 2: Reputation Calculator (New)
**Purpose:** Update wallet scores every hour

```yaml
name: orderflow-reputation
build:
  context: ./orderflow-bot
  dockerfile: Dockerfile.reputation
environment:
  DATABASE_URL: ${{Postgres.DATABASE_URL}}
  CALCULATION_INTERVAL_SECONDS: 3600
resources:
  memory: 256MB
  cpu: 0.25
restart: always
```

### Service 3: Signal Executor (New)
**Purpose:** Execute trades when whales make moves

```yaml
name: orderflow-executor
build:
  context: ./orderflow-bot
  dockerfile: Dockerfile.executor
environment:
  DATABASE_URL: ${{Postgres.DATABASE_URL}}
  PRIVATE_KEY: ${{TRADING_PRIVATE_KEY}}
  POLY_API_KEY: ${{POLY_API_KEY}}
  POLY_API_SECRET: ${{POLY_API_SECRET}}
  POLY_API_PASSPHRASE: ${{POLY_API_PASSPHRASE}}
  MAX_POSITION_USD: 1000
  MIN_SIGNAL_CONFIDENCE: 0.7
resources:
  memory: 256MB
  cpu: 0.25
restart: always
depends_on:
  - orderflow-listener
```

### Existing: PolyTrack Services
- polytrack-next (frontend)
- polytrack-server (backend API)
- postgres (shared database) ← **We add tables here**

## Cost Estimate

**Current PolyTrack costs:**
- PostgreSQL: $5/month
- polytrack-next: $5/month
- polytrack-server: $5/month
- **Total: ~$15/month**

**Adding order flow services:**
- orderflow-listener: $5/month (512MB)
- orderflow-reputation: $3/month (256MB)
- orderflow-executor: $3/month (256MB)
- Alchemy Polygon RPC: $50/month (100M compute units)
- **Additional: ~$61/month**

**New total: ~$76/month** (vs $15 now)

**ROI if profitable:**
- 20% weekly on $1,000 capital = $200/week
- Monthly: ~$800 profit
- Net after costs: $800 - $61 = **$739/month profit**
- **10x return on infrastructure costs!**

## Deployment Steps

### Step 1: Database Migration (5 minutes)

```bash
# In Railway dashboard:
# 1. Go to PolyTrack project → PostgreSQL service
# 2. Connect to database
# 3. Run migration:

railway run psql $DATABASE_URL -f migrations/001_orderflow_schema.sql
```

### Step 2: Create New Services (10 minutes)

**In Railway dashboard:**

1. **Add orderflow-listener service:**
   - New → GitHub Repo → Select `btc-arb-bot`
   - Set root directory: `/orderflow-listener`
   - Add env vars: `DATABASE_URL`, `POLYGON_RPC_URL`

2. **Add orderflow-reputation service:**
   - New → GitHub Repo → Select `btc-arb-bot`
   - Set root directory: `/orderflow-reputation`
   - Add env var: `DATABASE_URL`

3. **Add orderflow-executor service:**
   - New → GitHub Repo → Select `btc-arb-bot`
   - Set root directory: `/orderflow-executor`
   - Add env vars: All trading credentials

### Step 3: Environment Variables

**Add to Railway project:**

```env
# Polygon RPC (Alchemy)
POLYGON_RPC_URL=wss://polygon-mainnet.g.alchemy.com/v2/YOUR_API_KEY

# Polymarket Trading API
POLY_API_KEY=your_key_here
POLY_API_SECRET=your_secret_here
POLY_API_PASSPHRASE=your_passphrase_here

# Trading wallet (DO NOT commit to git!)
TRADING_PRIVATE_KEY=0xYOUR_PRIVATE_KEY
TRADING_ADDRESS=0xYOUR_ADDRESS

# Risk management
MAX_POSITION_USD=1000
MIN_SIGNAL_CONFIDENCE=0.7
MAX_DAILY_LOSS=500
MAX_OPEN_POSITIONS=5

# Feature flags
ENABLE_PAPER_TRADING=true  # Start with this!
ENABLE_WHALE_FOLLOWING=true
ENABLE_DEGEN_FADING=false  # Turn on later
```

## Project Structure

```
btc-arb-bot/
├── orderflow-listener/
│   ├── Dockerfile
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs           # Event listener
│   │   ├── polygon.rs        # Polygon RPC client
│   │   └── storage.rs        # Save to PostgreSQL
│   └── .dockerignore
│
├── orderflow-reputation/
│   ├── Dockerfile
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs           # Reputation calculator
│       ├── calculator.rs     # Scoring algorithm
│       └── models.rs         # Database models
│
├── orderflow-executor/
│   ├── Dockerfile
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs           # Signal executor
│       ├── signals.rs        # Signal generator
│       ├── executor.rs       # Order execution
│       └── risk.rs           # Position sizing
│
├── migrations/
│   └── 001_orderflow_schema.sql
│
├── shared/
│   └── src/
│       ├── models.rs         # Shared types
│       └── db.rs             # Database client
│
└── railway.json              # Railway config
```

## Monitoring & Alerts

### Railway Metrics to Watch

**orderflow-listener:**
- Trades/minute (should be ~7-10)
- Memory usage (should be <400MB)
- Restart count (should be 0)

**orderflow-executor:**
- Signals generated/hour (target: 5-10)
- Execution success rate (target: >95%)
- P&L (target: positive!)

### Discord Webhook Setup

```rust
// In each service, send critical alerts to Discord
pub async fn send_alert(message: &str) {
    let webhook_url = env::var("DISCORD_WEBHOOK_URL").unwrap();
    let payload = json!({
        "content": message,
        "username": "Order Flow Bot"
    });

    reqwest::Client::new()
        .post(&webhook_url)
        .json(&payload)
        .send()
        .await
        .ok();
}

// Examples:
send_alert("🚨 HIGH CONFIDENCE SIGNAL: Whale bought $50k BTC UP").await;
send_alert("✅ Trade executed: +$127 profit (23%)").await;
send_alert("⚠️ Daily loss limit reached: -$500").await;
```

## Testing Strategy

### Phase 1: Paper Trading (Week 1)
- `ENABLE_PAPER_TRADING=true`
- Generate signals, simulate execution
- Track what profit WOULD have been
- Goal: 70%+ win rate, 20%+ avg profit

### Phase 2: Micro Trading (Week 2)
- `ENABLE_PAPER_TRADING=false`
- `MAX_POSITION_USD=100`
- Real money, tiny positions
- Goal: Break even or small profit

### Phase 3: Small Scale (Week 3)
- `MAX_POSITION_USD=500`
- If Week 2 profitable
- Goal: $100+ weekly profit

### Phase 4: Full Scale (Week 4+)
- `MAX_POSITION_USD=5000`
- If Week 3 consistently profitable
- Goal: $1,000+ weekly profit

## Rollback Plan

**If something goes wrong:**

```bash
# Stop all order flow services immediately
railway down orderflow-listener
railway down orderflow-executor
railway down orderflow-reputation

# Check positions
psql $DATABASE_URL -c "SELECT * FROM orderflow_positions WHERE status = 'OPEN';"

# Close positions manually via Polymarket UI if needed

# Review logs
railway logs orderflow-executor --lines 1000

# Fix issues, redeploy when ready
```

## Security Checklist

- [ ] Private key stored in Railway env vars (NOT in code)
- [ ] API secrets in Railway env vars (NOT in code)
- [ ] `.env` files in `.gitignore`
- [ ] Database has SSL enabled
- [ ] Max position limits set
- [ ] Daily loss limits set
- [ ] Discord alerts configured
- [ ] Paper trading enabled first

## Next Steps

1. **Right now:** Set up Alchemy account ($50/month)
2. **Today:** Run database migration on Railway PostgreSQL
3. **This weekend:** Build orderflow-listener service
4. **Next week:** Build reputation calculator
5. **Week after:** Build executor, start paper trading
6. **Week 3:** Go live with $100 if paper trading profitable

**Want me to start writing the Rust code for the listener service?**
