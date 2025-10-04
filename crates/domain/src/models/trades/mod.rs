// mod trade_processor;

use std::collections::VecDeque;

use chrono::{DateTime, Duration, Utc};
use rust_decimal::{Decimal, MathematicalOps, dec};

// pub use trade_processor::TradeProcessor;
use crate::models::market_data::{AggregateTrade, TradeSummary};

#[derive(Debug, Clone, Copy)]
struct TradeData {
    timestamp: DateTime<Utc>,
    price: Decimal,
    quantity: Decimal,
    is_buy: bool,
}

#[derive(Debug, Clone)]
pub struct TradeTracker {
    trades: VecDeque<TradeData>,

    total_buy_volume: Decimal,
    total_sell_volume: Decimal,
    total_trade_count: i32,
    total_notional: Decimal,

    window_duration: Duration, // eg 60 seconds
    lambda: Decimal,
}

impl TradeTracker {
    #[must_use]
    pub fn new(window_duration: Duration) -> Self {
        Self {
            trades: VecDeque::new(),
            total_buy_volume: Decimal::ZERO,
            total_sell_volume: Decimal::ZERO,
            total_trade_count: 0,
            total_notional: Decimal::ZERO,
            window_duration,
            lambda: dec!(2).ln() / Decimal::from(window_duration.num_seconds()),
        }
    }

    fn expire_old_trades(&mut self, current_time: DateTime<Utc>) {
        while let Some(front) = self.trades.front() {
            if current_time - front.timestamp > self.window_duration
                && let Some(old_trade) = self.trades.pop_front()
            {
                if old_trade.is_buy {
                    self.total_buy_volume -= old_trade.quantity;
                } else {
                    self.total_sell_volume -= old_trade.quantity;
                }
                self.total_trade_count -= 1;
                self.total_notional -= old_trade.price * old_trade.quantity;
            } else {
                break;
            }
        }
    }

    pub fn add_trade(&mut self, trade: &AggregateTrade) {
        self.expire_old_trades(trade.trade_time);

        let is_buy = !trade.buyer_market_maker;
        if is_buy {
            // buyer was active (taker) = buy volume
            self.total_buy_volume += trade.quantity;
        } else {
            // if the buyer was passive (maker), seller was active (taker) = sell volume
            self.total_sell_volume += trade.quantity;
        }

        self.total_trade_count += 1;
        self.total_notional += trade.price * trade.quantity;
        self.trades.push_back(TradeData {
            timestamp: trade.trade_time,
            price: trade.price,
            quantity: trade.quantity,
            is_buy,
        });
    }

    /// Measures inherit price uncertainty from trade price changes.
    #[must_use]
    pub fn calculate_volatility(&self) -> Decimal {
        if self.trades.len() < 2 {
            return Decimal::ZERO;
        }

        let mut sum_squared_returns = Decimal::ZERO;
        let mut count = 0;

        for i in 1..self.trades.len() {
            let prev = &self.trades[i - 1];
            let current = &self.trades[i];
            if prev.price > Decimal::ZERO {
                let return_val = (current.price - prev.price) / prev.price;
                sum_squared_returns += return_val * return_val;
                count += 1;
            }
        }

        if count > 0 {
            (sum_squared_returns / Decimal::from(count))
                .sqrt()
                .expect("sqrt of non-negative")
        } else {
            Decimal::ZERO
        }
    }

    /// Measures flow direction where recent activity matters more than older activity.
    #[must_use]
    pub fn calculate_weighted_trade_imbalance(&self) -> Decimal {
        if self.trades.is_empty() {
            return Decimal::ZERO;
        }
        let reference_time = self.trades.back().map_or_else(Utc::now, |b| b.timestamp);
        let mut weighted_buy_volume = Decimal::ZERO;
        let mut weighted_sell_volume = Decimal::ZERO;

        for trade in &self.trades {
            let age_seconds = (reference_time - trade.timestamp).num_seconds();
            let weight = (-self.lambda * Decimal::from(age_seconds)).exp();

            let weighted_volume = trade.quantity * weight;
            if trade.is_buy {
                weighted_buy_volume += weighted_volume;
            } else {
                weighted_sell_volume += weighted_volume;
            }
        }

        let weighted_gross_volume = weighted_buy_volume + weighted_sell_volume;
        if weighted_gross_volume > Decimal::ZERO {
            (weighted_buy_volume - weighted_sell_volume) / weighted_gross_volume
        } else {
            Decimal::ZERO
        }
    }

    /// Single pass calculation
    fn calculate_volatility_and_imbalance(&self) -> (Decimal, Decimal) {
        if self.trades.is_empty() {
            return (Decimal::ZERO, Decimal::ZERO);
        }

        let reference_time = self.trades.back().map_or_else(Utc::now, |b| b.timestamp);
        let mut sum_squared_returns = Decimal::ZERO;
        let mut return_count = 0;
        let mut weighted_buy_volume = Decimal::ZERO;
        let mut weighted_sell_volume = Decimal::ZERO;

        for i in 0..self.trades.len() {
            let trade = &self.trades[i];

            // weighted trade imbalance calculation
            let age_seconds = (reference_time - trade.timestamp).num_seconds();
            let weight = (-self.lambda * Decimal::from(age_seconds)).exp();
            let weighted_volume = trade.quantity * weight;
            if trade.is_buy {
                weighted_buy_volume += weighted_volume;
            } else {
                weighted_sell_volume += weighted_volume;
            }

            // volatility calculation
            if i > 0 {
                let prev_trade = &self.trades[i - 1];
                if prev_trade.price > Decimal::ZERO {
                    let return_val = (trade.price - prev_trade.price) / prev_trade.price;
                    sum_squared_returns += return_val * return_val;
                    return_count += 1;
                }
            }
        }

        let volatility = if return_count > 0 {
            (sum_squared_returns / Decimal::from(return_count))
                .sqrt()
                .unwrap_or(Decimal::ZERO)
        } else {
            Decimal::ZERO
        };

        let weighted_gross_volume = weighted_buy_volume + weighted_sell_volume;
        let imbalance = if weighted_gross_volume > Decimal::ZERO {
            (weighted_buy_volume - weighted_sell_volume) / weighted_gross_volume
        } else {
            Decimal::ZERO
        };

        (volatility, imbalance)
    }

    pub fn summary(&self) -> TradeSummary {
        let trade_intensity = Decimal::from(self.total_trade_count)
            / Decimal::from(self.window_duration.num_seconds());

        let (volatility, imbalance) = self.calculate_volatility_and_imbalance();

        TradeSummary {
            event_time: self.trades.back().map_or_else(Utc::now, |t| t.timestamp),
            buy_volume: self.total_buy_volume,
            sell_volume: self.total_sell_volume,
            trade_count: self.total_trade_count,
            trade_intensity,
            volatility,
            imbalance,
        }
    }
}
