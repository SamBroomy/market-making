BEGIN;

SET
    timescaledb.enable_direct_compress_copy = off;

DROP TABLE orderbook_summary;

DROP TABLE depth_updates;

DROP TABLE rolling_window_ticker;

DROP TABLE ticker_data;

DROP TABLE aggregate_trades;

DROP TABLE depth_snapshots;

DROP TABLE orderbook_state;

COMMIT;