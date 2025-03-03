use anyhow::Result;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use tracing::{debug, info, warn};

use crate::{
    binance::data::{AggregateTrade, DepthUpdate},
    order_book_state::OrderBookState,
    recent_trades::{self, RecentTrades, Trade},
};

/// Configuration parameters for the simplified market maker
#[derive(Debug, Clone)]
pub struct MarketMakerConfig {
    /// Base k-factor for stink bid distance (multiplier of volatility)
    pub base_k: Decimal,
    /// Size of each stink bid order
    pub order_size: Decimal,
    /// Maximum number of active orders
    pub max_active_orders: usize,
    /// Strong imbalance threshold for aggressive stink bids
    pub strong_imbalance_threshold: Decimal,
    /// Moderate imbalance threshold for normal stink bids
    pub moderate_imbalance_threshold: Decimal,
    /// Volatility dampening factor
    pub vol_dampening: Decimal,
    /// Learning rate for k-factor adaptation
    pub learning_rate: Decimal,
    /// Minimum distance between stink bid and best bid (as percentage)
    pub min_distance_pct: Decimal,
}
impl Default for MarketMakerConfig {
    fn default() -> Self {
        Self {
            base_k: dec!(0.5),      // Start with a smaller multiplier for tighter bids
            order_size: dec!(0.01), // Standard order size
            max_active_orders: 3,   // Maximum concurrent orders
            strong_imbalance_threshold: dec!(-0.7), // Strong sell pressure
            moderate_imbalance_threshold: dec!(-0.3), // Moderate sell pressure
            vol_dampening: dec!(0.8), // Reduce volatility impact
            learning_rate: dec!(0.05), // 5% adjustment per success/failure
            min_distance_pct: dec!(0.05), // Minimum 0.05% distance from best bid
        }
    }
}

/// Represents a single order in the market
#[derive(Debug, Clone)]
pub struct Order {
    pub id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub status: OrderStatus,
    pub created_at: DateTime<Utc>,
    pub filled_at: Option<DateTime<Utc>>,
    pub reference_mid: Decimal,
    pub reference_best_bid: Decimal,
    pub k_factor_used: Decimal,
    pub imbalance_at_placement: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OrderStatus {
    New,
    Placed,
    Filled,
    Cancelled,
}
#[derive(Debug, Clone, PartialEq)]
pub enum OrderSide {
    Buy,
    Sell,
}

/// Market state used for making trading decisions
#[derive(Debug)]
pub struct MarketState {
    pub mid_price: Decimal,
    pub spread: Decimal,
    pub relative_spread: Decimal,
    pub imbalance: Decimal,
    pub volatility: Decimal,
    pub book_pressure: Decimal,
    pub regime: MarketRegime,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MarketRegime {
    Normal,
    HighVolatility,
    TrendingUp,
    TrendingDown,
    LowLiquidity,
}
/// Simplified manager for stink bid strategy
#[derive(Debug)]
pub struct MarketMaker {
    pub config: MarketMakerConfig,
    pub order_book: OrderBookState,
    pub recent_trades: RecentTrades,
    pub active_orders: Vec<Order>,
    pub filled_orders: Vec<Order>,
    pub cancelled_orders: Vec<Order>,

    // Adaptive parameters
    current_k: Decimal,
    successful_fill_count: usize,
    attempt_count: usize,

    // Performance tracking
    last_imbalance: Decimal,
    last_volatility: Decimal,

    // State tracking
    last_update_time: DateTime<Utc>,
    debug_mode: bool,
}

impl MarketMaker {
    pub fn new(
        config: MarketMakerConfig,
        order_book: OrderBookState,
        recent_trades: RecentTrades,
    ) -> Self {
        Self {
            current_k: config.base_k,
            config,
            order_book,
            recent_trades,
            active_orders: Vec::new(),
            filled_orders: Vec::new(),
            cancelled_orders: Vec::new(),
            successful_fill_count: 0,
            attempt_count: 0,
            last_imbalance: Decimal::ZERO,
            last_volatility: Decimal::ZERO,
            last_update_time: Utc::now(),
            debug_mode: true, // Set to true for detailed logging
        }
    }
    /// Updates order book state with a new depth update
    pub fn handle_depth_update(&mut self, update: DepthUpdate) -> Result<()> {
        // Process the update to our order book
        self.order_book.process_update(update)?;

        // Update tracking values
        if let Some(imbalance) = self.order_book.imbalance {
            self.last_imbalance = imbalance;
        }

        // Check if any orders should be cancelled
        self.manage_existing_orders()?;

        // Create new orders if needed
        self.place_stink_bids()?;

        Ok(())
    }

    /// Updates with a new trade
    pub fn handle_trade(&mut self, trade: impl Into<Trade>) -> Result<()> {
        let trade = trade.into();

        // Update our record of recent trades
        self.recent_trades.update(trade);

        // Update volatility tracking
        if let Some(vol) = self.recent_trades.volatility {
            // Apply dampening to reduce noise in volatility
            self.last_volatility = vol * self.config.vol_dampening;
        }

        // Check if any of our stink bids were filled
        self.check_order_fills(&trade)?;

        Ok(())
    }

    /// Checks if any orders were filled by recent trades
    fn check_order_fills(&mut self, trade: &Trade) -> Result<()> {
        // Only interested in trades where buyers are market makers (someone sold into a bid)
        if trade.buyer_market_maker {
            let mut filled_orders = Vec::new();
            let mut should_adjust_k_factor = false;

            // Check each active order to see if it was filled
            for (idx, order) in self.active_orders.iter().enumerate() {
                if order.status == OrderStatus::Placed && trade.price <= order.price {
                    filled_orders.push(idx);

                    // Calculate profit percentage
                    let profit_pct = (order.reference_mid - trade.price) / trade.price * dec!(100);

                    info!(
                        "🎯 STINK BID FILLED! Price: {}, Size: {}, Profit: {}%, K-factor: {}",
                        trade.price, order.size, profit_pct, order.k_factor_used
                    );

                    // Positive reinforcement - adjust k-factor for success
                    self.successful_fill_count += 1;

                    // Make k-factor slightly more aggressive for next time
                    should_adjust_k_factor = true;
                }
            }
            // Now apply the changes after the iteration is complete
            if should_adjust_k_factor {
                // Positive reinforcement - adjust k-factor for success
                self.successful_fill_count += 1;
                // Make k-factor slightly more aggressive for next time
                self.adjust_k_factor(true);
            }
            // Remove filled orders from active orders and add to filled orders
            for idx in filled_orders.iter().rev() {
                let mut order = self.active_orders.remove(*idx);
                order.status = OrderStatus::Filled;
                order.filled_at = Some(Utc::now());
                self.filled_orders.push(order);
            }
        }

        Ok(())
    }

    /// Manages existing orders (cancel if needed)
    fn manage_existing_orders(&mut self) -> Result<()> {
        let mut orders_to_cancel = Vec::new();
        let mut should_adjust_k_factor = false;

        if let Some((best_bid, _)) = self.order_book.best_bid {
            // Review each active order
            for (idx, order) in self.active_orders.iter().enumerate() {
                let distance_to_best = best_bid - order.price;
                let percent_distance = distance_to_best / best_bid;

                // Cancel if:
                // 1. Order is too far below current best bid (market moved up)
                // 2. Order is too close to best bid (risk of immediate fill)
                let should_cancel =
                    // Too far below (market moved up significantly)
                    (percent_distance > dec!(0.01) * order.k_factor_used * dec!(5)) ||
                    // Too close to best bid (risky)
                    (percent_distance < self.config.min_distance_pct * dec!(0.5));

                if should_cancel {
                    orders_to_cancel.push(idx);
                    info!(
                        "Cancelling stink bid - Price: {}, Best bid: {}, Distance: {}%",
                        order.price,
                        best_bid,
                        (percent_distance * dec!(100))
                    );

                    // Mark for adjustment instead of doing it here
                    should_adjust_k_factor = true;
                }
            }
        }

        // Adjust k-factor if needed
        if should_adjust_k_factor {
            // Consider this a failed attempt and adjust k-factor
            self.adjust_k_factor(false);
        }

        // Cancel orders that no longer make sense
        for idx in orders_to_cancel.iter().rev() {
            let mut order = self.active_orders.remove(*idx);
            order.status = OrderStatus::Cancelled;
            self.cancelled_orders.push(order);
        }

        Ok(())
    }

    /// Places stink bids based on current market conditions
    fn place_stink_bids(&mut self) -> Result<()> {
        // Only create new orders if we haven't reached max active orders
        if self.active_orders.len() >= self.config.max_active_orders {
            return Ok(());
        }

        // Check if we have all the necessary data
        if let (Some(mid_price), volatility, Some((best_bid, _)), Some((best_ask, _))) = (
            self.order_book.mid_price,
            self.last_volatility,
            self.order_book.best_bid,
            self.order_book.best_ask,
        ) {
            // Check if volatility is too low to make meaningful bids
            if volatility < dec!(0.00000001) {
                if self.debug_mode {
                    info!(
                        "Volatility too low for meaningful stink bids: {}",
                        volatility
                    );
                }
                return Ok(());
            }

            // Adjust k-factor based on imbalance
            let imbalance_adjusted_k =
                if self.last_imbalance < self.config.strong_imbalance_threshold {
                    // Very strong sell pressure - be aggressive
                    self.current_k * dec!(0.5)
                } else if self.last_imbalance < self.config.moderate_imbalance_threshold {
                    // Moderate sell pressure - use normal k
                    self.current_k
                } else if self.last_imbalance < dec!(0.3) {
                    // Balanced or light buy pressure - be more cautious
                    self.current_k * dec!(1.5)
                } else {
                    // Strong buy pressure - be very cautious
                    self.current_k * dec!(2.5)
                };

            // Convert volatility from return space to price space
            let price_volatility = volatility * mid_price;

            // Absolute minimal distance from best bid (safety)
            let min_price_distance = best_bid * self.config.min_distance_pct;

            // Calculate stink bid price: mid_price - (k * volatility)
            // The larger the k, the deeper the discount
            let raw_stink_bid_price = mid_price - (imbalance_adjusted_k * price_volatility);

            // Ensure minimum distance from best bid
            let stink_bid_price = if best_bid - raw_stink_bid_price < min_price_distance {
                best_bid - min_price_distance
            } else {
                raw_stink_bid_price
            };

            // Calculate the discount percentage
            let discount_pct = (mid_price - stink_bid_price) / mid_price * dec!(100);

            // Only place if discount is reasonable (not too small or too large)
            if discount_pct >= dec!(0.01) && discount_pct <= dec!(5.0) {
                // Create the new stink bid order
                self.place_order(
                    stink_bid_price,
                    self.config.order_size,
                    mid_price,
                    best_bid,
                    imbalance_adjusted_k,
                )?;
                self.attempt_count += 1;

                info!(
                    "Placing stink bid: Price={}, Mid={}, Discount={}%, Imbalance={}, K={}",
                    stink_bid_price,
                    mid_price,
                    discount_pct.round_dp(4),
                    self.last_imbalance,
                    imbalance_adjusted_k
                );
            } else if self.debug_mode {
                info!(
                    "Not placing stink bid - Discount {}% outside reasonable range (0.01-5.0%)",
                    discount_pct.round_dp(4)
                );
            }
        } else if self.debug_mode {
            // Log why we couldn't place an order
            info!(
                "Missing data for stink bid: mid_price={:?}, volatility={:?}, best_bid={:?}, best_ask={:?}",
                self.order_book.mid_price,
                self.last_volatility,
                self.order_book.best_bid,
                self.order_book.best_ask
            );
        }

        Ok(())
    }

    /// Creates and adds a new order to active orders
    fn place_order(
        &mut self,
        price: Decimal,
        size: Decimal,
        reference_mid: Decimal,
        reference_best_bid: Decimal,
        k_factor_used: Decimal,
    ) -> Result<()> {
        let order = Order {
            id: format!("order-{}", Utc::now().timestamp_millis()),
            price,
            size,
            status: OrderStatus::Placed, // Directly mark as placed
            created_at: Utc::now(),
            filled_at: None,
            reference_mid,
            reference_best_bid,
            k_factor_used,
            imbalance_at_placement: self.last_imbalance,
        };

        self.active_orders.push(order);

        Ok(())
    }

    /// Adjusts k-factor based on success or failure
    fn adjust_k_factor(&mut self, was_successful: bool) {
        if was_successful {
            // If order was filled successfully, slightly decrease k to be more aggressive
            self.current_k =
                (self.current_k * (dec!(1) - self.config.learning_rate)).max(dec!(0.1)); // Don't go below a minimum threshold
        } else {
            // If order wasn't filled, increase k to be more conservative
            self.current_k =
                (self.current_k * (dec!(1) + self.config.learning_rate)).min(dec!(3.0)); // Don't go above a maximum threshold
        }

        debug!(
            "Adjusted k-factor: {} (after {})",
            self.current_k,
            if was_successful {
                "successful fill"
            } else {
                "cancellation"
            }
        );
    }

    /// Gets current statistics
    pub fn get_statistics(&self) -> String {
        let win_rate = if self.attempt_count > 0 {
            (self.successful_fill_count as f64 / self.attempt_count as f64) * 100.0
        } else {
            0.0
        };

        format!(
            "Stink Bid Statistics:
             - Success Rate: {}/{} ({:.2}%)
             - Current K-Factor: {}
             - Active Orders: {}
             - Last Imbalance: {}
             - Last Volatility: {}
             - Total Filled Orders: {}
             - Total Cancelled Orders: {}",
            self.successful_fill_count,
            self.attempt_count,
            win_rate,
            self.current_k,
            self.active_orders.len(),
            self.last_imbalance,
            self.last_volatility,
            self.filled_orders.len(),
            self.cancelled_orders.len()
        )
    }
}
