# Order Flow Trading System - Project Structure

## 📁 Directory Layout

```
btc-arb-bot/
│
├── orderflow-listener/          # Service 1: Blockchain event listener
│   ├── src/
│   │   ├── main.rs             # Entry point, connects to Polygon + DB
│   │   ├── polygon.rs          # WebSocket listener, event parsing
│   │   ├── storage.rs          # PostgreSQL insertion logic
│   │   └── types.rs            # Trade data structures
│   ├── Cargo.toml              # Dependencies (ethers, sqlx, tokio)
│   ├── Dockerfile              # Multi-stage build for Railway
│   ├── .env.example            # Environment variables template
│   └── README.md               # Service-specific documentation
│
├── orderflow-reputation/        # Service 2: Wallet reputation calculator
│   ├── src/
│   │   ├── main.rs             # Entry point, calculation loop
│   │   ├── calculator.rs       # 5-factor scoring algorithm
│   │   └── models.rs           # WalletStats, ReputationScore types
│   ├── Cargo.toml              # Dependencies (sqlx, tokio, decimal)
│   ├── Dockerfile              # Multi-stage build for Railway
│   ├── .env.example            # Environment variables template
│   └── README.md               # Service-specific documentation
│
├── orderflow-executor/          # Service 3: Signal generator & executor
│   ├── src/
│   │   ├── main.rs             # Entry point, execution loop
│   │   ├── config.rs           # ExecutorConfig from env vars
│   │   ├── signals.rs          # Signal generation (whale/degen)
│   │   ├── executor.rs         # Order execution logic
│   │   └── risk.rs             # Risk management (limits, checks)
│   ├── Cargo.toml              # Dependencies (sqlx, ethers, reqwest)
│   ├── Dockerfile              # Multi-stage build for Railway
│   ├── .env.example            # Environment variables template
│   └── README.md               # Service-specific documentation
│
├── migrations/
│   └── 001_orderflow_schema.sql  # PostgreSQL schema (10 tables + views)
│
├── docs/
│   ├── ORDER_FLOW_STRATEGY.md    # Strategy explanation & architecture
│   ├── RAILWAY_DEPLOYMENT.md     # Railway-specific deployment guide
│   └── DEPLOYMENT_GUIDE.md       # Complete step-by-step deployment
│
├── src/                          # Original arbitrage bot (kept for reference)
│   ├── main.rs
│   ├── market.rs
│   ├── orderbook.rs
│   └── ...
│
├── README.md                     # Main project README
└── Cargo.toml                    # Original bot dependencies
```

---

## 🔧 Service Details

### Service 1: orderflow-listener

**Purpose:** Stream all Polymarket trades from Polygon blockchain

**Technology:**
- Rust + tokio (async runtime)
- ethers-rs (Ethereum/Polygon RPC)
- sqlx (PostgreSQL client)

**Key Features:**
- WebSocket connection to Polygon RPC (Alchemy)
- Subscribes to CTF Exchange contract events
- Captures OrderFilled + OrdersMatched events
- Stores trades in `orderflow_trades` table
- Deduplication via `tx_hash` unique constraint

**Performance:**
- Memory: 200-400 MB
- CPU: <10%
- Throughput: ~10 trades/minute

**Docker:**
- Multi-stage build (builder + runtime)
- Final image: ~50 MB
- Health check via process monitoring

---

### Service 2: orderflow-reputation

**Purpose:** Calculate wallet reputation scores (0-10) based on trading performance

**Technology:**
- Rust + tokio
- sqlx (PostgreSQL client)
- Complex SQL queries for analytics

**Key Features:**
- Runs every hour (configurable)
- 5-factor scoring algorithm:
  1. Win rate (40%)
  2. Profit factor (30%)
  3. Consistency (15%)
  4. Volume (10%)
  5. Timing (5%)
- Assigns trader tiers: WHALE / SMART / AVERAGE / NOVICE / DEGEN
- Saves historical snapshots

**Performance:**
- Memory: 100-200 MB
- CPU: 10-30% during calculation, idle between
- Duration: 2-10 minutes per calculation
- Wallets scored: 100-500 per run

**Docker:**
- Multi-stage build
- Final image: ~40 MB
- Health check via process monitoring

---

### Service 3: orderflow-executor

**Purpose:** Generate trading signals and execute trades

**Technology:**
- Rust + tokio
- sqlx (PostgreSQL)
- reqwest (HTTP client for CLOB API)
- ethers-rs (order signing)

**Key Features:**
- Polls every 3 seconds for new whale trades
- Generates signals:
  - **FOLLOW_WHALE**: Copy high-score wallet trades
  - **FADE_DEGEN**: Bet opposite of panic sells
- Position sizing via Kelly criterion
- Risk management:
  - Max open positions
  - Daily loss limits
  - Per-trade limits
- Paper trading mode (simulated execution)

**Performance:**
- Memory: 100-200 MB
- CPU: <5%
- Signals: 5-10 per day (whale following)

**Docker:**
- Multi-stage build
- Final image: ~45 MB
- Health check via process monitoring

---

## 🗃️ Database Schema

### Core Tables

**orderflow_trades** (24 columns)
- Every trade ever made on Polymarket
- ~10,000 rows/day
- Primary key: id
- Unique: tx_hash
- Indexed: wallet_address, market_id, timestamp

**orderflow_wallet_stats** (26 columns)
- Aggregated performance metrics per wallet
- Primary key: wallet_address
- Updated hourly by reputation service
- Contains reputation_score, trader_tier

**orderflow_market_outcomes** (12 columns)
- Market resolution data
- Primary key: market_id
- Contains winning_outcome, resolved_at

**orderflow_signals** (17 columns)
- Generated trading signals
- Primary key: id
- Status: PENDING → EXECUTED/SKIPPED/EXPIRED
- Tracks P&L after resolution

**orderflow_positions** (13 columns)
- Currently open positions
- Primary key: id
- References signals(id)
- Status: OPEN → CLOSED

**orderflow_reputation_history** (7 columns)
- Historical snapshots of wallet scores
- Used to track score changes over time

**orderflow_performance** (14 columns)
- System-wide daily stats
- Win rate, P&L, Sharpe ratio

### Views

**orderflow_top_wallets**
- Top 100 wallets by reputation score
- Last 30 days activity

**orderflow_hot_signals**
- Recent high-confidence signals (last hour)
- Confidence > 70%

### Functions & Triggers

**update_wallet_stats()**
- Trigger function that runs after each trade insert
- Updates basic stats (trade count, last trade time)

---

## 🚀 Deployment Flow

### Local Development

```bash
# Listener
cd orderflow-listener
cargo run

# Reputation
cd orderflow-reputation
cargo run

# Executor
cd orderflow-executor
cargo run
```

### Railway Deployment

**Build:**
- Railway detects Dockerfile automatically
- Runs multi-stage build
- Pushes image to registry

**Deploy:**
- Pulls image
- Sets environment variables
- Starts container
- Monitors health checks

**Auto-redeploy:**
- Push to GitHub main branch
- Railway auto-detects changes
- Rebuilds and redeploys affected services

---

## 📊 Data Flow

```
1. Polygon Blockchain
   ↓
2. orderflow-listener (WebSocket)
   ↓
3. PostgreSQL orderflow_trades
   ↓
4. orderflow-reputation (hourly calculation)
   ↓
5. PostgreSQL orderflow_wallet_stats (scores updated)
   ↓
6. orderflow-executor (polls every 3s)
   ↓
7. PostgreSQL orderflow_signals (new signals)
   ↓
8. orderflow-executor (executes signals)
   ↓
9. PostgreSQL orderflow_positions (tracking)
   ↓
10. Market resolves
   ↓
11. Update P&L in orderflow_signals
```

---

## 🔑 Environment Variables

### All Services
```
DATABASE_URL=postgresql://...  # Railway PostgreSQL
RUST_LOG=info                  # Logging level
```

### orderflow-listener
```
POLYGON_RPC_URL=wss://polygon-mainnet.g.alchemy.com/v2/...
```

### orderflow-reputation
```
CALCULATION_INTERVAL_SECONDS=3600  # How often to recalculate
```

### orderflow-executor
```
# API credentials (optional, for real trading)
POLY_API_KEY=...
POLY_API_SECRET=...
POLY_API_PASSPHRASE=...
PRIVATE_KEY=0x...

# Risk management
MAX_POSITION_USD=1000
MIN_SIGNAL_CONFIDENCE=0.7
MAX_DAILY_LOSS=500
MAX_OPEN_POSITIONS=5

# Signal thresholds
MIN_WHALE_SCORE=7.0
MAX_FADE_SCORE=3.0

# Feature flags
ENABLE_PAPER_TRADING=true
ENABLE_WHALE_FOLLOWING=true
ENABLE_DEGEN_FADING=false

# Position sizing
KELLY_FRACTION=0.25
```

---

## 📈 Monitoring

### Key Metrics

**orderflow-listener:**
- Trades/minute: 7-10
- Memory: <400 MB
- Uptime: 99.9%

**orderflow-reputation:**
- Wallets scored/hour: 100-500
- Calculation duration: 2-10 min
- Memory: <200 MB

**orderflow-executor:**
- Signals/day: 5-10
- Execution rate: 70%+
- Win rate: 70%+ (target)

### Health Checks

All services have Docker health checks:
```dockerfile
HEALTHCHECK --interval=30s --timeout=10s CMD pgrep service_name
```

Railway monitors these and auto-restarts if unhealthy.

---

## 🔄 Development Workflow

### Making Changes

1. **Local testing:**
   ```bash
   cd orderflow-listener
   cargo test
   cargo run
   ```

2. **Commit changes:**
   ```bash
   git add orderflow-listener/
   git commit -m "Fix: Handle WebSocket reconnection"
   git push origin main
   ```

3. **Railway auto-deploys:**
   - Detects changes in `/orderflow-listener`
   - Rebuilds only affected service
   - Deploys new version
   - Monitors health checks

### Adding Features

**New signal type example:**

1. Add to `orderflow-executor/src/signals.rs`
2. Update database if needed (new migration)
3. Test locally
4. Deploy to Railway
5. Monitor logs for new signals

---

## 📝 Documentation

**High-level:**
- `ORDER_FLOW_STRATEGY.md` - Strategy explanation
- `DEPLOYMENT_GUIDE.md` - Complete deployment walkthrough
- `RAILWAY_DEPLOYMENT.md` - Railway-specific guide

**Service-level:**
- `orderflow-listener/README.md` - Listener details
- `orderflow-reputation/README.md` - Reputation algorithm
- `orderflow-executor/README.md` - Execution flow

**Database:**
- `migrations/001_orderflow_schema.sql` - Schema with comments

---

## 🎯 Project Goals

**Phase 1 (Complete):** Build all 3 services ✅
**Phase 2 (Next):** Deploy to Railway, verify data collection
**Phase 3 (Week 1):** Paper trading, optimize thresholds
**Phase 4 (Week 2):** Go live with $100 positions
**Phase 5 (Month 1):** Scale to $1,000-5,000 positions
**Phase 6 (Month 3):** $5,000+ monthly profit, full automation

---

## 🧪 Testing

**Unit tests:**
```bash
cd orderflow-reputation
cargo test
```

**Integration tests:**
- Deploy all 3 services
- Verify trades flowing: `SELECT COUNT(*) FROM orderflow_trades`
- Verify scores calculated: `SELECT COUNT(*) FROM orderflow_wallet_stats WHERE reputation_score IS NOT NULL`
- Verify signals generated: `SELECT COUNT(*) FROM orderflow_signals`

**Performance tests:**
- Load test database queries
- Verify sub-second signal generation
- Check memory usage under load

---

## 🔒 Security

**Best practices:**
- Private keys in Railway env vars (encrypted)
- Database connections over SSL
- API secrets never committed to git
- Paper trading enabled by default
- Risk limits enforced in code

**Audit trail:**
- All signals logged to database
- Execution history preserved
- Reputation score changes tracked
- Can replay decisions from logs
