BEGIN;

SET timescaledb.enable_direct_compress_copy = on;

-- Depth Updates (streaming order book changes)
CREATE TABLE depth_updates (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    first_update_id NUMERIC NOT NULL,
    final_update_id NUMERIC NOT NULL,
    bids JSONB, -- Array of objects with "size" and "price" fields
    asks JSONB -- Array of objects with "size" and "price" fields
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

-- Add compression policies (compress data older than 1 day for high-frequency tables)
SELECT add_compression_policy('depth_updates', INTERVAL '1d');

-- Ticker Data (24hr statistics)
CREATE TABLE ticker_data (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    price_change DECIMAL(20, 8) NOT NULL,
    price_change_percent DECIMAL(10, 4) NOT NULL,
    weighted_avg_price DECIMAL(20, 8) NOT NULL,
    first_trade_price DECIMAL(20, 8) NOT NULL,
    last_price DECIMAL(20, 8) NOT NULL,
    last_quantity DECIMAL(20, 8) NOT NULL,
    best_bid_price DECIMAL(20, 8) NOT NULL,
    best_bid_quantity DECIMAL(20, 8) NOT NULL,
    best_ask_price DECIMAL(20, 8) NOT NULL,
    best_ask_quantity DECIMAL(20, 8) NOT NULL,
    open_price DECIMAL(20, 8) NOT NULL,
    high_price DECIMAL(20, 8) NOT NULL,
    low_price DECIMAL(20, 8) NOT NULL,
    volume DECIMAL(20, 8) NOT NULL,
    quote_volume DECIMAL(20, 8) NOT NULL,
    open_time TIMESTAMPTZ NOT NULL,
    close_time TIMESTAMPTZ NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_count NUMERIC NOT NULL
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('ticker_data', INTERVAL '2d');

-- Rolling Window Ticker (aggregated ticker data for different intervals)
CREATE TABLE rolling_window_ticker (
    event_type TEXT NOT NULL, -- "1hTicker", "4hTicker", etc.
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    price_change DECIMAL(20, 8) NOT NULL,
    price_change_percent DECIMAL(10, 4) NOT NULL,
    open_price DECIMAL(20, 8) NOT NULL,
    high_price DECIMAL(20, 8) NOT NULL,
    low_price DECIMAL(20, 8) NOT NULL,
    close_price DECIMAL(20, 8) NOT NULL,
    weighted_avg_price DECIMAL(20, 8) NOT NULL,
    volume DECIMAL(20, 8) NOT NULL,
    quote_volume DECIMAL(20, 8) NOT NULL,
    open_time TIMESTAMPTZ NOT NULL,
    close_time TIMESTAMPTZ NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_count NUMERIC NOT NULL
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol,event_type',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('rolling_window_ticker', INTERVAL '2d');

-- Aggregate Trades
CREATE TABLE aggregate_trades (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    aggregate_trade_id NUMERIC NOT NULL,
    price DECIMAL(20, 8) NOT NULL,
    quantity DECIMAL(20, 8) NOT NULL,
    first_trade_id NUMERIC NOT NULL,
    last_trade_id NUMERIC NOT NULL,
    trade_time TIMESTAMPTZ NOT NULL,
    buyer_market_maker BOOLEAN NOT NULL
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('aggregate_trades', INTERVAL '1d');

-- Depth Snapshots (full order book state)
CREATE TABLE depth_snapshots (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    last_update_id NUMERIC NOT NULL,
    bids JSONB NOT NULL, -- Array of objects with "size" and "price" fields
    asks JSONB NOT NULL, -- Array of objects with "size" and "price" fields
    snapshot_reason TEXT -- 'initial', 'periodic', 'gap_recovery'
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('depth_snapshots', INTERVAL '2d');

-- Orderbook State (periodic snapshots with configurable depth)
CREATE TABLE orderbook_state (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    bids JSONB NOT NULL, -- Array of objects with "size" and "price" fields
    asks JSONB NOT NULL, -- Array of objects with "size" and "price" fields
    last_update_id NUMERIC NOT NULL,
    depth_limit INTEGER NOT NULL -- Number of levels captured (e.g., 50, 100)
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('orderbook_state', INTERVAL '2d');

-- Orderbook Summary (computed metrics for trading algorithms and dashboards)
CREATE TABLE orderbook_summary (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    spread_bps DECIMAL(10, 4) NOT NULL,
    mid_price DECIMAL(20, 8) NOT NULL,
    bid_volume_l1 DECIMAL(20, 8) NOT NULL, -- Raw bid volume at best price
    ask_volume_l1 DECIMAL(20, 8) NOT NULL, -- Raw ask volume at best price
    quote_imbalance_l1 DECIMAL(10, 4) NOT NULL, -- Normalized [-1,1] -> [0,1]
    bid_volume_l5 DECIMAL(20, 8) NOT NULL, -- Total bid volume top 5 levels
    ask_volume_l5 DECIMAL(20, 8) NOT NULL, -- Total ask volume top 5 levels
    quote_imbalance_l5 DECIMAL(10, 4) NOT NULL, -- Normalized [-1,1] -> [0,1]
    -- Derived prices
    weighted_mid DECIMAL(20, 8) NOT NULL,
    micro_price DECIMAL(20, 8) NOT NULL, -- Stoikov's micro-price
    -- Metadata
    update_id NUMERIC NOT NULL
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);

SELECT add_compression_policy('orderbook_summary', INTERVAL '1d');



CREATE TABLE trade_summaries (
    event_time TIMESTAMPTZ NOT NULL,
    symbol TEXT NOT NULL,
    buy_volume DECIMAL(20, 8) NOT NULL,
    sell_volume DECIMAL(20, 8) NOT NULL,
    trade_count INTEGER NOT NULL,
    trade_intensity DECIMAL(10, 4) NOT NULL, -- trades/second
    imbalance DECIMAL(10, 4) NOT NULL,       -- Exponentially weighted
    volatility DECIMAL(20, 8) NOT NULL       -- Simple variance
)
WITH (
    tsdb.hypertable,
    tsdb.partition_column = 'event_time',
    tsdb.segmentby = 'symbol',
    tsdb.orderby = 'event_time DESC',
    tsdb.chunk_interval = '1d'
);
SELECT add_compression_policy('trade_summaries', INTERVAL '1d');

COMMIT;