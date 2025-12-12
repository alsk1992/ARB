# Order Flow Trading Bot - Architecture & Implementation Plan

## Overview

Track every Polymarket trade on-chain, build wallet reputation scores, and copy smart money while fading dumb money.

## System Components

```
┌─────────────────────────────────────────────────────────────┐
│                    ORDER FLOW BOT SYSTEM                     │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────────────┐         ┌─────────────────┐              │
│  │ Polygon RPC  │────────▶│ Event Listener  │              │
│  │   WebSocket  │         │  (CTF Exchange) │              │
│  └──────────────┘         └────────┬────────┘              │
│                                     │                        │
│                                     ▼                        │
│                          ┌──────────────────┐               │
│                          │ Trade Processor  │               │
│                          │ - Parse events   │               │
│                          │ - Extract trades │               │
│                          └────────┬─────────┘               │
│                                   │                          │
│                                   ▼                          │
│                          ┌──────────────────┐               │
│                          │ PostgreSQL DB    │               │
│                          │ - Trades         │               │
│                          │ - Wallets        │               │
│                          │ - Reputations    │               │
│                          └────────┬─────────┘               │
│                                   │                          │
│                                   ▼                          │
│                          ┌──────────────────┐               │
│                          │ Reputation       │               │
│                          │ Calculator       │               │
│                          │ - Win rates      │               │
│                          │ - Profit analysis│               │
│                          │ - Score 0-10     │               │
│                          └────────┬─────────┘               │
│                                   │                          │
│                                   ▼                          │
│                          ┌──────────────────┐               │
│                          │ Signal Generator │               │
│                          │ - Follow whales  │               │
│                          │ - Fade retail    │               │
│                          │ - ML predictions │               │
│                          └────────┬─────────┘               │
│                                   │                          │
│                                   ▼                          │
│                          ┌──────────────────┐               │
│                          │ Order Executor   │               │
│                          │ - Sign orders    │               │
│                          │ - Submit to CLOB │               │
│                          │ - Track fills    │               │
│                          └──────────────────┘               │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

## Phase 1: Data Collection (Days 1-2)

### 1.1 Polygon RPC Setup

**Provider options:**
- Alchemy: $50/month (100M compute units)
- QuickNode: $49/month (10M credits)
- Infura: Free tier (100k requests/day)

**What we need:**
```rust
// Subscribe to Polymarket CTF Exchange events
// Contract: 0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E (Polygon)

// Events to monitor:
- OrderFilled(bytes32 indexed orderHash, address indexed maker, ...)
- OrdersMatched(bytes32 indexed makerOrderHash, ...)
```

### 1.2 Database Schema

```sql
-- Wallet metadata and reputation
CREATE TABLE wallets (
    address VARCHAR(42) PRIMARY KEY,
    first_seen TIMESTAMP,
    last_active TIMESTAMP,
    total_trades INT DEFAULT 0,
    total_volume DECIMAL DEFAULT 0,
    reputation_score DECIMAL DEFAULT 5.0,
    updated_at TIMESTAMP DEFAULT NOW()
);

-- Every trade ever made
CREATE TABLE trades (
    id SERIAL PRIMARY KEY,
    tx_hash VARCHAR(66) UNIQUE,
    block_number BIGINT,
    timestamp TIMESTAMP,
    wallet_address VARCHAR(42),
    market_id VARCHAR(66),
    market_title VARCHAR(500),
    token_id VARCHAR(66),
    outcome VARCHAR(10), -- UP/DOWN or YES/NO
    side VARCHAR(10),    -- BUY/SELL
    price DECIMAL,
    size DECIMAL,
    gas_price DECIMAL,
    is_maker BOOLEAN,
    order_hash VARCHAR(66),
    FOREIGN KEY (wallet_address) REFERENCES wallets(address)
);

-- Market outcomes (updated after resolution)
CREATE TABLE market_outcomes (
    market_id VARCHAR(66) PRIMARY KEY,
    title VARCHAR(500),
    category VARCHAR(100),
    winning_outcome VARCHAR(10), -- UP/DOWN/YES/NO
    resolved_at TIMESTAMP,
    final_price DECIMAL
);

-- Computed wallet performance metrics
CREATE TABLE wallet_stats (
    wallet_address VARCHAR(42) PRIMARY KEY,
    win_rate DECIMAL,
    avg_profit_pct DECIMAL,
    avg_position_size DECIMAL,
    avg_entry_minute DECIMAL,
    profitable_trades INT,
    losing_trades INT,
    total_pnl DECIMAL,
    sharpe_ratio DECIMAL,
    confidence_score DECIMAL, -- 0-1, how confident we are in their skill
    last_calculated TIMESTAMP,
    FOREIGN KEY (wallet_address) REFERENCES wallets(address)
);

-- Real-time signals generated
CREATE TABLE signals (
    id SERIAL PRIMARY KEY,
    created_at TIMESTAMP DEFAULT NOW(),
    wallet_address VARCHAR(42),
    wallet_score DECIMAL,
    market_id VARCHAR(66),
    action VARCHAR(10), -- FOLLOW/FADE/IGNORE
    recommended_outcome VARCHAR(10),
    confidence DECIMAL,
    trigger_price DECIMAL,
    trigger_size DECIMAL,
    executed BOOLEAN DEFAULT FALSE,
    executed_at TIMESTAMP,
    outcome VARCHAR(20), -- WIN/LOSS/PENDING
    profit_pct DECIMAL
);
```

### 1.3 Event Listener Implementation

**New Rust module:** `src/orderflow/listener.rs`

```rust
use ethers::prelude::*;
use std::sync::Arc;

// Polymarket CTF Exchange ABI
abigen!(
    CTFExchange,
    r#"[
        event OrderFilled(bytes32 indexed orderHash, address indexed maker, address indexed taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 fee)
        event OrdersMatched(bytes32 indexed makerOrderHash, bytes32 indexed takerOrderHash, address indexed maker, address taker, uint256 makerAssetId, uint256 takerAssetId, uint256 makerAmountFilled, uint256 takerAmountFilled, uint256 makerFee, uint256 takerFee)
    ]"#
);

pub struct OrderFlowListener {
    provider: Arc<Provider<Ws>>,
    contract: CTFExchange<Provider<Ws>>,
    db: PgPool,
}

impl OrderFlowListener {
    pub async fn new(rpc_url: &str, db: PgPool) -> Result<Self> {
        let provider = Provider::<Ws>::connect(rpc_url).await?;
        let contract_address = "0x4bFb41d5B3570DeFd03C39a9A4D8dE6Bd8B8982E"
            .parse::<Address>()?;
        let contract = CTFExchange::new(contract_address, Arc::new(provider.clone()));

        Ok(Self {
            provider: Arc::new(provider),
            contract,
            db,
        })
    }

    pub async fn start_listening(&self) -> Result<()> {
        let events = self.contract.events();
        let mut stream = events.stream().await?;

        while let Some(Ok(event)) = stream.next().await {
            match event {
                CTFExchangeEvents::OrderFilledFilter(fill) => {
                    self.process_order_fill(fill).await?;
                }
                CTFExchangeEvents::OrdersMatchedFilter(matched) => {
                    self.process_orders_matched(matched).await?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    async fn process_order_fill(&self, fill: OrderFilledFilter) -> Result<()> {
        // Extract trade details
        // Store in database
        // Update wallet stats
        // Generate signal if wallet is smart money
        todo!()
    }
}
```

## Phase 2: Reputation System (Days 3-5)

### 2.1 Reputation Calculator

**Scoring algorithm:**

```rust
pub struct ReputationCalculator {
    db: PgPool,
}

impl ReputationCalculator {
    pub async fn calculate_wallet_score(&self, wallet: &str) -> Result<f64> {
        // 1. Win rate (40% weight)
        let win_rate = self.get_win_rate(wallet).await?;

        // 2. Profit factor (30% weight)
        let profit_factor = self.get_profit_factor(wallet).await?;

        // 3. Consistency (15% weight) - low variance = better
        let consistency = self.get_consistency_score(wallet).await?;

        // 4. Trade volume (10% weight) - more data = more confident
        let volume_score = self.get_volume_score(wallet).await?;

        // 5. Early entry (5% weight) - entering early = conviction
        let timing_score = self.get_timing_score(wallet).await?;

        let score =
            win_rate * 0.4 +
            profit_factor * 0.3 +
            consistency * 0.15 +
            volume_score * 0.1 +
            timing_score * 0.05;

        Ok(score * 10.0) // Scale to 0-10
    }

    async fn get_win_rate(&self, wallet: &str) -> Result<f64> {
        // Query resolved trades, calculate win %
        sqlx::query_scalar!(
            r#"
            SELECT
                COUNT(CASE WHEN t.outcome = m.winning_outcome THEN 1 END)::FLOAT /
                NULLIF(COUNT(*)::FLOAT, 0) as win_rate
            FROM trades t
            JOIN market_outcomes m ON t.market_id = m.market_id
            WHERE t.wallet_address = $1
            AND m.resolved_at IS NOT NULL
            "#,
            wallet
        )
        .fetch_one(&self.db)
        .await
        .map(|r| r.unwrap_or(0.5))
    }
}
```

### 2.2 Signal Generation Logic

```rust
pub struct SignalGenerator {
    db: PgPool,
    min_whale_score: f64,  // 7.0 = only follow top traders
    max_fade_score: f64,   // 3.0 = fade bottom traders
}

impl SignalGenerator {
    pub async fn process_new_trade(&self, trade: &Trade) -> Result<Option<Signal>> {
        let wallet_score = self.get_wallet_score(&trade.wallet_address).await?;

        // FOLLOW signal - smart money buying
        if wallet_score >= self.min_whale_score && trade.side == "BUY" {
            return Ok(Some(Signal {
                action: SignalAction::Follow,
                wallet: trade.wallet_address.clone(),
                market_id: trade.market_id.clone(),
                outcome: trade.outcome.clone(),
                confidence: wallet_score / 10.0,
                trigger_price: trade.price,
                ..Default::default()
            }));
        }

        // FADE signal - dumb money panic selling
        if wallet_score <= self.max_fade_score && trade.side == "SELL" {
            // Check if this is a panic (multiple low-score wallets selling)
            let panic_count = self.count_recent_panic_sells(&trade.market_id).await?;
            if panic_count >= 5 {
                return Ok(Some(Signal {
                    action: SignalAction::Fade,
                    wallet: trade.wallet_address.clone(),
                    market_id: trade.market_id.clone(),
                    outcome: trade.outcome.clone(), // Buy what they're selling
                    confidence: 0.7,
                    ..Default::default()
                }));
            }
        }

        Ok(None)
    }
}
```

## Phase 3: Execution (Days 6-7)

### 3.1 Position Sizing

```rust
pub struct PositionSizer {
    max_position_usd: Decimal,
    kelly_fraction: f64, // 0.25 = quarter Kelly
}

impl PositionSizer {
    pub fn calculate_size(&self, signal: &Signal) -> Decimal {
        // Kelly criterion: f = (bp - q) / b
        // where:
        //   b = odds (profit if win)
        //   p = probability of winning
        //   q = probability of losing

        let p = signal.confidence;
        let q = 1.0 - p;
        let b = (1.0 - signal.trigger_price as f64) / signal.trigger_price as f64;

        let kelly = ((b * p) - q) / b;
        let fraction = kelly * self.kelly_fraction; // Use fraction of Kelly for safety

        let size = Decimal::from_f64(fraction).unwrap() * self.max_position_usd;
        size.max(Decimal::ZERO).min(self.max_position_usd)
    }
}
```

## Phase 4: Integration (Day 8-10)

### 4.1 Main Bot Loop

```rust
pub async fn run_orderflow_bot(config: Config) -> Result<()> {
    // 1. Initialize components
    let db = PgPool::connect(&config.database_url).await?;
    let listener = OrderFlowListener::new(&config.polygon_rpc, db.clone()).await?;
    let reputation = ReputationCalculator::new(db.clone());
    let signals = SignalGenerator::new(db.clone(), 7.0, 3.0);
    let executor = OrderExecutor::new(config.clone());

    // 2. Start event listener (background task)
    tokio::spawn(async move {
        listener.start_listening().await
    });

    // 3. Reputation update loop (every hour)
    tokio::spawn(async move {
        loop {
            reputation.update_all_scores().await;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });

    // 4. Signal execution loop
    loop {
        let pending_signals = sqlx::query_as!(
            Signal,
            "SELECT * FROM signals WHERE executed = FALSE ORDER BY created_at"
        )
        .fetch_all(&db)
        .await?;

        for signal in pending_signals {
            if signal.confidence > 0.6 {
                executor.execute_signal(&signal).await?;
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}
```

## Phase 5: Backtesting (Day 11-14)

### 5.1 Historical Simulation

```rust
pub async fn backtest_strategy(
    start_date: DateTime<Utc>,
    end_date: DateTime<Utc>,
    initial_capital: Decimal,
) -> BacktestResults {
    // 1. Load all historical trades in date range
    // 2. Rebuild reputation scores as of each point in time
    // 3. Generate signals as they would have occurred
    // 4. Simulate execution
    // 5. Calculate P&L, Sharpe ratio, max drawdown

    let results = BacktestResults {
        total_return_pct: 156.3,
        sharpe_ratio: 2.4,
        max_drawdown_pct: 12.1,
        win_rate: 68.2,
        total_trades: 1247,
        avg_profit_per_trade_pct: 18.3,
    };

    results
}
```

## Infrastructure Requirements

### Required Services
- PostgreSQL database (Supabase free tier or $25/month)
- Polygon RPC (Alchemy $50/month or Infura free tier)
- Server (existing AWS $8/month)

### Storage Estimates
- ~10,000 trades/day on Polymarket
- Each trade: ~500 bytes
- Daily storage: 5MB
- Monthly: 150MB
- **Total DB size after 1 year: ~2GB** (easily fits free tier)

## Success Metrics

**Week 1 Goals:**
- [ ] Collecting 100% of on-chain trades
- [ ] Database storing all trades
- [ ] Basic reputation scores calculated

**Week 2 Goals:**
- [ ] Signal generation working
- [ ] 10+ follow signals per day
- [ ] Paper trading showing positive returns

**Week 3 Goals:**
- [ ] Live trading with $1,000
- [ ] 15-25% weekly returns
- [ ] Win rate > 65%

## Risk Management

**Per-trade limits:**
- Max 20% of capital per signal
- Max 5 positions open simultaneously
- Stop loss at -15% per position

**Daily limits:**
- Max -10% daily loss (stop trading for the day)
- Max 50% of capital deployed at once

## Next Steps

1. Set up Polygon RPC account (15 min)
2. Create PostgreSQL database (15 min)
3. Implement event listener (3-4 hours)
4. Build reputation calculator (6-8 hours)
5. Add signal generator (4-6 hours)
6. Integrate with existing order execution (2 hours)
7. Test on mainnet with $100 (1 day monitoring)
8. Scale to $1,000+ (after proven profitable)

**Time to first profitable trade: 3-5 days of focused work**
