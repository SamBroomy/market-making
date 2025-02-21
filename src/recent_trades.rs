use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, MathematicalOps};

use crate::binance::data::{AggregateTrade, TradeEventData};

pub struct RecentTrades {
    trades: VecDeque<Trade>,
    window_size: usize,
}

impl RecentTrades {
    pub fn new(window_size: usize) -> Self {
        Self {
            trades: VecDeque::with_capacity(window_size),
            window_size,
        }
    }

    pub fn update(&mut self, trade: impl Into<Trade>) {
        let trade = trade.into();

        if self.trades.len() == self.window_size {
            self.trades.pop_back();
        }
        self.trades.push_front(trade);
    }

    pub fn update_many(&mut self, trades: impl Iterator<Item = impl Into<Trade>>) {
        for trade in trades {
            self.update(trade);
        }
    }

    pub fn calculate_volitility(&self) -> Option<Decimal> {
        if self.trades.len() < 2 {
            return None;
        }
        let count = Decimal::from(self.trades.len());
        let sum = self.trades.iter().map(|t| t.price).sum::<Decimal>();
        let mean = sum / count;
        let variance = self
            .trades
            .iter()
            .map(|t| (t.price - mean).powi(2))
            .sum::<Decimal>()
            / count;
        variance.sqrt()
    }
}

pub struct Trade {
    price: Decimal,
    quantity: Decimal,
    trade_time: DateTime<Utc>,
    buyer_market_maker: bool,
    num_trades: u64,
}

impl From<TradeEventData> for Trade {
    fn from(event: TradeEventData) -> Self {
        Self {
            price: event.price,
            quantity: event.quantity,
            trade_time: event.trade_time,
            buyer_market_maker: event.buyer_market_maker,
            num_trades: 1,
        }
    }
}

impl From<AggregateTrade> for Trade {
    fn from(event: AggregateTrade) -> Self {
        Self {
            price: event.price,
            quantity: event.quantity,
            trade_time: event.trade_time,
            buyer_market_maker: event.buyer_market_maker,
            num_trades: event.last_trade_id - event.first_trade_id + 1,
        }
    }
}
