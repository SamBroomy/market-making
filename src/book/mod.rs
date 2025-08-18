use rust_decimal::Decimal;

pub type Price = Decimal;
pub type Size = Decimal;
pub type Volume = Decimal;

pub mod book_state;

pub mod half_book;
pub mod order_book;

// Re-export commonly used types
pub use book_state::{OrderBookState, ProcessResult};
pub use order_book::{OrderBook, SnapshotReason, SnapshotRequest};

// Type aliases for convenience
pub type SnapshotRequestSender = tokio::sync::mpsc::UnboundedSender<SnapshotRequest>;
pub type SnapshotRequestReceiver = tokio::sync::mpsc::UnboundedReceiver<SnapshotRequest>;
