SET timescaledb.enable_direct_compress_copy=on;

-- Depth Updates (streaming order book changes)
CREATE TABLE depth_updates (
    "event_time" TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    first_update_id NUMERIC NOT NULL,
    final_update_id NUMERIC NOT NULL,
    bids JSONB,
    asks JSONB
) WITH (
    tsdb.hypertable,
    tsdb.partition_column='event_time',
    tsdb.segmentby='symbol',
    tsdb.orderby='event_time DESC',
    tsdb.chunk_interval='1d'
);

-- Ticker Data (24hr statistics)
CREATE TABLE ticker_data (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    price_change DECIMAL(20,8) NOT NULL,
    price_change_percent DECIMAL(10,4) NOT NULL,
    weighted_avg_price DECIMAL(20,8) NOT NULL,
    first_trade_price DECIMAL(20,8) NOT NULL,
    last_price DECIMAL(20,8) NOT NULL,
    last_quantity DECIMAL(20,8) NOT NULL,
    best_bid_price DECIMAL(20,8) NOT NULL,
    best_bid_quantity DECIMAL(20,8) NOT NULL,
    best_ask_price DECIMAL(20,8) NOT NULL,
    best_ask_quantity DECIMAL(20,8) NOT NULL,
    open_price DECIMAL(20,8) NOT NULL,
    high_price DECIMAL(20,8) NOT NULL,
    low_price DECIMAL(20,8) NOT NULL,
    volume DECIMAL(20,8) NOT NULL,
    quote_volume DECIMAL(20,8) NOT NULL,
    open_time TIMESTAMPTZ NOT NULL,
    close_time TIMESTAMPTZ NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_count NUMERIC NOT NULL
) WITH (
    tsdb.hypertable,
    tsdb.partition_column='event_time',
    tsdb.segmentby='symbol',
    tsdb.orderby='event_time DESC',
    tsdb.chunk_interval='1d'
);


-- Rolling Window Ticker (aggregated ticker data for different intervals)
CREATE TABLE rolling_window_ticker (
    event_type TEXT NOT NULL, -- "1hTicker", "4hTicker", etc.
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    price_change DECIMAL(20,8) NOT NULL,
    price_change_percent DECIMAL(10,4) NOT NULL,
    open_price DECIMAL(20,8) NOT NULL,
    high_price DECIMAL(20,8) NOT NULL,
    low_price DECIMAL(20,8) NOT NULL,
    close_price DECIMAL(20,8) NOT NULL,
    weighted_avg_price DECIMAL(20,8) NOT NULL,
    volume DECIMAL(20,8) NOT NULL,
    quote_volume DECIMAL(20,8) NOT NULL,
    open_time TIMESTAMPTZ NOT NULL,
    close_time TIMESTAMPTZ NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_count NUMERIC NOT NULL
) WITH (
    tsdb.hypertable,
    tsdb.partition_column='event_time',
    tsdb.segmentby='symbol,event_type',
    tsdb.orderby='event_time DESC',
    tsdb.chunk_interval='1d'
);

CREATE TABLE aggregate_trades (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    aggregate_trade_id NUMERIC NOT NULL,
    price DECIMAL(20,8) NOT NULL,
    quantity DECIMAL(20,8) NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_time TIMESTAMPTZ NOT NULL,
    buyer_market_maker BOOLEAN NOT NULL
) WITH (
    tsdb.hypertable,
    tsdb.partition_column='event_time',
    tsdb.segmentby='symbol',
    tsdb.orderby='event_time DESC',
    tsdb.chunk_interval='1d'
);

-- Depth Snapshots (full order book state)
CREATE TABLE depth_snapshots (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    last_update_id NUMERIC NOT NULL,
    bids JSONB NOT NULL,
    asks JSONB NOT NULL,
    snapshot_reason TEXT -- 'initial', 'periodic', 'gap_recovery'
) WITH (
   tsdb.hypertable,
   tsdb.partition_column='event_time',
   tsdb.segmentby='symbol',
   tsdb.orderby='event_time DESC'
);

-- Crypto assets reference table
CREATE TABLE crypto_assets (
    symbol TEXT PRIMARY KEY,
    "name" TEXT,
    base_asset TEXT,
    quote_asset TEXT,
    status TEXT,
    created_at TIMESTAMPTZ DEFAULT NOW()
);

-- Add compression policies (compress data older than 7 days)
-- CALL add_columnstore_policy('conditions', after => INTERVAL '1d');


-- Add retention policies (keep data for 1 year)
-- SELECT add_retention_policy('depth_updates', drop_after => INTERVAL '1 year');
