use core::fmt;
use std::{collections::BTreeMap, marker::PhantomData};

use either::Either;
use rust_decimal::Decimal;
use tracing::debug;

use super::{Price, Size, Volume};
use crate::data::binance::models::OfferData;

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

pub type PriceLevels = BTreeMap<Price, Size>;
// Marker types to distinguish bid and ask books
#[derive(Debug, Clone, Default)]
pub struct BidSide;

#[derive(Debug, Clone, Default)]
pub struct AskSide;

type IterBook<'a> = Either<
    std::iter::Rev<std::collections::btree_map::Iter<'a, Price, Size>>,
    std::collections::btree_map::Iter<'a, Price, Size>,
>;
pub trait IterHalfBook {
    fn iter_from_best(map: &PriceLevels) -> IterBook<'_>;
    fn iter_from_worst(map: &PriceLevels) -> IterBook<'_>;
}
impl IterHalfBook for BidSide {
    fn iter_from_best(map: &PriceLevels) -> IterBook<'_> {
        Either::Left(map.iter().rev())
    }

    fn iter_from_worst(map: &PriceLevels) -> IterBook<'_> {
        Either::Right(map.iter())
    }
}
impl IterHalfBook for AskSide {
    fn iter_from_best(map: &PriceLevels) -> IterBook<'_> {
        Either::Right(map.iter())
    }

    fn iter_from_worst(map: &PriceLevels) -> IterBook<'_> {
        Either::Left(map.iter().rev())
    }
}
pub trait BookSide {
    fn is_bid() -> bool;
    fn is_ask() -> bool;
    #[must_use]
    fn side() -> OrderSide {
        if Self::is_bid() {
            OrderSide::Bid
        } else {
            OrderSide::Ask
        }
    }
}
impl BookSide for BidSide {
    fn is_bid() -> bool {
        true
    }

    fn is_ask() -> bool {
        false
    }
}
impl BookSide for AskSide {
    fn is_bid() -> bool {
        false
    }

    fn is_ask() -> bool {
        true
    }
}

pub trait SideData: IterHalfBook {
    fn best_price(map: &PriceLevels) -> Price;
    fn best_quote(map: &PriceLevels) -> Size;
    fn best_offer(map: &PriceLevels) -> OfferData;
    #[must_use]
    fn volume_weighted_price(map: &PriceLevels, volume_threshold: Volume) -> Option<Price> {
        let mut remaining_volume = volume_threshold;
        let mut total_notional = Decimal::ZERO;
        let mut total_size = Decimal::ZERO;

        for (&price, &size) in Self::iter_from_best(map) {
            let notional = price * size;
            if notional >= remaining_volume {
                let partial_size = remaining_volume / price;
                total_notional += price * partial_size;
                total_size += partial_size;
                break; // Stop once we reach the cutoff
            }
            total_notional += notional;
            total_size += size;
            remaining_volume -= notional;
        }
        total_notional
            .checked_div(total_size)
            .filter(|&v| !v.is_zero())
    }
}
impl SideData for BidSide {
    fn best_price(map: &PriceLevels) -> Price {
        map.last_key_value()
            .map(|(price, _)| *price)
            .expect("Book should not be empty")
    }

    fn best_quote(map: &PriceLevels) -> Size {
        map.last_key_value()
            .map(|(_, size)| *size)
            .expect("Book should not be empty")
    }

    fn best_offer(map: &PriceLevels) -> OfferData {
        map.last_key_value()
            .map(|(&price, &size)| (price, size).into())
            .expect("Book should not be empty")
    }
}

impl SideData for AskSide {
    fn best_price(map: &PriceLevels) -> Price {
        map.first_key_value()
            .map(|(price, _)| *price)
            .expect("Book should not be empty")
    }

    fn best_quote(map: &PriceLevels) -> Size {
        map.first_key_value()
            .map(|(_, size)| *size)
            .expect("Book should not be empty")
    }

    fn best_offer(map: &PriceLevels) -> OfferData {
        map.first_key_value()
            .map(|(&price, &size)| (price, size).into())
            .expect("Book should not be empty")
    }
}

pub trait ApplySnapshot: BookSide {
    fn apply_snapshot_offers(price_levels: &mut PriceLevels, offers: &[OfferData]) {
        assert!(
            !offers.is_empty(),
            "Cannot apply an empty snapshot to a half book"
        );
        price_levels.clear();
        Self::apply_changes(price_levels, offers);
    }

    fn apply_changes(price_levels: &mut PriceLevels, offers: &[OfferData]) {
        for &OfferData { price, size } in offers {
            if !size.is_zero() {
                match price_levels.insert(price, size) {
                    Some(existing_size) if existing_size != size => {
                        debug!(side= %Self::side(), price = %price,  old_size = %existing_size, size = %size,
                            "Updated level",
                        );
                    }
                    _ => {
                        debug!(side= %Self::side(), price = %price, size = %size,
                            "Added level",
                        );
                    }
                }
            } else if let Some(existing_size) = price_levels.remove(&price) {
                debug!(side= %Self::side(), price = %price, size = %existing_size,
                    "Removed level",
                );
            }
        }
    }
}
impl ApplySnapshot for BidSide {}
impl ApplySnapshot for AskSide {}

pub trait SideOps: SideData + ApplySnapshot {}
impl SideOps for BidSide {}
impl SideOps for AskSide {}

#[derive(Debug, Clone)]
pub struct HalfBook<Side> {
    pub price_levels: PriceLevels,
    _phantom: PhantomData<Side>,
}

impl<Side> HalfBook<Side> {
    pub fn clear(&mut self) {
        self.price_levels.clear();
    }
}

// Type aliases for convenience
pub type BidBook = HalfBook<BidSide>;
pub type AskBook = HalfBook<AskSide>;

impl<Side: IterHalfBook> HalfBook<Side> {
    #[must_use]
    pub fn iter(&self) -> IterBook<'_> {
        Side::iter_from_best(&self.price_levels)
    }
}

impl<'a, Side: IterHalfBook> IntoIterator for &'a HalfBook<Side> {
    type IntoIter = IterBook<'a>;
    type Item = (&'a Decimal, &'a Decimal);

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<Side: SideData> HalfBook<Side> {
    #[must_use]
    pub fn best_price(&self) -> Price {
        Side::best_price(&self.price_levels)
    }

    #[must_use]
    pub fn best_quote(&self) -> Size {
        Side::best_quote(&self.price_levels)
    }

    #[must_use]
    pub fn best_offer(&self) -> OfferData {
        Side::best_offer(&self.price_levels)
    }

    #[must_use]
    pub fn volume_weighted_price(&self, volume_threshold: Volume) -> Option<Price> {
        Side::volume_weighted_price(&self.price_levels, volume_threshold)
    }
}
impl<Side: ApplySnapshot> HalfBook<Side> {
    /// If a half book is created from snapshot data then its always assumed the half book is not empty.
    #[must_use]
    pub fn from_snapshot(snapshot: &[OfferData]) -> Self {
        assert!(
            !snapshot.is_empty(),
            "Cannot create a half book from an empty snapshot"
        );
        let mut price_levels = PriceLevels::new();
        Side::apply_changes(&mut price_levels, snapshot);
        Self {
            price_levels,
            _phantom: PhantomData,
        }
    }

    pub fn apply_snapshot_offers(&mut self, offers: &[OfferData]) {
        Side::apply_snapshot_offers(&mut self.price_levels, offers);
    }

    pub fn apply_changes(&mut self, offers: &[OfferData]) {
        Side::apply_changes(&mut self.price_levels, offers);
    }
}
impl<Side: BookSide> HalfBook<Side> {
    #[must_use]
    pub fn side() -> OrderSide {
        Side::side()
    }
}
