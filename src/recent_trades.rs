use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use rust_decimal::{Decimal, MathematicalOps};
use rust_decimal_macros::dec;

use crate::binance::data::{AggregateTrade, TradeEventData};

#[derive(Debug)]
pub struct RecentTrades {
    // Trades & returns
    trades: VecDeque<(Trade, Decimal)>,
    window_size: usize,
    pub volatility: Option<Decimal>,
}

impl Default for RecentTrades {
    fn default() -> Self {
        Self::new(1_000)
    }
}

impl RecentTrades {
    pub fn new(window_size: usize) -> Self {
        Self {
            trades: VecDeque::with_capacity(window_size),
            window_size,
            volatility: None,
        }
    }

    pub fn update(&mut self, trade: impl Into<Trade>) {
        let trade = trade.into();
        let returns = self.calculate_returns(&trade);
        if self.trades.len() == self.window_size {
            self.trades.pop_back();
        }
        self.trades.push_front((trade, returns));
        self.volatility = self.calculate_volatility();
    }

    pub fn update_many(&mut self, trades: impl Iterator<Item = impl Into<Trade>>) {
        for trade in trades {
            self.update(trade);
        }
    }

    fn calculate_returns(&self, trade: &Trade) -> Decimal {
        if let Some((prev_trade, _)) = self.trades.front() {
            (trade.price - prev_trade.price)
                .checked_div(prev_trade.price)
                .unwrap_or_default()
        } else {
            dec!(0)
        }
    }

    fn calculate_volatility(&self) -> Option<Decimal> {
        let total_trades = self.trades.len();
        if total_trades < 2 {
            return None;
        }
        let total_trades = Decimal::from(total_trades);

        let sum = self.trades.iter().map(|(_, ret)| ret).sum::<Decimal>();
        let mean = sum / total_trades;

        // Use only the most recent subset (e.g., 30%) of trades for variance
        let window_size = Decimal::from(self.window_size);
        let recent_window = (window_size * dec!(0.3)).ceil();
        let recent_count = total_trades.min(recent_window);

        let variance = self
            .trades
            .iter()
            .take(recent_count.try_into().unwrap_or(0))
            .map(|(_, ret)| (*ret - mean).powi(2))
            .sum::<Decimal>()
            / recent_count;
        variance.sqrt()
    }
    fn calculate_ewma_volatility(&self, lambda: Decimal) -> Option<Decimal> {
        if self.trades.is_empty() {
            return None;
        }

        let mut ewma_var = Decimal::ZERO;
        let alpha = Decimal::ONE - lambda;

        for (i, (_, returns)) in self.trades.iter().enumerate() {
            if i == 0 {
                ewma_var = returns.powi(2);
            } else {
                ewma_var = lambda * ewma_var + alpha * returns.powi(2);
            }
        }

        ewma_var.sqrt()
    }

    pub fn price_movement(&self, over_recent_trades: impl Into<usize>) -> Option<Decimal> {
        let over_recent_trades = over_recent_trades.into();

        if self.trades.len() < over_recent_trades {
            return None;
        }

        let latest_price = self.trades.front()?.0.price;
        let earlier_trade = self.trades.get(over_recent_trades - 1)?.0.price;
        (latest_price - earlier_trade).checked_div(earlier_trade)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Trade {
    pub price: Decimal,
    pub quantity: Decimal,
    trade_time: DateTime<Utc>,
    pub buyer_market_maker: bool,
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
