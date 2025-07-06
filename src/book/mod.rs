use rust_decimal::Decimal;

pub type Price = Decimal;
pub type Size = Decimal;
pub type Volume = Decimal;

mod book_state;
mod half_book;
mod order_book;

pub use order_book::{OrderBook, SnapshotRequest};
