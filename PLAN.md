# BTC 15-Min Arbitrage Bot - Ultimate Plan

## Goal
Build the fastest, most reliable Rust bot for BTC 15-min arbitrage on Polymarket.

---

## Strategy (Confirmed from Pro Analysis)

### Primary: Limit Order Ladder
1. Post limit BUY orders on BOTH Up and Down at prices where combined < $1
2. Ladder orders across multiple price levels (e.g., 20¢, 25¢, 30¢, 35¢, 40¢, 45¢)
3. As market swings, orders fill on both sides
4. Hold to resolution → one side pays $1 → guaranteed profit

### Secondary: Spread Sniping
1. Monitor orderbook in real-time via WebSocket
2. When best_ask(UP) + best_ask(DOWN) < $0.98 → instant market buy both
3. Requires sub-100ms execution

---

## Infrastructure

### Server Location
- Polymarket CLOB is behind Cloudflare (anycast)
- Polygon RPC nodes are globally distributed
- **Recommended: AWS us-east-1 (N. Virginia)** or **eu-west-1 (Ireland)**
- Most US-based crypto infra is in us-east-1
- Cloudflare has edge nodes everywhere, so latency will be similar

### Server Specs
- **Provider**: AWS EC2 or Hetzner (cheaper, good perf)
- **Instance**: c6i.large or equivalent (compute optimized)
- **OS**: Ubuntu 22.04
- **Network**: Enhanced networking enabled

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        BTC ARB BOT                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────────┐ │
│  │   Market    │  │  Orderbook  │  │      Order Manager      │ │
│  │   Monitor   │  │   Stream    │  │                         │ │
│  │             │  │  (WebSocket)│  │  - Ladder posting       │ │
│  │ - Find new  │  │             │  │  - Fill tracking        │ │
│  │   15m mkts  │  │ - Real-time │  │  - Position balancing   │ │
│  │ - Track     │  │   UP/DOWN   │  │  - Cancel/replace       │ │
│  │   resolution│  │   prices    │  │                         │ │
│  └──────┬──────┘  └──────┬──────┘  └───────────┬─────────────┘ │
│         │                │                      │               │
│         └────────────────┼──────────────────────┘               │
│                          │                                      │
│                          ▼                                      │
│                 ┌─────────────────┐                            │
│                 │  Signal Engine  │                            │
│                 │                 │                            │
│                 │ - Spread calc   │                            │
│                 │ - Entry signals │                            │
│                 │ - Risk checks   │                            │
│                 └────────┬────────┘                            │
│                          │                                      │
│                          ▼                                      │
│                 ┌─────────────────┐                            │
│                 │    Executor     │                            │
│                 │                 │                            │
│                 │ - HMAC signing  │                            │
│                 │ - Parallel POST │                            │
│                 │ - Retry logic   │                            │
│                 └────────┬────────┘                            │
│                          │                                      │
│                          ▼                                      │
│                 ┌─────────────────┐                            │
│                 │   CLOB API      │                            │
│                 │ (Polymarket)    │                            │
│                 └─────────────────┘                            │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## Rust Crates

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` |
| HTTP client | `reqwest` |
| WebSocket | `tokio-tungstenite` |
| JSON | `serde`, `serde_json` |
| HMAC signing | `hmac`, `sha2` |
| Base64 | `base64` |
| Crypto (EIP-712) | `ethers` |
| Logging | `tracing` |
| Config | `config` or `dotenvy` |
| Decimal math | `rust_decimal` |

---

## Key Components

### 1. Market Monitor
- Poll gamma-api every 5 seconds for new BTC 15-min markets
- Track: token IDs, start time, end time, current prices
- Identify upcoming markets to post orders on

### 2. Orderbook Stream
- WebSocket connection to CLOB for real-time orderbook
- Track best bid/ask for both UP and DOWN tokens
- Calculate spread continuously

### 3. Order Manager
- Post ladder of limit orders on both sides
- Track which orders are filled
- Rebalance if one side fills more than other
- Cancel unfilled orders before market resolution

### 4. Executor
- HMAC-SHA256 signing for CLOB authentication
- EIP-712 signing for order placement
- Parallel order submission (both sides simultaneously)
- Connection pooling for fastest requests

### 5. Risk Manager
- Max position size per market
- Max daily loss limit
- Don't enter if spread too tight
- Emergency stop capability

---

## Order Signing (Critical)

Polymarket uses two layers of auth:

### 1. API Authentication (HMAC)
```
Headers:
- POLY-ADDRESS: 0x...
- POLY-API-KEY: key
- POLY-PASSPHRASE: passphrase
- POLY-TIMESTAMP: unix_seconds
- POLY-SIGNATURE: HMAC-SHA256(timestamp + method + path + body)
```

### 2. Order Signing (EIP-712)
Orders must be signed with wallet private key using EIP-712 typed data.

Structure:
```
Order {
  salt: random,
  maker: address,
  signer: address,
  taker: 0x0000...,
  tokenId: token_id,
  makerAmount: shares,
  takerAmount: cost,
  expiration: timestamp,
  nonce: 0,
  feeRateBps: 0,
  side: BUY,
  signatureType: EOA (0) or POLY_PROXY (1)
}
```

---

## Speed Optimizations

1. **Connection pooling** - Reuse HTTP connections
2. **Pre-computed signatures** - Have order templates ready, just fill in price/size
3. **Parallel requests** - Submit UP and DOWN orders simultaneously via tokio::join!
4. **Local order book** - Maintain copy, don't re-fetch
5. **Binary protocol** - Use msgpack if available (check CLOB docs)
6. **TCP tuning** - TCP_NODELAY, keep-alive

---

## File Structure

```
btc-arb-bot/
├── Cargo.toml
├── .env
├── src/
│   ├── main.rs           # Entry point, orchestration
│   ├── config.rs         # Configuration loading
│   ├── auth.rs           # HMAC + EIP-712 signing
│   ├── clob.rs           # CLOB API client
│   ├── market.rs         # Market discovery + tracking
│   ├── orderbook.rs      # WebSocket orderbook stream
│   ├── strategy.rs       # Ladder + spread detection
│   ├── executor.rs       # Order submission
│   ├── position.rs       # Position tracking
│   └── risk.rs           # Risk management
└── tests/
    └── integration.rs    # Live market tests
```

---

## Trading Parameters

| Parameter | Value | Notes |
|-----------|-------|-------|
| Capital | $1,270 | £1k starting |
| Max per market | $500 | Don't risk all in one market |
| Ladder levels | 6 | Orders at different prices |
| Price spacing | 5¢ | Between ladder levels |
| Target spread | 3%+ | Combined price < 97¢ |
| Min spread | 2% | Below this, don't enter |
| Order size | ~$40/level | $500 / 6 levels / 2 sides |

---

## Execution Flow

```
1. Bot starts
   └─> Connect WebSocket to CLOB
   └─> Poll for active BTC 15-min market

2. New market found
   └─> Get token IDs for UP and DOWN
   └─> Subscribe to orderbook updates

3. Post ladder orders
   └─> Calculate 6 price levels per side
   └─> Sign all 12 orders (EIP-712)
   └─> Submit in parallel

4. Monitor fills
   └─> Track filled shares per side
   └─> If imbalanced, adjust remaining orders

5. Pre-resolution (2 min before)
   └─> Cancel any unfilled orders
   └─> Log final position

6. Resolution
   └─> One side pays $1
   └─> Calculate P&L
   └─> Log results

7. Repeat for next market
```

---

## Deployment

### Initial (Testing)
- Run locally with DRY_RUN=true
- Paper trade to verify logic

### Production
1. Provision AWS EC2 in us-east-1
2. Install Rust, clone repo, build release
3. Set up systemd service for auto-restart
4. Configure monitoring (Prometheus + Grafana or simple Discord alerts)
5. Start with small position sizes
6. Scale up as proven profitable

---

## Answers (Resolved)

1. **WebSocket**: YES - `wss://` endpoint with real-time data
   - Topic: `clob_market`, Type: `agg_orderbook` - Real-time aggregated orderbook
   - Topic: `clob_market`, Type: `price_change` - Real-time price updates
   - Topic: `crypto_prices_chainlink`, Type: `update` - BTC price from Chainlink
   - Topic: `clob_user`, Type: `order` - Your order updates (needs auth)
   - Topic: `clob_user`, Type: `trade` - Your trade fills (needs auth)

2. **Rate limits**: Not explicitly documented, but libraries mention built-in handling

3. **Minimum order size**: Available in orderbook snapshot as `min_order_size`

4. **Server location**: Behind Cloudflare (anycast), origin likely us-east-1

---

## WebSocket Endpoints

**Base URL**: `wss://ws-subscriptions-clob.polymarket.com/ws/`

### Subscribe to orderbook:
```json
{
  "type": "subscribe",
  "subscriptions": [{
    "topic": "clob_market",
    "type": "agg_orderbook",
    "filters": ["<token_id_1>", "<token_id_2>"]
  }]
}
```

### Subscribe to BTC price (Chainlink):
```json
{
  "type": "subscribe",
  "subscriptions": [{
    "topic": "crypto_prices_chainlink",
    "type": "update",
    "filters": "{\"symbol\":\"BTCUSDT\"}"
  }]
}
```

### Subscribe to your fills (authenticated):
```json
{
  "type": "subscribe",
  "subscriptions": [{
    "topic": "clob_user",
    "type": "*",
    "clob_auth": {
      "key": "xxx",
      "secret": "xxx",
      "passphrase": "xxx"
    }
  }]
}
```

---

## Critical Speed Optimizations for Rust

1. **tokio-tungstenite** - Async WebSocket, zero-copy where possible
2. **Connection pooling** - `reqwest` with keep-alive for HTTP
3. **Pre-built order templates** - Only fill in price/size at runtime
4. **Parallel order submission** - `tokio::join!` for UP + DOWN
5. **Local orderbook cache** - Don't re-fetch, update from WebSocket deltas
6. **TCP_NODELAY** - Disable Nagle's algorithm for low latency
7. **Direct struct deserialization** - `serde` with `#[serde(rename)]` for compact fields

---

## Next Steps

1. ✅ Plan complete
2. [ ] Set up Rust project with dependencies
3. [ ] Implement HMAC auth
4. [ ] Implement EIP-712 order signing
5. [ ] Build WebSocket orderbook stream
6. [ ] Build market monitor (find BTC 15m markets)
7. [ ] Build ladder order strategy
8. [ ] Build fill tracker
9. [ ] Test on live market with small size
10. [ ] Deploy to AWS us-east-1
