# Order Flow Listener

Streams all Polymarket trades from Polygon blockchain and stores them in PostgreSQL.

## Architecture

```
Polygon RPC (WebSocket)
    ↓
CTF Exchange Events (OrderFilled, OrdersMatched)
    ↓
Parse & Store in PostgreSQL
    ↓
Trigger wallet stats update
```

## Setup

1. **Get Polygon RPC URL**:
   - Alchemy: https://alchemy.com (recommended)
   - Infura: https://infura.io
   - QuickNode: https://quicknode.com

2. **Set environment variables**:
   ```bash
   cp .env.example .env
   # Edit .env with your DATABASE_URL and POLYGON_RPC_URL
   ```

3. **Run database migration**:
   ```bash
   # From Railway dashboard or local psql
   psql $DATABASE_URL -f ../migrations/001_orderflow_schema.sql
   ```

4. **Run locally**:
   ```bash
   cargo run
   ```

5. **Deploy to Railway**:
   - Create new service from GitHub repo
   - Set root directory to `/orderflow-listener`
   - Add environment variables
   - Railway will auto-build using Dockerfile

## Monitoring

**Logs to watch for**:
- `✅ Connected to PostgreSQL` - Database connected
- `✅ Connected to Polygon` - WebSocket connected
- `📡 Listening for Polymarket trades...` - Subscribed to events
- `💸 Trade: ...` - New trade detected and saved
- `📊 Processed 100 trades` - Progress counter

**Expected metrics**:
- ~7-10 trades per minute during active hours
- ~3,000-5,000 trades per day
- Memory usage: 200-400 MB
- CPU usage: <10%

## Troubleshooting

**WebSocket disconnects**:
- Alchemy/Infura have connection limits
- Listener will auto-reconnect (add retry logic if needed)

**Duplicate trades**:
- Database has UNIQUE constraint on tx_hash
- `ON CONFLICT DO NOTHING` prevents duplicates

**Missing trades**:
- Check block number gaps in database
- Verify WebSocket is still connected
- Check Alchemy/Infura quota

## Database Tables

**orderflow_trades** - Every trade ever made
- Primary key: id
- Unique: tx_hash
- Indexed: wallet_address, market_id, timestamp

**orderflow_wallet_stats** - Aggregated per wallet
- Automatically updated via database trigger
- Recalculated hourly by reputation service
