# Market Making Demo

A Rust-based market data collection and processing system for exploring cryptocurrency market making concepts. Connects to Binance WebSocket streams to collect real-time market data, process order books, track trades, and calculate market metrics.

## Overview

This project serves as an educational tool to understand market making principles, order book dynamics, and real-time market data processing. It streams live data from Binance, maintains local order book state, and stores metrics in a time-series database for analysis.

**Note:** This is a demonstration project intended for learning purposes only. It is not optimized for actual trading or production use.

## Features

- **Multi-stream market data collection** from Binance WebSocket API
  - Differential order book depth updates
  - Individual and rolling window ticker data
  - Aggregate trade streams
- **Real-time order book reconstruction** with configurable snapshot limits
- **Trade aggregation and analysis** with windowed volatility calculations
- **Message queue integration** (Apache Iggy) for event streaming
- **Time-series persistence** (TimescaleDB) for historical analysis
- **Monitoring dashboards** (Grafana) with pre-configured visualizations
- **Multi-symbol support** with configurable stream parameters per trading pair

## Architecture

The system is organized as a Cargo workspace with layered architecture:

- **`domain`** - Core business logic and models (order books, trades, market data)
- **`infrastructure`** - External integrations (Binance, Iggy, TimescaleDB)
- **`application`** - Orchestration and stream processing handlers
- **`support`** - Cross-cutting utilities (shutdown coordination, logging)
- **`market-data-engine`** - Main binary that wires everything together

Data flows from Binance WebSocket streams → handlers → message queue (Iggy) + database (TimescaleDB) + in-memory processors for derived metrics.

## Getting Started

### Prerequisites

- **Rust** (edition 2024) - Install from [rustup.rs](https://rustup.rs)
- **Docker & Docker Compose** - For running infrastructure services
- **Just** - Task runner (`cargo install just`)

### Installation

1. Clone the repository:

```bash
git clone https://github.com/SamBroomy/market-making.git
cd market-making
```

2. Copy the environment file and configure as needed:

```bash
cp .env.example .env
```

3. Start infrastructure services (TimescaleDB, Iggy, Grafana):

```bash
just infra-up
```

### Running the System

**Option 1: Full Docker deployment** (recommended for testing the complete system)

```bash
just start
```

This starts all services including the market data engine in Docker.

**Option 2: Local development** (infrastructure in Docker, engine runs locally)

```bash
# Start infrastructure only
just infra-up

# Run the market data engine locally with development config
just producer-local
```

### Accessing Services

Once running, access the web interfaces:

- **Iggy Web UI**: <http://localhost:3050> (message queue monitoring)
- **Grafana**: <http://localhost:3000> (username: `admin`, password: `admin`)
  - Pre-configured with TimescaleDB datasource and market data dashboards

### Configuration

Trading pairs and stream settings are configured in TOML files under `configuration/`:

- `base.toml` - Base configuration with trading pairs
- `development.toml` - Development overrides (debug logging)
- `production.toml` - Production settings

Control which config is loaded via the `MARKET_ENVIRONMENT` environment variable:

```bash
MARKET_ENVIRONMENT=development just producer-local
```

To add new trading pairs, edit `configuration/base.toml` and add a `[trading.market.SYMBOL]` section.

## Development

### Common Commands

```bash
# View all available commands
just

# Format code
just fmt

# Run lints
just check

# Run tests
just test

# View logs
just producer-logs
just all-logs

# Stop all services
just stop
```

See the [justfile](./justfile) for all available commands.

### Project Structure

```
crates/
├── domain/              # Core models and business logic
├── infrastructure/      # Binance, Iggy, TimescaleDB implementations
├── application/         # Stream handlers and orchestration
├── support/             # Utilities (shutdown, logging)
└── bin-market-data-engine/  # Main binary
```

For detailed development guidance, see [CLAUDE.md](./CLAUDE.md).

## Data Storage

Market data is stored in **TimescaleDB** (PostgreSQL with time-series extensions):

- Order book depth updates
- Individual ticker snapshots
- Rolling window ticker data
- Aggregate trade data
- Trade summaries and volatility metrics

Run database migrations with:

```bash
just migrate
```

## Message Streaming

The system publishes processed data to **Apache Iggy** message queue streams:

- `diff_book_depth` - Order book depth updates
- `orderbook_state` - Reconstructed order book snapshots
- `orderbook_signals` - Order book imbalance signals
- `ticker` - Individual ticker updates
- `rolling_window_ticker_*` - Windowed ticker data
- `agg_trade` - Aggregate trades
- `agg_trade_summary_*` - Trade summaries with volatility metrics

## Monitoring

Grafana dashboards provide real-time visualization of:

- Market data ingestion rates
- Order book depth and spread
- Trade volume and volatility
- System performance metrics

Access Grafana at <http://localhost:3000> after running `just start`.

## Theoretical Background

This project explores concepts from academic research on market making, including:

> When the order book is highly imbalanced the order value is lower. The lowest values appear when both of the limit queues are short. When the queue length is short, the probability that the queue will become empty increases before other limit orders arrive behind the MM's orders; when his order is executed on one side of the order book but not on the opposite side, the MM will have to close the position at a loss with a market order.

The implementation connects to real market data to observe and analyze these order book dynamics in real-time.

## License

[MIT License](LICENSE)

## References

- [Market Making and Mean Reversion](https://wp.lancs.ac.uk/finec2018/files/2018/09/FINEC-2018-028-Xiaofei.Lu_.pdf)

## Security Note

This setup is intended for local development and learning. Infrastructure services are exposed on localhost without production-grade security. Only run on trusted networks.
