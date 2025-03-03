# Market Making Demo

A Rust-based demo project for exploring cryptocurrency market making concepts by connecting to Binance's real-time data streams.

## Overview

This project serves as an educational tool to understand market making principles, order book dynamics, and real-time market data processing. It connects to Binance WebSocket streams to receive live market data, processes this information, and calculates relevant trading statistics.

**Note:** This is a demonstration project intended for learning purposes only. It is not optimized for actual trading or production use.

## Features

- Real-time connection to Binance market data streams
- Order book state maintenance and analysis
- Recent trade tracking and volatility calculations
- Market making strategy simulation with "stink bid" approach
- Adaptive parameter adjustment based on market conditions

## Components

- **Binance Connection**: Interfaces with Binance WebSocket API to stream market data
- **Order Book State**: Maintains a local copy of the market's order book
- **Recent Trades**: Tracks and analyzes recent market trades
- **Market Maker**: Implements simple market making strategies
- **Statistics**: Generates real-time market metrics and performance indicators

## Getting Started

### Prerequisites

- Rust (edition 2024)
- Cargo

### Installation

```bash
git clone https://github.com/yourusername/market-making.git
cd market-making
cargo build
```

### Running the Demo

```bash
cargo run
```

## Theoretical Background

This project explores concepts from academic research on market making, including:

> When the order book is highly imbalanced the order value is lower. The lowest values appear when both of the limit queues are short. When the queue length is short, the probability that the queue will become empty increases before other limit orders arrive behind the MM's orders; when his order is executed on one side of the order book but not on the opposite side, the MM will have to close the position at a loss with a market order.

The implementation tries to visualize these concepts by connecting to real market data and observing order book dynamics in real-time.

## License

[MIT License](LICENSE)

## References

- [Market Making and Mean Reversion](https://wp.lancs.ac.uk/finec2018/files/2018/09/FINEC-2018-028-Xiaofei.Lu_.pdf)
