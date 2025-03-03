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
    pub bids: BTreeMap<Price, Size>,
    pub asks: BTreeMap<Price, Size>,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,
    pub spread: Option<Decimal>,
    pub relative_spread: Option<Decimal>,
    pub mid_price: Option<Decimal>,
    pub imbalance: Option<Decimal>,
    pub weighted_imbalance: Option<Decimal>,
    pub best_bid: Option<(Price, Size)>,
    pub best_ask: Option<(Price, Size)>,
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

        info!(
            "Update applied successfully, new last_update_id: {}",
            update.final_update_id
        );
        self.last_update_id = update.final_update_id;
        self.last_update_time = update.event_time;
        self.spread = self.spread();
        self.relative_spread = self.relative_spread();
        self.mid_price = self.mid_price();
        self.imbalance = self.imbalance();

        self.best_bid = self.bids.last_key_value().map(|(&k, &v)| (k, v));
        self.best_ask = self.asks.first_key_value().map(|(&k, &v)| (k, v));

        Ok(())
    }

    fn spread(&self) -> Option<Decimal> {
        let top_bid = self.bids.last_key_value()?.0;
        let top_ask = self.asks.first_key_value()?.0;

        Some(top_ask - top_bid)
    }

    fn relative_spread(&self) -> Option<Decimal> {
        let top_bid = self.bids.last_key_value()?.0;
        let top_ask = self.asks.first_key_value()?.0;
        let mid_price = (top_bid + top_ask) / Decimal::from(2);

        Some((top_ask - top_bid) / mid_price)
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
    /// Calculates the weighted relative imbalance over the top `depth` levels of the order book.
    ///
    /// Both buy and sell volumes are weighted so that orders nearer the top have a larger impact.
    ///
    /// Returns a value in the range [-1, 1]. Positive values indicate a buy imbalance,
    /// while negative values indicate a sell imbalance.
    pub fn weighted_relative_imbalance(&self, depth: impl Into<usize>) -> Option<Decimal> {
        let depth: usize = depth.into();
        if depth == 0 {
            return None;
        }

        let mut weighted_bid = Decimal::ZERO;
        let mut weighted_ask = Decimal::ZERO;

        // For bids, iterate from best (last) to deeper levels.
        for (i, volume) in self.bids.values().rev().take(depth).enumerate() {
            // Example weighting: orders closer to the top (i==0) get weight 1,
            // then weight decays as 1/(i+1)
            let weight = Decimal::ONE / Decimal::from((i as u32) + 1);
            weighted_bid += volume * weight;
        }

        // For asks, iterate from best (first) to deeper levels.
        for (i, volume) in self.asks.values().take(depth).enumerate() {
            let weight = Decimal::ONE / Decimal::from((i as u32) + 1);
            weighted_ask += volume * weight;
        }

        let total = weighted_bid + weighted_ask;
        if total == Decimal::ZERO {
            None
        } else {
            Some((weighted_bid - weighted_ask) / total)
        }
    }

    pub fn relative_book_imbalance(&self, depth: impl Into<usize>) -> Option<Decimal> {
        let depth = depth.into();
        let best_bid = self.best_bid()?;
        let worst_bid = self.bids.iter().rev().nth(depth - 1).map(|(&k, _)| k)?;
        let best_ask = self.best_ask()?;
        let worst_ask = self.asks.iter().nth(depth - 1).map(|(&k, _)| k)?;
        let (bid_vwap, ask_vwap) = self.relative_imbalance_vwap(depth)?;

        let bid_weighted = (best_bid - bid_vwap) / (best_bid - worst_bid);
        let ask_weighted = (best_ask - ask_vwap) / (best_ask - worst_ask);

        Some((bid_weighted - ask_weighted) * Decimal::ONE_HUNDRED)
    }

    /// Calculates the relative imbalance of the mid price over the top `depth` levels of the order book.
    ///
    /// Both buy and sell volumes are weighted so that orders nearer the top have a larger impact.
    pub fn relative_mid_price_imbalance(&self, depth: impl Into<usize>) -> Option<Decimal> {
        let depth = depth.into();
        let mid_price = self.mid_price()?;
        let (bid_imbalance, ask_imbalance) = self.relative_imbalance_vwap(depth)?;

        let bid_weighted = (mid_price - bid_imbalance) / (mid_price);
        let ask_weighted = (mid_price - ask_imbalance) / (mid_price);

        Some((bid_weighted - ask_weighted) * Decimal::ONE_HUNDRED)
    }

    fn relative_imbalance_vwap(&self, depth: usize) -> Option<(Decimal, Decimal)> {
        if depth > self.bids.len().min(self.asks.len()) {
            info!("Relative imbalance depth is less than the order book depth");
            return None;
        }
        let bids_iter = self.bids.iter().rev().take(depth);
        let bid_vwap = bids_iter
            .clone()
            .map(|(&price, &size)| price * size)
            .sum::<Decimal>()
            / bids_iter.map(|(_, &size)| size).sum::<Decimal>();

        let asks_iter = self.asks.iter().take(depth);
        let ask_vwap = asks_iter
            .clone()
            .map(|(&price, &size)| price * size)
            .sum::<Decimal>()
            / asks_iter.map(|(_, &size)| size).sum::<Decimal>();

        Some((bid_vwap, ask_vwap))
    }

    fn best_bid(&self) -> Option<Decimal> {
        self.bids.last_key_value().map(|(&k, _)| k)
    }
    fn best_ask(&self) -> Option<Decimal> {
        self.asks.first_key_value().map(|(&k, _)| k)
    }
    fn best_bid_size(&self) -> Option<Decimal> {
        self.bids.last_key_value().map(|(_, &v)| v)
    }
    fn best_ask_size(&self) -> Option<Decimal> {
        self.asks.first_key_value().map(|(_, &v)| v)
    }
}
