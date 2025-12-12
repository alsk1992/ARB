# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

BTC 15-minute arbitrage bot for Polymarket. The strategy buys both UP and DOWN outcomes on BTC binary markets when the combined cost is less than $1, guaranteeing profit regardless of outcome.

## Build and Run Commands

```bash
# Build release binary
cargo build --release

# Run bot (uses .env config)
./target/release/btc-arb-bot

# Run with logging
RUST_LOG=info ./target/release/btc-arb-bot
RUST_LOG=debug ./target/release/btc-arb-bot

# Test order signing
cargo run --bin test_signing
```

## ML Pipeline Commands

```bash
cd ml
pip install -r requirements.txt

# Feature extraction from collected data
python extract_features.py

# Train models
python train_models.py

# Start prediction server (used by Rust bot)
python predict.py --serve 8765

# Auto-trainer (watches for new data)
python auto_train.py
```

## Service Management (Production)

```bash
# Start all services
sudo systemctl start btc-arb-bot btc-ml-trainer btc-ml-predictor

# View logs
journalctl -u btc-arb-bot -f
journalctl -u btc-ml-trainer -f
journalctl -u btc-ml-predictor -f
```

## Architecture

### Core Flow
1. `main.rs` runs the trading loop - polls for BTC 15-min markets, connects per-market WebSocket
2. `market.rs` discovers active markets via gamma-api REST polling
3. `websocket.rs` connects to `wss://ws-subscriptions-clob.polymarket.com` for real-time orderbook
4. `orderbook.rs` maintains local orderbook state from WebSocket deltas
5. `strategy.rs` generates ladder orders across price levels, submits via CLOB API
6. `position.rs` tracks fills and calculates P&L

### Order Signing (Two Layers)
- **API auth** (`auth.rs`): HMAC-SHA256 for CLOB API authentication (headers: POLY-SIGNATURE, POLY-TIMESTAMP, etc.)
- **Order signing** (`signer.rs`): EIP-712 typed data signing with wallet private key for order placement

### Strategies (`src/strategies/`)
- `pure_arb.rs` - Buy both sides, hold to resolution (safest)
- `scalper.rs` - Take profit on price movements
- `market_maker.rs` - Post buy/sell orders, capture spread
- `momentum.rs` - Add to position when trend detected
- `hybrid.rs` - Combined approach

### ML Integration
- `ml_client.rs` connects to Python prediction server on port 8765
- Models predict: entry timing, fill probability, spread changes
- Data logged to `./data/` as JSONL files for training

## API Endpoints

- `clob.polymarket.com` - Order placement, orderbook
- `data-api.polymarket.com` - Trade history, positions
- `gamma-api.polymarket.com` - Market discovery
- `wss://ws-subscriptions-clob.polymarket.com/ws/` - WebSocket feeds

## Key Configuration (.env)

```
POLY_API_KEY, POLY_API_SECRET, POLY_API_PASSPHRASE  # API auth
POLY_ADDRESS, PRIVATE_KEY                            # Wallet
MAX_POSITION_USD, TARGET_SPREAD_PERCENT             # Trading params
LADDER_LEVELS, ORDER_SIZE_PER_LEVEL                 # Ladder config
DRY_RUN=true                                        # Enable for testing
```

## Performance Considerations

- Bot uses `tcp_nodelay`, connection pooling, pre-warmed connections
- `presigned_cache.rs` allows pre-computing order signatures
- Parallel order submission via `tokio::join!`
- Recommended deployment: AWS EC2 c6i.large in us-east-1 for lowest latency
