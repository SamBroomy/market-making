use core::fmt;
use std::{
    collections::{BTreeMap, VecDeque},
    marker::PhantomData,
};

use anyhow::Result;
use chrono::{DateTime, Utc};
use either::Either;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, info, warn};

use crate::data::binance::models::{DepthSnapshot, DepthUpdate, OfferData};

type Price = Decimal;
type Size = Decimal;
type Volume = Decimal;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OrderSide {
    Bid,
    Ask,
}
impl fmt::Display for OrderSide {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bid => write!(f, "Bid"),
            Self::Ask => write!(f, "Ask"),
        }
    }
}

// Marker types to distinguish bid and ask books
#[derive(Debug, Clone, Default)]
pub struct BidSide;

#[derive(Debug, Clone, Default)]
pub struct AskSide;

#[derive(Debug, Clone, Default)]
pub struct HalfBook<Side: SideOps> {
    pub price_levels: BTreeMap<Price, Size>,
    _phantom: PhantomData<Side>,
}

// Type aliases for convenience
pub type BidBook = HalfBook<BidSide>;
pub type AskBook = HalfBook<AskSide>;

pub trait SideOps {
    fn iter_from_best(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Iter<'_, Price, Size>>,
        std::collections::btree_map::Iter<'_, Price, Size>,
    >;
    fn iter_prices(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Values<'_, Price, Size>>,
        std::collections::btree_map::Values<'_, Price, Size>,
    >;

    fn best_price(map: &BTreeMap<Price, Size>) -> Option<Price>;
    fn best_size(map: &BTreeMap<Price, Size>) -> Option<Size>;
    fn best_offer(map: &BTreeMap<Price, Size>) -> Option<OfferData>;
    fn side() -> OrderSide;

    #[must_use]
    fn volume_weighted_price(map: &BTreeMap<Price, Size>, cutoff: Volume) -> Option<Price> {
        let mut remaining_cutoff = cutoff;
        let mut total_notional = Decimal::ZERO;
        let mut total_size = Decimal::ZERO;

        for (&price, &size) in map {
            let v = price * size;
            if v >= remaining_cutoff {
                let partial_size = remaining_cutoff / price;
                total_notional += price * partial_size;
                total_size += partial_size;
                break; // Stop once we reach the cutoff
            }
            total_notional += v;
            total_size += size;
            remaining_cutoff -= v;
        }

        total_notional
            .checked_div(total_size)
            .filter(|&v| !v.is_zero())
    }
}
impl SideOps for BidSide {
    fn iter_from_best(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Iter<'_, Price, Size>>,
        std::collections::btree_map::Iter<'_, Price, Size>,
    > {
        Either::Left(map.iter().rev())
    }

    fn iter_prices(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Values<'_, Price, Size>>,
        std::collections::btree_map::Values<'_, Price, Size>,
    > {
        Either::Left(map.values().rev())
    }

    fn best_price(map: &BTreeMap<Price, Size>) -> Option<Price> {
        map.last_key_value().map(|(&price, _)| price)
    }

    fn best_size(map: &BTreeMap<Price, Size>) -> Option<Size> {
        map.last_key_value().map(|(_, &size)| size)
    }

    fn best_offer(map: &BTreeMap<Price, Size>) -> Option<OfferData> {
        map.last_key_value()
            .map(|(&price, &size)| OfferData { price, size })
    }

    fn side() -> OrderSide {
        OrderSide::Bid
    }
}

impl SideOps for AskSide {
    fn iter_from_best(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Iter<'_, Price, Size>>,
        std::collections::btree_map::Iter<'_, Price, Size>,
    > {
        Either::Right(map.iter())
    }

    fn iter_prices(
        map: &'_ BTreeMap<Price, Size>,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Values<'_, Price, Size>>,
        std::collections::btree_map::Values<'_, Price, Size>,
    > {
        Either::Right(map.values())
    }

    fn best_price(map: &BTreeMap<Price, Size>) -> Option<Price> {
        map.first_key_value().map(|(&price, _)| price)
    }

    fn best_size(map: &BTreeMap<Price, Size>) -> Option<Size> {
        map.first_key_value().map(|(_, &size)| size)
    }

    fn best_offer(map: &BTreeMap<Price, Size>) -> Option<OfferData> {
        map.first_key_value()
            .map(|(&price, &size)| OfferData { price, size })
    }

    fn side() -> OrderSide {
        OrderSide::Ask
    }
}

impl<S: SideOps> HalfBook<S> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            price_levels: BTreeMap::new(),
            _phantom: PhantomData,
        }
    }

    #[must_use]
    pub fn side_type_name() -> &'static str {
        std::any::type_name::<S>()
    }

    pub fn apply_snapshot_offers(&mut self, offers: &[OfferData]) {
        self.price_levels.clear();
        for &OfferData { price, size } in offers {
            if size > Decimal::ZERO {
                self.price_levels.insert(price, size);
            }
        }
    }

    pub fn apply_change(&mut self, offers: &[OfferData]) {
        for &OfferData { price, size } in offers {
            if size > Decimal::ZERO {
                match self.price_levels.insert(price, size) {
                    Some(existing_size) => {
                        if existing_size == size {
                            debug!(
                                "{} price: {} size unchanged: {}",
                                Self::side_type_name(),
                                price,
                                size
                            );
                        } else {
                            debug!(
                                "Updated {} price: {} from {} to {} diff: {}",
                                Self::side_type_name(),
                                price,
                                existing_size,
                                size,
                                existing_size - size
                            );
                        }
                    }
                    None => {
                        debug!(
                            "New {} price: {} with size: {}",
                            Self::side_type_name(),
                            price,
                            size
                        );
                    }
                }
            } else {
                match self.price_levels.remove(&price) {
                    Some(existing_size) => {
                        debug!(
                            "Removed {} price: {} with size: {}",
                            Self::side_type_name(),
                            price,
                            existing_size
                        );
                    }
                    None => {
                        debug!(
                            "Ignoring zero size {} price: {}",
                            Self::side_type_name(),
                            price
                        );
                    }
                }
            }
        }
    }

    #[must_use]
    pub fn iter(
        &self,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Iter<'_, Price, Size>>,
        std::collections::btree_map::Iter<'_, Price, Size>,
    > {
        S::iter_from_best(&self.price_levels)
    }

    #[must_use]
    pub fn iter_prices(
        &self,
    ) -> Either<
        std::iter::Rev<std::collections::btree_map::Values<'_, Price, Size>>,
        std::collections::btree_map::Values<'_, Price, Size>,
    > {
        S::iter_prices(&self.price_levels)
    }

    #[must_use]
    pub fn best_price(&self) -> Option<Price> {
        S::best_price(&self.price_levels)
    }

    #[must_use]
    pub fn best_offer(&self) -> Option<OfferData> {
        S::best_offer(&self.price_levels)
    }

    #[must_use]
    pub fn best_size(&self) -> Option<Size> {
        S::best_size(&self.price_levels)
    }

    pub fn top_levels(&self, depth: usize) -> impl Iterator<Item = (&Price, &Size)> {
        self.iter().take(depth)
    }

    #[must_use]
    pub fn order_side() -> OrderSide {
        S::side()
    }

    #[must_use]
    pub fn volume_weighted_price(&self, cutoff: Volume) -> Option<Price> {
        S::volume_weighted_price(&self.price_levels, cutoff)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.price_levels.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.price_levels.is_empty()
    }
}

impl<'a, S: SideOps> IntoIterator for &'a HalfBook<S> {
    type IntoIter = Either<
        std::iter::Rev<
            std::collections::btree_map::Iter<'a, Decimal, Decimal>,
        >,
        std::collections::btree_map::Iter<'a, Decimal, Decimal>,
    >;
    type Item = (&'a Decimal, &'a Decimal);

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// impl<S: SideOps> Deref for HalfBook<S> {
//     type Target = BTreeMap<Price, Size>;

//     fn deref(&self) -> &Self::Target {
//         &self.price_levels
//     }
// }

// impl<S: SideOps> DerefMut for HalfBook<S> {
//     fn deref_mut(&mut self) -> &mut Self::Target {
//         &mut self.price_levels
//     }
// }

#[derive(Debug, Clone, Default)]
pub struct OrderBookState {
    pub tick_size: Decimal,
    pub lot_size: Decimal,
    pub timestamp: DateTime<Utc>,
    pub bid_depth: BidBook,
    pub ask_depth: AskBook,
    last_update_id: u64,
    last_update_time: DateTime<Utc>,
    pub best_bid_tick: Price,
    pub best_ask_tick: Price,
    pub snapshot: Option<DepthSnapshot>,
}

impl OrderBookState {
    #[must_use]
    pub fn book(&self, side: OrderSide) -> Either<&BidBook, &AskBook> {
        match side {
            OrderSide::Bid => Either::Left(&self.bid_depth),
            OrderSide::Ask => Either::Right(&self.ask_depth),
        }
    }

    pub fn book_mut(&mut self, side: OrderSide) -> Either<&mut BidBook, &mut AskBook> {
        match side {
            OrderSide::Bid => Either::Left(&mut self.bid_depth),
            OrderSide::Ask => Either::Right(&mut self.ask_depth),
        }
    }

    pub fn apply_snapshot(&mut self, snapshot: &DepthSnapshot) {
        info!(
            "Applying snaphot with last_update_id: {}",
            snapshot.last_update_id
        );

        self.bid_depth.apply_snapshot_offers(&snapshot.bids);
        self.ask_depth.apply_snapshot_offers(&snapshot.asks);

        self.last_update_id = snapshot.last_update_id;
        self.last_update_time = Utc::now();
        info!(
            "Local orderbook state initialized with last_update_id: {}",
            self.last_update_id
        );
    }

    pub fn process_update(&mut self, update: &DepthUpdate) -> Result<()> {
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

        self.apply_update_changes(update);
        Ok(())
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
                self.apply_update_changes(&update);
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

    fn apply_update_changes(&mut self, update: &DepthUpdate) {
        self.bid_depth.apply_change(&update.bids);
        self.ask_depth.apply_change(&update.asks);
        info!(
            "Update applied successfully, new last_update_id: {}",
            update.final_update_id
        );
        self.last_update_id = update.final_update_id;
        self.last_update_time = update.event_time;
    }

    fn spread(&self) -> Option<Decimal> {
        let (top_bid, top_ask) = (self.bid_depth.best_price()?, self.ask_depth.best_price()?);
        Some(top_ask - top_bid)
    }

    fn relative_spread(&self) -> Option<Decimal> {
        let (top_bid, top_ask) = (self.bid_depth.best_price()?, self.ask_depth.best_price()?);
        let mid_price = (top_bid + top_ask) / dec!(2);

        Some((top_ask - top_bid) / mid_price)
    }

    #[must_use]
    pub fn mid_price(&self) -> Option<Decimal> {
        let (top_bid, top_ask) = (self.bid_depth.best_price()?, self.ask_depth.best_price()?);
        Some((top_bid + top_ask) / Decimal::from(2))
    }

    /// Vbid−Vask/Vbid+Vask
    /// Positive values indicate a buy imbalance, while negative values indicate a sell imbalance.
    #[must_use]
    pub fn imbalance(&self) -> Option<Decimal> {
        let top_bid_volume = self.bid_depth.best_size()?;
        let top_ask_volume = self.ask_depth.best_size()?;
        Some((top_bid_volume - top_ask_volume) / (top_bid_volume + top_ask_volume))
    }

    pub fn imbalance_depth(&self, depth: impl Into<usize>) -> Decimal {
        let depth: usize = depth.into();
        if depth == 0 {
            return Decimal::ZERO;
        }
        let bids = self
            .bid_depth
            .top_levels(depth)
            .map(|(_, &size)| size)
            .sum::<Decimal>();
        let asks = self
            .ask_depth
            .top_levels(depth)
            .map(|(_, &size)| size)
            .sum::<Decimal>();
        (bids - asks) / (bids + asks)
    }

    #[must_use]
    pub fn volume_weighted_price(&self, side: OrderSide, cutoff: Decimal) -> Option<Decimal> {
        let offers = match side {
            OrderSide::Bid => self.bid_depth.iter(),
            OrderSide::Ask => self.ask_depth.iter(),
        };
        let mut remaining_cutoff = cutoff;
        let mut total_notional = Decimal::ZERO;
        let mut total_size = Decimal::ZERO;

        for (&price, &size) in offers {
            let v = price * size;
            if v >= remaining_cutoff {
                let partial_size = remaining_cutoff / price;
                total_notional += price * partial_size;
                total_size += partial_size;
                break; // Stop once we reach the cutoff
            }
            total_notional += v;
            total_size += size;
            remaining_cutoff -= v;
        }

        total_notional
            .checked_div(total_size)
            .filter(|&v| !v.is_zero())
    }

    /// Calculates the Volume-adjusted Mid Price (VAMP) over the top `depth` levels of the order book.
    pub fn vamp(&self, depth: impl Into<usize>) -> Decimal {
        let depth: usize = depth.into();
        if depth == 0 {
            return Decimal::ZERO;
        }
        let total_bid_volume: Decimal = self.bid_depth.iter_prices().take(depth).sum();
        let bid_price = self
            .bid_depth
            .top_levels(depth)
            .map(|(&price, &volume)| price * volume)
            .sum::<Decimal>()
            / total_bid_volume;
        let total_ask_volume: Decimal = self.ask_depth.iter_prices().take(depth).sum();
        let ask_price = self
            .ask_depth
            .top_levels(depth)
            .map(|(&price, &volume)| price * volume)
            .sum::<Decimal>()
            / total_ask_volume;

        (bid_price + ask_price) / dec!(2)
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
        for (i, volume) in self.bid_depth.iter_prices().take(depth).enumerate() {
            // Example weighting: orders closer to the top (i==0) get weight 1,
            // then weight decays as 1/(i+1)
            let weight = Decimal::ONE / Decimal::from((i as u32) + 1);
            weighted_bid += volume * weight;
        }

        // For asks, iterate from best (first) to deeper levels.
        for (i, volume) in self.ask_depth.iter_prices().take(depth).enumerate() {
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
        let best_bid = self.bid_depth.best_price()?;
        let worst_bid = self.bid_depth.iter().nth(depth - 1).map(|(&k, _)| k)?;
        let best_ask = self.ask_depth.best_price()?;
        let worst_ask = self.ask_depth.iter().nth(depth - 1).map(|(&k, _)| k)?;
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
        if depth > self.bid_depth.len().min(self.ask_depth.len()) {
            info!("Relative imbalance depth is less than the order book depth");
            return None;
        }
        let bids_iter = self.bid_depth.iter().rev().take(depth);
        let bid_vwap = bids_iter
            .clone()
            .map(|(&price, &size)| price * size)
            .sum::<Decimal>()
            / bids_iter.map(|(_, &size)| size).sum::<Decimal>();

        let asks_iter = self.ask_depth.iter().take(depth);
        let ask_vwap = asks_iter
            .clone()
            .map(|(&price, &size)| price * size)
            .sum::<Decimal>()
            / asks_iter.map(|(_, &size)| size).sum::<Decimal>();

        Some((bid_vwap, ask_vwap))
    }
}
