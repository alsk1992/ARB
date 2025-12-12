-- Order Flow Trading - Database Schema
-- Add to existing PolyTrack PostgreSQL database

-- ===========================================
-- TRADES TABLE - All on-chain Polymarket trades
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_trades (
    id BIGSERIAL PRIMARY KEY,

    -- Transaction data
    tx_hash VARCHAR(66) UNIQUE NOT NULL,
    block_number BIGINT NOT NULL,
    timestamp TIMESTAMPTZ NOT NULL,

    -- Trader info
    wallet_address VARCHAR(42) NOT NULL,
    is_maker BOOLEAN NOT NULL,

    -- Market info
    market_id VARCHAR(66) NOT NULL,
    market_title TEXT,
    token_id VARCHAR(78) NOT NULL,
    outcome VARCHAR(10), -- UP/DOWN or YES/NO

    -- Trade details
    side VARCHAR(10) NOT NULL, -- BUY/SELL
    price NUMERIC(20, 18) NOT NULL,
    size NUMERIC(30, 18) NOT NULL,
    value_usd NUMERIC(20, 2),

    -- Order data
    order_hash VARCHAR(66),
    fee_paid NUMERIC(20, 18),
    gas_price NUMERIC(20, 0),

    created_at TIMESTAMPTZ DEFAULT NOW(),

    INDEX idx_wallet (wallet_address),
    INDEX idx_market (market_id),
    INDEX idx_timestamp (timestamp DESC),
    INDEX idx_tx_hash (tx_hash)
);

-- ===========================================
-- WALLET STATS - Aggregated performance metrics
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_wallet_stats (
    wallet_address VARCHAR(42) PRIMARY KEY,

    -- Basic stats
    first_trade_at TIMESTAMPTZ,
    last_trade_at TIMESTAMPTZ,
    total_trades INTEGER DEFAULT 0,
    total_volume_usd NUMERIC(20, 2) DEFAULT 0,

    -- Performance metrics
    winning_trades INTEGER DEFAULT 0,
    losing_trades INTEGER DEFAULT 0,
    win_rate NUMERIC(5, 4), -- 0.0000 to 1.0000
    total_pnl_usd NUMERIC(20, 2) DEFAULT 0,
    avg_profit_per_trade_pct NUMERIC(8, 4),

    -- Behavioral patterns
    avg_position_size_usd NUMERIC(20, 2),
    avg_entry_minute NUMERIC(5, 2), -- 0-15 for 15min markets
    avg_hold_duration_minutes NUMERIC(10, 2),

    -- Market preferences
    favorite_categories JSONB, -- ["Crypto", "Politics", ...]
    crypto_win_rate NUMERIC(5, 4),
    politics_win_rate NUMERIC(5, 4),
    sports_win_rate NUMERIC(5, 4),

    -- Risk metrics
    sharpe_ratio NUMERIC(8, 4),
    max_drawdown_pct NUMERIC(8, 4),
    volatility NUMERIC(8, 4),

    -- Reputation scoring
    reputation_score NUMERIC(4, 2), -- 0.00 to 10.00
    confidence_level NUMERIC(3, 2), -- 0.00 to 1.00 (how confident we are in score)
    trader_tier VARCHAR(20), -- WHALE / SMART / AVERAGE / NOVICE / DEGEN

    -- Meta
    last_calculated_at TIMESTAMPTZ,
    calculation_version INTEGER DEFAULT 1,

    created_at TIMESTAMPTZ DEFAULT NOW(),
    updated_at TIMESTAMPTZ DEFAULT NOW()
);

-- ===========================================
-- MARKET OUTCOMES - Resolution data
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_market_outcomes (
    market_id VARCHAR(66) PRIMARY KEY,
    title TEXT NOT NULL,
    category VARCHAR(100),

    -- Outcome data
    winning_outcome VARCHAR(10), -- UP/DOWN/YES/NO
    winning_token_id VARCHAR(78),
    resolved_at TIMESTAMPTZ,
    final_price_up NUMERIC(20, 18),
    final_price_down NUMERIC(20, 18),

    -- Market metadata
    created_at_timestamp TIMESTAMPTZ,
    ends_at_timestamp TIMESTAMPTZ,
    total_volume_usd NUMERIC(20, 2),
    unique_traders INTEGER,

    created_at TIMESTAMPTZ DEFAULT NOW(),

    INDEX idx_resolved (resolved_at),
    INDEX idx_category (category)
);

-- ===========================================
-- SIGNALS - Generated trading signals
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_signals (
    id BIGSERIAL PRIMARY KEY,

    -- Signal source
    trigger_wallet VARCHAR(42) NOT NULL,
    trigger_tx_hash VARCHAR(66),
    wallet_score NUMERIC(4, 2),
    trader_tier VARCHAR(20),

    -- Signal details
    signal_type VARCHAR(20) NOT NULL, -- FOLLOW_WHALE / FADE_DEGEN / COUNTER_PANIC
    action VARCHAR(10) NOT NULL, -- BUY / SELL
    market_id VARCHAR(66) NOT NULL,
    market_title TEXT,
    outcome VARCHAR(10) NOT NULL,

    -- Confidence & sizing
    confidence NUMERIC(3, 2) NOT NULL, -- 0.00 to 1.00
    recommended_size_usd NUMERIC(20, 2),
    max_price NUMERIC(20, 18), -- Don't buy above this

    -- Execution tracking
    status VARCHAR(20) DEFAULT 'PENDING', -- PENDING / EXECUTED / SKIPPED / EXPIRED
    executed_at TIMESTAMPTZ,
    executed_price NUMERIC(20, 18),
    executed_size NUMERIC(30, 18),
    executed_tx_hash VARCHAR(66),

    -- Outcome tracking
    outcome_status VARCHAR(20), -- WIN / LOSS / PENDING
    profit_loss_usd NUMERIC(20, 2),
    profit_loss_pct NUMERIC(8, 4),
    closed_at TIMESTAMPTZ,

    -- Meta
    created_at TIMESTAMPTZ DEFAULT NOW(),
    expires_at TIMESTAMPTZ,

    INDEX idx_status (status),
    INDEX idx_created (created_at DESC),
    INDEX idx_market (market_id)
);

-- ===========================================
-- REPUTATION HISTORY - Track score changes over time
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_reputation_history (
    id BIGSERIAL PRIMARY KEY,
    wallet_address VARCHAR(42) NOT NULL,

    score NUMERIC(4, 2) NOT NULL,
    tier VARCHAR(20) NOT NULL,
    win_rate NUMERIC(5, 4),
    total_trades INTEGER,
    total_pnl_usd NUMERIC(20, 2),

    calculated_at TIMESTAMPTZ DEFAULT NOW(),

    INDEX idx_wallet_time (wallet_address, calculated_at DESC)
);

-- ===========================================
-- LIVE POSITIONS - Currently open positions
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_positions (
    id BIGSERIAL PRIMARY KEY,

    signal_id BIGINT REFERENCES orderflow_signals(id),

    market_id VARCHAR(66) NOT NULL,
    token_id VARCHAR(78) NOT NULL,
    outcome VARCHAR(10) NOT NULL,

    entry_price NUMERIC(20, 18) NOT NULL,
    size NUMERIC(30, 18) NOT NULL,
    value_usd NUMERIC(20, 2) NOT NULL,

    current_price NUMERIC(20, 18),
    unrealized_pnl_usd NUMERIC(20, 2),

    status VARCHAR(20) DEFAULT 'OPEN', -- OPEN / CLOSED

    opened_at TIMESTAMPTZ DEFAULT NOW(),
    closed_at TIMESTAMPTZ,

    INDEX idx_status (status),
    INDEX idx_market (market_id)
);

-- ===========================================
-- PERFORMANCE METRICS - System-wide stats
-- ===========================================
CREATE TABLE IF NOT EXISTS orderflow_performance (
    id BIGSERIAL PRIMARY KEY,

    date DATE NOT NULL UNIQUE,

    -- Signal stats
    signals_generated INTEGER DEFAULT 0,
    signals_executed INTEGER DEFAULT 0,
    signals_skipped INTEGER DEFAULT 0,

    -- Trade stats
    total_trades INTEGER DEFAULT 0,
    winning_trades INTEGER DEFAULT 0,
    losing_trades INTEGER DEFAULT 0,
    win_rate NUMERIC(5, 4),

    -- P&L
    gross_pnl_usd NUMERIC(20, 2) DEFAULT 0,
    fees_paid_usd NUMERIC(20, 2) DEFAULT 0,
    net_pnl_usd NUMERIC(20, 2) DEFAULT 0,

    -- Risk metrics
    sharpe_ratio NUMERIC(8, 4),
    max_drawdown_pct NUMERIC(8, 4),

    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- ===========================================
-- VIEWS - Useful queries
-- ===========================================

-- Top performing wallets (last 30 days)
CREATE OR REPLACE VIEW orderflow_top_wallets AS
SELECT
    w.wallet_address,
    w.reputation_score,
    w.trader_tier,
    w.win_rate,
    w.total_trades,
    w.total_pnl_usd,
    w.avg_profit_per_trade_pct,
    COUNT(DISTINCT t.market_id) as markets_traded,
    MAX(t.timestamp) as last_trade
FROM orderflow_wallet_stats w
LEFT JOIN orderflow_trades t ON t.wallet_address = w.wallet_address
WHERE t.timestamp > NOW() - INTERVAL '30 days'
GROUP BY w.wallet_address, w.reputation_score, w.trader_tier, w.win_rate, w.total_trades, w.total_pnl_usd, w.avg_profit_per_trade_pct
ORDER BY w.reputation_score DESC
LIMIT 100;

-- Recent high-confidence signals
CREATE OR REPLACE VIEW orderflow_hot_signals AS
SELECT
    s.id,
    s.signal_type,
    s.market_title,
    s.outcome,
    s.confidence,
    s.wallet_score,
    s.status,
    s.created_at,
    w.trader_tier
FROM orderflow_signals s
JOIN orderflow_wallet_stats w ON s.trigger_wallet = w.wallet_address
WHERE s.created_at > NOW() - INTERVAL '1 hour'
AND s.confidence > 0.7
ORDER BY s.created_at DESC;

-- ===========================================
-- FUNCTIONS - Utility functions
-- ===========================================

-- Update wallet stats after new trade
CREATE OR REPLACE FUNCTION update_wallet_stats()
RETURNS TRIGGER AS $$
BEGIN
    INSERT INTO orderflow_wallet_stats (wallet_address, total_trades, last_trade_at)
    VALUES (NEW.wallet_address, 1, NEW.timestamp)
    ON CONFLICT (wallet_address)
    DO UPDATE SET
        total_trades = orderflow_wallet_stats.total_trades + 1,
        last_trade_at = NEW.timestamp,
        updated_at = NOW();

    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Trigger to auto-update wallet stats
CREATE TRIGGER trg_update_wallet_stats
AFTER INSERT ON orderflow_trades
FOR EACH ROW
EXECUTE FUNCTION update_wallet_stats();

-- ===========================================
-- INDEXES for performance
-- ===========================================

-- Composite indexes for common queries
CREATE INDEX IF NOT EXISTS idx_trades_wallet_timestamp
ON orderflow_trades(wallet_address, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_trades_market_timestamp
ON orderflow_trades(market_id, timestamp DESC);

CREATE INDEX IF NOT EXISTS idx_signals_confidence
ON orderflow_signals(confidence DESC, created_at DESC)
WHERE status = 'PENDING';

-- ===========================================
-- SEED DATA (optional)
-- ===========================================

-- Insert example tiers
INSERT INTO orderflow_wallet_stats (
    wallet_address, trader_tier, reputation_score, confidence_level
) VALUES
    ('0x0000000000000000000000000000000000000000', 'EXAMPLE', 5.0, 0.0)
ON CONFLICT DO NOTHING;

COMMENT ON TABLE orderflow_trades IS 'All on-chain Polymarket trades streamed from Polygon';
COMMENT ON TABLE orderflow_wallet_stats IS 'Aggregated performance metrics for each wallet';
COMMENT ON TABLE orderflow_signals IS 'Trading signals generated by order flow analysis';
COMMENT ON COLUMN orderflow_wallet_stats.reputation_score IS 'Score 0-10 where 10=best trader, calculated from win rate, profit, consistency';
COMMENT ON COLUMN orderflow_signals.confidence IS 'Signal confidence 0.0-1.0, used for position sizing via Kelly criterion';
