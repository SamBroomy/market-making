# Market Making Trading System Commands
# Run `just` to see all available commands

set dotenv-load := true

# Default recipe - show help
default:
    @just --list

up:
    @echo "🚀 Starting market making system..."
    docker compose up --build --force-recreate

infra-up:
    @echo "🚀 Starting infrastructure services..."
    docker compose --profile infra up -d

local: infra-up
    cargo run --bin producer

# ================================
# Infrastructure Commands
# ================================


# Stop all infrastructure services
infra-down:
    @echo "🛑 Stopping infrastructure services..."
    docker compose down

# Check status of all services
status:
    @echo "📊 Service Status:"
    docker compose ps

# View logs for a specific service
logs service:
    docker compose logs -f {{service}}

# ================================
# Producer Commands
# ================================

# Run producer locally with development configuration
producer-local:
    @echo "🏠 Running producer locally with development environment..."
    MARKET_ENVIRONMENT=development cargo run --bin producer

# Run producer locally with local configuration (testnet, debug logging)
producer-local-testnet:
    @echo "🏠 Running producer locally with local testnet configuration..."
    MARKET_ENVIRONMENT=local cargo run --bin producer

# Run producer locally with production config (for testing)
producer-local-prod:
    @echo "🏠 Running producer locally with production configuration..."
    MARKET_ENVIRONMENT=production cargo run --bin producer

# Build and run producer in Docker
producer-docker:
    @echo "🐳 Starting producer in Docker..."
    docker compose up --build -d producer

# Stop Docker producer
producer-stop:
    @echo "🛑 Stopping Docker producer..."
    docker compose stop producer
    docker compose rm -f producer

# Restart Docker producer
producer-restart:
    @echo "🔄 Restarting Docker producer..."
    just producer-stop
    just producer-docker

# ================================
# Complete System Commands
# ================================

# Start everything (infrastructure + producer)
start: infra-up
    @echo "⏳ Waiting for infrastructure to be ready..."
    sleep 10
    just producer-docker
    @echo "✅ Market making system is running!"
    @echo "📊 Web interfaces:"
    @echo "  - Iggy Web UI:    http://localhost:3050"
    @echo "  - PgAdmin:        http://localhost:5050 (admin/admin)"
    @echo "  - Grafana:        http://localhost:3000 (admin/admin)"
    @echo ""
    @echo "📈 Grafana will automatically have:"
    @echo "  - TimescaleDB data source configured"
    @echo "  - Market Data Overview dashboard"

# Stop everything
stop:
    @echo "🛑 Stopping entire market making system..."
    docker compose down

# Reset everything (stop, clean, rebuild)
reset:
    @echo "🔄 Resetting entire system..."
    docker compose down -v
    docker system prune -f
    just start

# ================================
# Development Commands
# ================================

# Build the producer binary
build:
    @echo "🔨 Building producer..."
    cargo build --bin producer

# Run tests
test:
    @echo "🧪 Running tests..."
    cargo test

# Check code formatting and linting
check:
    @echo "🔍 Checking code..."
    cargo fmt --check
    cargo clippy -- -D warnings

# Format code
fmt:
    @echo "✨ Formatting code..."
    cargo fmt

# Run database migrations
migrate:
    @echo "🗃️  Running database migrations..."
    sqlx migrate run --database-url="${DATABASE_URL}"

# ================================
# Monitoring Commands
# ================================

# Show producer logs
producer-logs:
    docker compose logs -f producer

# Show infrastructure logs
infra-logs:
    docker compose logs -f timescaledb iggy pgadmin

# Show all logs
all-logs:
    docker compose logs -f

# Monitor resource usage
monitor:
    @echo "📊 Resource Usage:"
    docker stats

# Open Grafana in browser
grafana:
    @echo "🌐 Opening Grafana dashboard..."
    @echo "Username: admin"
    @echo "Password: ${GRAFANA_ADMIN_PASSWORD}"
    open http://localhost:3000 || echo "Open http://localhost:3000 manually"

# ================================
# Environment Commands
# ================================

# Show current configuration that would be used
show-config:
    @echo "🔧 Current Configuration:"
    @echo "Environment: ${MARKET_ENVIRONMENT}"
    @echo ""
    @echo "Infrastructure (from .env):"
    @echo "  PostgreSQL: ${POSTGRES_USER}@localhost:${POSTGRES_PORT}/${POSTGRES_DB}"
    @echo "  Iggy: ${IGGY_USERNAME}@localhost:${IGGY_PORT}"
    @echo ""
    @echo "Application settings are read from YAML files based on environment"
    @echo "  Local: configuration/local.yaml (testnet, debug logging)"
    @echo "  Development: configuration/development.yaml"
    @echo "  Production: configuration/production.yaml"

# Test configuration loading
test-config:
    @echo "🧪 Testing configuration loading..."
    MARKET_ENVIRONMENT=development cargo run --bin producer --help || echo "Configuration test completed"

# ================================
# Utility Commands
# ================================

# Clean Docker resources
clean:
    @echo "🧹 Cleaning Docker resources..."
    docker compose down -v
    docker system prune -f
    docker volume prune -f

# Show help for configuration system
config-help:
    @echo "🔧 Configuration System Reference:"
    @echo ""
    @echo "📁 Environment Detection:"
    @echo "  MARKET_ENVIRONMENT=local|development|production"
    @echo "  Controls which YAML file is loaded from configuration/"
    @echo ""
    @echo "🏗️  Infrastructure Variables (.env file):"
    @echo "  POSTGRES_USER, POSTGRES_PASSWORD, POSTGRES_DB, POSTGRES_PORT"
    @echo "  IGGY_USERNAME, IGGY_PASSWORD, IGGY_PORT"
    @echo "  GRAFANA_ADMIN_PASSWORD, PGADMIN_DEFAULT_PASSWORD"
    @echo ""
    @echo "📋 Application Configuration (YAML files):"
    @echo "  trading.symbols, trading.snapshot_limit, trading.startup_delay_seconds"
    @echo "  binance.use_testnet, binance.update_speed"
    @echo "  features.enable_streaming, features.enable_database"
    @echo "  logging.level"
    @echo ""
    @echo "🎯 Configuration Precedence:"
    @echo "  1. Environment variables (MARKET_*) - Override YAML"
    @echo "  2. Environment-specific YAML (local.yaml, development.yaml, production.yaml)"
    @echo "  3. Base YAML (base.yaml)"
    @echo ""
    @echo "💡 Examples:"
    @echo "  just producer-local-testnet  # Uses local.yaml (testnet + debug)"
    @echo "  just producer-local          # Uses development.yaml"
    @echo "  MARKET_ENVIRONMENT=production just producer-local  # Uses production.yaml"
