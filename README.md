# BTC 15-Minute Arbitrage Bot

Automated trading bot for Polymarket's BTC 15-minute binary markets.

## Strategy

Polymarket has markets every 15 minutes: "Will BTC go up or down?"

**The arb:**
- Buy BOTH "Up" and "Down" outcomes
- If combined cost < $1, profit is guaranteed
- Example: Buy Up @ 45¢ + Down @ 48¢ = 93¢ cost → $1 payout → 7¢ profit

**The ladder:**
- Place 30 limit orders at different price levels on each side
- As price oscillates, orders fill at various prices
- Average cost ends up lower than a single market order
- This is how pro traders (nobuyoshi005, 0xf247...) execute

## Architecture

```
src/
├── main.rs              # Entry point, trading loop
├── config.rs            # Environment config
├── auth.rs              # HMAC-SHA256 API auth
├── signer.rs            # EIP-712 order signing
├── clob.rs              # Polymarket CLOB API client
├── market.rs            # Market discovery
├── orderbook.rs         # Local orderbook tracking
├── websocket.rs         # Real-time price feeds
├── position.rs          # Position tracking
├── strategy.rs          # Original ladder strategy
├── strategies/          # Multi-strategy framework
│   ├── mod.rs           # Strategy trait
│   ├── pure_arb.rs      # Strategy 1: Pure arbitrage
│   ├── scalper.rs       # Strategy 2: Buy low sell high
│   ├── market_maker.rs  # Strategy 3: Capture spreads
│   ├── momentum.rs      # Strategy 4: Follow trends
│   └── hybrid.rs        # Strategy 5: Combined approach
├── multi_strategy.rs    # Run all strategies in parallel
├── datalog.rs           # Save data for ML analysis
├── ml_client.rs         # ML prediction client
├── alerts.rs            # Discord notifications
└── retry.rs             # Retry logic, circuit breaker

ml/
├── extract_features.py  # Feature extraction from logs
├── train_models.py      # Train XGBoost models
├── predict.py           # Prediction server
├── auto_train.py        # Auto-retrain on new data
└── requirements.txt     # Python dependencies

deploy/
├── setup.sh             # AWS EC2 setup script
└── README.md            # Deployment guide
```

## Setup

### Prerequisites
- Rust 1.70+
- Polymarket account with API credentials
- Private key for signing orders

### Configuration

Create `.env` file:

```env
POLY_API_KEY=your_api_key
POLY_API_SECRET=your_api_secret
POLY_API_PASSPHRASE=your_passphrase
POLY_ADDRESS=0xYourAddress
PRIVATE_KEY=your_private_key
MAX_POSITION_USD=100
TARGET_SPREAD_PERCENT=4
MIN_SPREAD_PERCENT=2
LADDER_LEVELS=30
ORDER_SIZE_PER_LEVEL=20
DRY_RUN=true
LOG_LEVEL=info
DISCORD_WEBHOOK=
```

### Build & Run

```bash
# Build
cargo build --release

# Run (dry mode - no real orders)
./target/release/btc-arb-bot

# Run with logging
RUST_LOG=info ./target/release/btc-arb-bot
```

## Multi-Strategy Testing

The bot can run 5 strategies in parallel on the same market data to compare performance:

1. **pure_arb** - Buy both sides, hold to resolution (safest)
2. **scalper** - Take profit on price movements
3. **market_maker** - Post buy/sell orders, capture spread
4. **momentum** - Add to position when trend detected
5. **hybrid** - Combination of above

After each market, see which strategy performed best. Deploy the winner with real money.

## ML Pipeline

```bash
# Install Python deps
cd ml && pip install -r requirements.txt

# After collecting data with DRY_RUN=true:
python extract_features.py   # Extract features
python train_models.py       # Train models
python predict.py --serve    # Start prediction server
```

The bot will use ML predictions to optimize:
- Entry timing
- Price level selection
- Fill probability

## Deployment (AWS)

See `deploy/README.md` for full AWS EC2 setup guide.

Quick version:
```bash
# On server
./deploy/setup.sh
nano .env  # Add credentials
sudo systemctl start btc-arb-bot
journalctl -u btc-arb-bot -f  # View logs
```

Recommended: `c6i.large` in `us-east-1` (~$62/month) for lowest latency to Polymarket.

## Remote Access

### Quick Connect (after SSH config setup)
```bash
ssh arb                     # Connect to server
ssh arb "cd ~/ARB && nvim ." # Open neovim on server
```

### SSH Config Setup
Add to `~/.ssh/config`:
```
Host arb
    HostName 3.237.6.125
    User ubuntu
    IdentityFile /path/to/btc-bot-key.pem
```

### Remote Editing Options

**Option 1: SSH + Neovim on server (simplest)**
```bash
ssh arb
cd ~/ARB && nvim .
```

**Option 2: SSHFS mount (edit locally)**
```bash
sudo apt install sshfs
mkdir -p ~/arb-remote
sshfs arb:~/ARB ~/arb-remote
nvim ~/arb-remote/
# Unmount when done: fusermount -u ~/arb-remote
```

**Option 3: Neovim built-in SCP**
```bash
nvim scp://arb//home/ubuntu/ARB/
```

**Option 4: rsync workflow**
```bash
# Pull changes
rsync -avz arb:~/ARB/ ./ARB-local/
# Push changes
rsync -avz ./ARB-local/ arb:~/ARB/
```

### Useful Server Commands
```bash
# View bot logs
ssh arb "journalctl -u btc-arb-bot -f"

# Check ML server
ssh arb "curl -s -X POST http://127.0.0.1:8765 -d '{\"spread_now\": 3.5}'"

# Check processes
ssh arb "ps aux | grep -E '(btc-arb|predict)'"

# View collected data
ssh arb "cat ~/ARB/data/summaries.jsonl"
```

## Key Files to Understand

1. **`src/signer.rs`** - EIP-712 signing (critical for orders to work)
2. **`src/strategy.rs`** - Ladder generation logic
3. **`src/strategies/`** - Different trading approaches
4. **`src/websocket.rs`** - Real-time orderbook updates

## API Endpoints Used

- `clob.polymarket.com` - Order placement, orderbook
- `data-api.polymarket.com` - Trade history, positions
- `gamma-api.polymarket.com` - Market discovery
- `wss://ws-subscriptions-clob.polymarket.com/ws/` - WebSocket feeds

## Disclaimer

This is experimental trading software. Use at your own risk. Start with small amounts and DRY_RUN=true.
