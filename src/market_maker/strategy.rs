use chrono::TimeDelta;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

// Import your existing types
use crate::book::order_book::{MarketDataSummary, StateSnapshot};
use crate::{book::half_book::OrderSide, trades::TradeSummary};

// Strategy configuration
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub signal_threshold: Decimal,         // 0.001 = 0.1%
    pub max_skew: Decimal,                 // 0.3 = 30%
    pub base_spread_multiplier: Decimal,   // 1.2x current spread
    pub max_position_usd: Decimal,         // 1000 USD
    pub high_vol_threshold: Decimal,       // 0.02 = 2%
    pub update_threshold: Decimal,         // 0.0005 = 0.05%
    pub evaluation_interval_ms: TimeDelta, // 100ms
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            signal_threshold: dec!(0.001),                        // 0.1%
            max_skew: dec!(0.3),                                  // 30%
            base_spread_multiplier: dec!(1.2),                    // 1.2x current spread
            max_position_usd: dec!(1000),                         // 1000 USD
            high_vol_threshold: dec!(0.02),                       // 2%
            update_threshold: dec!(0.0005),                       // 0.05%
            evaluation_interval_ms: TimeDelta::milliseconds(100), // 100ms
        }
    }
}
#[derive(Debug, Clone)]
pub enum BidAskPressure {
    Strong,
    Moderate,
    Weak,
    Neutral,
}
// Data flowing between strategy components
#[derive(Debug, Clone)]
pub struct SignalAnalysis {
    pub direction_signal: Decimal, // [-1, 1] bearish to bullish
    pub confidence_level: Decimal, // [0, 1]
    pub signal_strength: Decimal,  // Raw signal magnitude
    pub bid_pressure: BidAskPressure,
}
#[derive(Debug, Clone)]
pub struct QuoteTargets {
    pub target_bid_price: Decimal,
    pub target_ask_price: Decimal,
    pub quote_bid_size: Decimal,
    pub quote_ask_size: Decimal,
    pub skew_applied: Decimal,
}

#[derive(Debug, Clone)]
pub struct RiskAdjustedQuotes {
    pub final_bid_price: Decimal,
    pub final_ask_price: Decimal,
    pub final_bid_size: Decimal,
    pub final_ask_size: Decimal,
    pub risk_adjustment: String, // Description of what was adjusted
}

#[derive(Debug, Clone)]
pub enum OrderAction {
    Place {
        price: Decimal,
        size: Decimal,
        side: OrderSide,
    },
    Cancel {
        order_id: String,
    },
    Replace {
        order_id: String,
        new_price: Decimal,
        new_size: Decimal,
    },
    NoAction,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub current_position_usd: Decimal,
    pub base_asset_qty: Decimal,
}

pub struct ActiveOrder {
    pub order_id: String,
    pub price: Decimal,
    pub size: Decimal,
    pub side: OrderSide,
}

// ==== COMPONENT 1: SIGNAL ANALYSIS ENGINE ====
pub struct SignalAnalysisEngine {
    config: StrategyConfig,
}
impl SignalAnalysisEngine {
    pub fn new(config: StrategyConfig) -> Self {
        Self { config }
    }

    pub fn analyze_signals(
        &self,
        market_data: &MarketDataSummary,
        trade_data: &TradeSummary,
    ) -> SignalAnalysis {
        // Calculate directional signal from micro-price vs mid-price
        let price_diff = market_data.micro_price - market_data.mid_price;
        let spread = market_data.mid_price * market_data.spread_bps / dec!(10000);

        // Normalize by spread to get signal strength
        let raw_signal = if spread > dec!(0) {
            price_diff / spread
        } else {
            dec!(0)
        };

        // Apply threshold - only generate signals above noise level
        let direction_signal = if raw_signal.abs() > self.config.signal_threshold {
            raw_signal.max(dec!(-1)).min(dec!(1)) // Clamp to [-1, 1]
        } else {
            dec!(0)
        };

        // Calculate confidence based on multiple factors
        let imbalance_confidence = (market_data.quote_imbalance_l5 - dec!(0.5)).abs() * dec!(2);
        let vol_confidence =
            dec!(1) - (trade_data.volatility / self.config.high_vol_threshold).min(dec!(1));
        let confidence_level = (imbalance_confidence * vol_confidence).min(dec!(1));

        // Assess bid/ask pressure
        let bid_pressure = match market_data.quote_imbalance_l5 {
            i if i > dec!(0.7) => BidAskPressure::Strong,
            i if i > dec!(0.6) => BidAskPressure::Moderate,
            i if i < dec!(0.3) => BidAskPressure::Strong, // Ask pressure
            i if i < dec!(0.4) => BidAskPressure::Moderate, // Ask pressure
            _ => BidAskPressure::Neutral,
        };

        SignalAnalysis {
            direction_signal,
            confidence_level,
            signal_strength: raw_signal.abs(),
            bid_pressure,
        }
    }
}

// ==== COMPONENT 2: QUOTE PRICING ENGINE ====
pub struct QuotePricingEngine {
    config: StrategyConfig,
}

impl QuotePricingEngine {
    pub fn new(config: StrategyConfig) -> Self {
        Self { config }
    }

    pub fn calculate_target_quotes(
        &self,
        signals: &SignalAnalysis,
        market_data: &MarketDataSummary,
        state: &StateSnapshot,
    ) -> QuoteTargets {
        // Calculate skew based on signal direction and confidence
        let skew = signals.direction_signal * signals.confidence_level * self.config.max_skew;

        // Calculate base spread (minimum spread we want to maintain)
        let current_spread = market_data.mid_price * market_data.spread_bps / dec!(10000);
        let base_spread = current_spread * self.config.base_spread_multiplier;

        // Apply skew to spread allocation
        let bid_offset = base_spread * (dec!(1) - skew);
        let ask_offset = base_spread * (dec!(1) + skew);

        // Use micro-price as fair value center
        let target_bid_price = market_data.micro_price - bid_offset;
        let target_ask_price = market_data.micro_price + ask_offset;

        // Size quotes based on confidence (more confident = bigger size)
        let base_size = dec!(100); // Base order size in USD
        let confidence_multiplier = dec!(0.5) + (signals.confidence_level * dec!(0.5));
        let quote_size = base_size * confidence_multiplier;

        QuoteTargets {
            target_bid_price,
            target_ask_price,
            quote_bid_size: quote_size,
            quote_ask_size: quote_size,
            skew_applied: skew,
        }
    }
}

// ==== COMPONENT 3: RISK MANAGEMENT ====
pub struct RiskManager {
    config: StrategyConfig,
}

impl RiskManager {
    pub fn new(config: StrategyConfig) -> Self {
        Self { config }
    }

    pub fn apply_risk_controls(
        &self,
        targets: &QuoteTargets,
        position: &Position,
        market_data: &MarketDataSummary,
        trade_data: &TradeSummary,
    ) -> RiskAdjustedQuotes {
        let mut final_bid_price = targets.target_bid_price;
        let mut final_ask_price = targets.target_ask_price;
        let mut final_bid_size = targets.quote_bid_size;
        let mut final_ask_size = targets.quote_ask_size;
        let mut adjustments = Vec::new();

        // 1. Inventory Risk Management
        let position_ratio = position.current_position_usd / self.config.max_position_usd;
        if position_ratio.abs() > dec!(0.8) {
            if position_ratio > dec!(0) {
                // Long position - encourage selling, discourage buying
                final_ask_price *= dec!(0.99); // Tighter asks
                final_bid_price *= dec!(0.995); // Wider bids
                final_ask_size *= dec!(1.5);
                final_bid_size *= dec!(0.5);
                adjustments.push("inventory_skew_long");
            } else {
                // Short position - encourage buying, discourage selling
                final_bid_price *= dec!(1.01); // Tighter bids
                final_ask_price *= dec!(1.005); // Wider asks
                final_bid_size *= dec!(1.5);
                final_ask_size *= dec!(0.5);
                adjustments.push("inventory_skew_short");
            }
        }

        // 2. Volatility Risk Management
        if trade_data.volatility > self.config.high_vol_threshold {
            let vol_multiplier = dec!(1) + (trade_data.volatility / self.config.high_vol_threshold);
            // Widen spreads and reduce sizes during high volatility
            final_bid_price *= dec!(0.995) / vol_multiplier;
            final_ask_price *= vol_multiplier / dec!(0.995);
            final_bid_size *= dec!(0.7);
            final_ask_size *= dec!(0.7);
            adjustments.push("high_volatility_protection");
        }

        // 3. Sanity Checks
        let current_bid = market_data.mid_price * dec!(0.95); // Don't bid more than 5% below mid
        let current_ask = market_data.mid_price * dec!(1.05); // Don't ask more than 5% above mid

        final_bid_price = final_bid_price.min(current_bid);
        final_ask_price = final_ask_price.max(current_ask);

        // 4. Minimum Size Checks
        let min_size = dec!(10); // Minimum $10 order
        final_bid_size = final_bid_size.max(min_size);
        final_ask_size = final_ask_size.max(min_size);

        if final_bid_price >= final_ask_price {
            adjustments.push("crossed_quotes_fixed");
            let mid = (final_bid_price + final_ask_price) / dec!(2);
            let min_spread = market_data.mid_price * dec!(0.0001); // 0.01% minimum spread
            final_bid_price = mid - min_spread;
            final_ask_price = mid + min_spread;
        }

        RiskAdjustedQuotes {
            final_bid_price,
            final_ask_price,
            final_bid_size,
            final_ask_size,
            risk_adjustment: adjustments.join(", "),
        }
    }
}
// ==== COMPONENT 4: ORDER DECISION ENGINE ====
pub struct OrderDecisionEngine {
    config: StrategyConfig,
}

impl OrderDecisionEngine {
    pub fn new(config: StrategyConfig) -> Self {
        Self { config }
    }

    pub fn generate_order_actions(
        &self,
        quotes: &RiskAdjustedQuotes,
        active_orders: &[ActiveOrder],
    ) -> Vec<OrderAction> {
        let mut actions = Vec::new();

        // Check existing bid orders
        let mut found_good_bid = false;
        for order in active_orders
            .iter()
            .filter(|o| matches!(o.side, OrderSide::Bid))
        {
            let price_diff = (order.price - quotes.final_bid_price).abs();
            let update_threshold = quotes.final_bid_price * self.config.update_threshold;

            if price_diff > update_threshold {
                // Price moved significantly, need to update
                actions.push(OrderAction::Replace {
                    order_id: order.order_id.clone(),
                    new_price: quotes.final_bid_price,
                    new_size: quotes.final_bid_size,
                });
                found_good_bid = true;
            } else {
                // Current order is fine
                found_good_bid = true;
            }
        }

        // If no good bid order exists, place one
        if !found_good_bid {
            actions.push(OrderAction::Place {
                price: quotes.final_bid_price,
                size: quotes.final_bid_size,
                side: OrderSide::Bid,
            });
        }

        // Check existing ask orders (same logic)
        let mut found_good_ask = false;
        for order in active_orders
            .iter()
            .filter(|o| matches!(o.side, OrderSide::Ask))
        {
            let price_diff = (order.price - quotes.final_ask_price).abs();
            let update_threshold = quotes.final_ask_price * self.config.update_threshold;

            if price_diff > update_threshold {
                actions.push(OrderAction::Replace {
                    order_id: order.order_id.clone(),
                    new_price: quotes.final_ask_price,
                    new_size: quotes.final_ask_size,
                });
                found_good_ask = true;
            } else {
                found_good_ask = true;
            }
        }

        if !found_good_ask {
            actions.push(OrderAction::Place {
                price: quotes.final_ask_price,
                size: quotes.final_ask_size,
                side: OrderSide::Ask,
            });
        }

        actions
    }
}

// ==== MAIN STRATEGY ORCHESTRATOR ====
pub struct MicroPriceMarketMaker {
    config: StrategyConfig,
    signal_engine: SignalAnalysisEngine,
    pricing_engine: QuotePricingEngine,
    risk_manager: RiskManager,
    order_engine: OrderDecisionEngine,
}

impl MicroPriceMarketMaker {
    pub fn new(config: StrategyConfig) -> Self {
        Self {
            signal_engine: SignalAnalysisEngine::new(config.clone()),
            pricing_engine: QuotePricingEngine::new(config.clone()),
            risk_manager: RiskManager::new(config.clone()),
            order_engine: OrderDecisionEngine::new(config.clone()),
            config,
        }
    }

    /// Main strategy execution - this is called every `evaluation_interval_ms`
    pub fn evaluate_and_generate_actions(
        &self,
        market_data: &MarketDataSummary,
        state: &StateSnapshot,
        trade_data: &TradeSummary,
        current_position: &Position,
        active_orders: &[ActiveOrder],
    ) -> (
        SignalAnalysis,
        QuoteTargets,
        RiskAdjustedQuotes,
        Vec<OrderAction>,
    ) {
        // 1. Analyze market signals
        let signals = self.signal_engine.analyze_signals(market_data, trade_data);

        // 2. Calculate target quotes based on signals
        let targets = self
            .pricing_engine
            .calculate_target_quotes(&signals, market_data, state);

        // 3. Apply risk management
        let final_quotes = self.risk_manager.apply_risk_controls(
            &targets,
            current_position,
            market_data,
            trade_data,
        );

        // 4. Generate order actions
        let actions = self
            .order_engine
            .generate_order_actions(&final_quotes, active_orders);

        (signals, targets, final_quotes, actions)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;

    use super::*;

    #[test]
    fn test_basic_strategy_flow() {
        let config = StrategyConfig::default();
        let strategy = MicroPriceMarketMaker::new(config);

        // Mock data using your actual types
        let market_data = MarketDataSummary {
            event_time: Utc::now(),
            spread_bps: dec!(10),     // 0.1% spread
            mid_price: dec!(50000),   // $50,000
            micro_price: dec!(50001), // Micro-price slightly above mid
            quote_imbalance_l1: dec!(0.4),
            quote_imbalance_l5: dec!(0.65), // Bid heavy
            weighted_mid: dec!(49999),
            bid_volume_l1: dec!(100),
            ask_volume_l1: dec!(150),
            bid_volume_l5: dec!(500),
            ask_volume_l5: dec!(300),
            update_id: 12345,
        };

        let state = StateSnapshot {
            bids: BTreeMap::from([(dec!(49999), dec!(100)), (dec!(49998), dec!(200))]),
            asks: BTreeMap::from([(dec!(50001), dec!(150)), (dec!(50002), dec!(100))]),

            last_update_id: 12345,
            last_update_time: Utc::now(),
            depth_limit: 100,
        };

        let trade_data = TradeSummary {
            event_time: Utc::now(),
            buy_volume: dec!(1000),
            sell_volume: dec!(800),
            trade_count: 50,
            trade_intensity: dec!(25),
            imbalance: dec!(0.2),
            volatility: dec!(0.015), // 1.5% volatility
        };

        let position = Position {
            current_position_usd: dec!(0),
            base_asset_qty: dec!(0),
        };

        let active_orders = vec![];

        let (signals, targets, final_quotes, actions) = strategy.evaluate_and_generate_actions(
            &market_data,
            &state,
            &trade_data,
            &position,
            &active_orders,
        );

        println!("Signals: {signals:?}");
        println!("Targets: {targets:?}");
        println!("Final Quotes: {final_quotes:?}");
        println!("Actions: {actions:?}");

        // Basic sanity checks
        assert!(signals.direction_signal > dec!(0)); // Should be bullish due to micro-price > mid-price
        assert!(final_quotes.final_bid_price < final_quotes.final_ask_price); // No crossed quotes
        assert!(!actions.is_empty()); // Should generate some orders
    }
}
