use crate::binance::data::{DepthSnapshot, DepthUpdate, OfferData};
use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use std::collections::{BTreeMap, VecDeque};
use tracing::{debug, info, warn};

type Price = Decimal;
type Size = Decimal;

#[derive(Debug, Clone, Default)]
pub struct OrderBookState {
    bids: BTreeMap<Price, Size>,
    asks: BTreeMap<Price, Size>,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,
}

impl OrderBookState {
    pub fn apply_snapshot(&mut self, snapshot: DepthSnapshot) {
        info!(
            "Applying snaphot with last_update_id: {}",
            snapshot.last_update_id
        );

        self.bids.clear();
        self.asks.clear();

        for OfferData { price, size } in snapshot.bids {
            if size > Decimal::ZERO {
                self.bids.insert(price, size);
            }
        }

        for OfferData { price, size } in snapshot.asks {
            if size > Decimal::ZERO {
                self.asks.insert(price, size);
            }
        }

        self.last_update_id = snapshot.last_update_id;
        self.last_update_time = Utc::now();
        info!(
            "Local orderbook state initialized with last_update_id: {}",
            self.last_update_id
        );
    }

    pub fn process_update(&mut self, update: DepthUpdate) -> Result<()> {
        debug!(
            "Processing update: [{}-{}]",
            update.first_update_id, update.final_update_id
        );
        if update.final_update_id <= self.last_update_id {
            debug!("Ignoring old update");
            return Ok(()); // Silently ignore old updates
        }
        if update.first_update_id > self.last_update_id + 1 {
            return Err(anyhow::Error::msg(format!(
                "Update sequence gap detected. Local: {}, Update: [{}, {}]",
                self.last_update_id, update.first_update_id, update.final_update_id
            )));
        }

        self.apply_update_changes(update)
    }

    pub fn process_buffer(&mut self, mut buffer: VecDeque<DepthUpdate>) -> Result<()> {
        let buffer_size = buffer.len();
        info!("Processing {} buffered updates", buffer_size);

        while let Some(update) = buffer.pop_front() {
            if update.final_update_id <= self.last_update_id {
                debug!("Ignoring old update: {}", update.final_update_id);
                continue;
            }
            if update.first_update_id <= self.last_update_id + 1 {
                self.apply_update_changes(update)?;
            } else {
                warn!(
                    "Out of sequence update during initial buffering: {}",
                    update.final_update_id
                );
                return Err(anyhow::Error::msg(
                    "Out of sequence update during initial buffering",
                ));
            }
        }
        Ok(())
    }

    fn apply_update_changes(&mut self, update: DepthUpdate) -> Result<()> {
        for &OfferData { price, size } in &update.bids {
            if size > Decimal::ZERO {
                match self.bids.insert(price, size) {
                    Some(existing_size) => {
                        if existing_size != size {
                            debug!(
                                "Updated bid price: {} from {} to {} diff: {}",
                                price,
                                existing_size,
                                size,
                                existing_size - size
                            );
                        } else {
                            debug!("Bid price: {} size unchanged: {}", price, size);
                        }
                    }
                    None => {
                        debug!("New bid price: {} with size: {}", price, size);
                    }
                }
            } else {
                match self.bids.remove(&price) {
                    Some(existing_size) => {
                        debug!("Removed bid price: {} with size: {}", price, existing_size);
                    }
                    None => {
                        debug!("Ignoring zero size bid price: {}", price);
                    }
                }
            }
        }

        for &OfferData { price, size } in &update.asks {
            if size > Decimal::ZERO {
                match self.asks.insert(price, size) {
                    Some(existing_size) => {
                        if existing_size != size {
                            debug!(
                                "Updated ask price: {} from {} to {} diff: {}",
                                price,
                                existing_size,
                                size,
                                existing_size - size
                            );
                        } else {
                            debug!("Ask price: {} size unchanged: {}", price, size);
                        }
                    }
                    None => {
                        debug!("New ask price: {} with size: {}", price, size);
                    }
                }
            } else {
                match self.asks.remove(&price) {
                    Some(existing_size) => {
                        debug!("Removed ask price: {} with size: {}", price, existing_size);
                    }
                    None => {
                        debug!("Ignoring zero size ask price: {}", price);
                    }
                }
            }
        }

        debug!(
            "Update applied successfully, new last_update_id: {}",
            update.final_update_id
        );
        self.last_update_id = update.final_update_id;
        self.last_update_time = update.event_time;
        Ok(())
    }

    pub fn spread(&self) -> Option<Decimal> {
        let top_bid = self.bids.last_key_value()?.0;
        let top_ask = self.asks.first_key_value()?.0;
        Some(top_ask - top_bid)
    }

    pub fn mid_price(&self) -> Option<Decimal> {
        let top_bid = self.bids.last_key_value()?.0;
        let top_ask = self.asks.first_key_value()?.0;
        Some((top_bid + top_ask) / Decimal::from(2))
    }

    /// Vbid−Vask/Vbid+Vask
    /// Positive values indicate a buy imbalance, while negative values indicate a sell imbalance.
    pub fn imbalance(&self) -> Option<Decimal> {
        let top_bid_volume = self.bids.last_key_value()?.1;
        let top_ask_volume = self.asks.first_key_value()?.1;

        Some((top_bid_volume - top_ask_volume) / (top_bid_volume + top_ask_volume))
    }

    pub fn imbalance_depth(&self, depth: impl Into<usize>) -> Option<Decimal> {
        let depth = depth.into();

        let bids = self.bids.values().rev().take(depth).sum::<Decimal>();

        let asks = self.asks.values().take(depth).sum::<Decimal>();

        Some((bids - asks) / (bids + asks))
    }

    pub fn best_bid(&self) -> Option<Decimal> {
        self.bids.last_key_value().map(|(&k, _)| k)
    }
    pub fn best_ask(&self) -> Option<Decimal> {
        self.asks.first_key_value().map(|(&k, _)| k)
    }
}
