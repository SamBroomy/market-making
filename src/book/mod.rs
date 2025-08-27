use rust_decimal::Decimal;

pub type Price = Decimal;
pub type Size = Decimal;
pub type Volume = Decimal;

pub mod order_book;

pub mod half_book;
pub mod order_book_processor;

// Re-export commonly used types
pub use order_book::{OrderBook, ProcessResult};
pub use order_book_processor::{OrderBookProcessor, SnapshotReason, SnapshotRequest};

// Type aliases for convenience
pub type SnapshotRequestSender = tokio::sync::mpsc::UnboundedSender<SnapshotRequest>;
pub type SnapshotRequestReceiver = tokio::sync::mpsc::UnboundedReceiver<SnapshotRequest>;
