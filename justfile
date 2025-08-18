# Market Making Trading System Commands
# Run `just` to see all available commands
set dotenv-required

set dotenv-load := true
# Default recipe - show help
default:
    @just --list



up:
    docker compose -f docker-compose-iggy.yaml up --force-recreate --build

# Infrastructure Management
[group('infrastructure')]
[working-directory("infrastructure")]
infra-up:
    @echo "🚀 Starting Fluvio infrastructure..."
    docker compose up --build -d
    @echo "✅ Fluvio cluster is starting up"

[group('infrastructure')]
[working-directory("./infrastructure")]
infra-down *FLAGS:
    @echo "🛑 Stopping Fluvio infrastructure..."
    docker-compose down {{ FLAGS }}
    @echo "✅ Fluvio cluster stopped"

[group('infrastructure')]
[working-directory("./infrastructure")]
infra-logs:
    @echo "📋 Showing Fluvio infrastructure logs..."
    docker-compose logs -f

[group('infrastructure')]
[working-directory("./infrastructure")]
infra-status:
    @echo "📊 Checking Fluvio infrastructure status..."
    docker-compose ps
    @echo "\n📊 Fluvio cluster status:"
    -fluvio cluster status 2>/dev/null || echo "❌ Fluvio CLI not connected"

[group('infrastructure')]
[working-directory("./infrastructure")]
infra-rebuild:
    @echo "🔨 Rebuilding Fluvio infrastructure..."
    docker-compose down -v
    docker-compose up --build -d

# Producer Management
[group('producer')]
[working-directory("./producer")]
producer-up symbol="BTCUSDT":
    @echo "🏭 Starting producer for {{symbol}}..."
    SYMBOL={{symbol}} docker-compose up --build -d
    @echo "✅ Producer started for {{symbol}}"


[group('producer')]
[working-directory("./producer")]
producer-down *FLAGS:
    docker-compose down {{ FLAGS }}


[group('producer')]
[working-directory("./producer")]
producer-logs:
    @echo "📋 Showing producer logs..."
    docker-compose logs -f


# Consumer Management
[group('consumer')]
[working-directory("./consumer")]
consumer-up:
    docker-compose up --build -d
[group('consumer')]
[working-directory("./consumer")]
consumer-down *ARGS:
    docker-compose down {{ ARGS }}


[group('consumer')]
consumer-analytics:
    @echo "📊 Starting analytics engine..."
    cd consumer && docker-compose --profile analytics up -d
    @echo "✅ Analytics engine started"



[group('consumer')]
consumer-logs:
    @echo "📋 Showing consumer logs..."
    cd consumer && docker-compose logs -f

# Development Commands
[group('development')]
dev-setup: infra-up
    @echo "🔧 Setting up development environment..."
    @sleep 10  # Wait for infrastructure
    @echo "✅ Development environment ready"
    @echo "💡 Run 'just producer-up' to start producing data"

# [group('development')]
# dev-full: infra-up producer-multi consumer-trading
#     @echo "🚀 Starting full development stack..."
#     @echo "✅ Full stack running!"
#     @echo "📊 Check status with: just status"

[group('development')]
dev-down: consumer-down producer-down infra-down
    @echo "🛑 Stopping full development stack..."
    @echo "✅ All services stopped"

[group('development')]
dev-rebuild: dev-down infra-rebuild
    @echo "🔨 Rebuilding entire development stack..."
    cd producer && docker-compose build
    cd consumer && docker-compose build
    @echo "✅ Development stack rebuilt"

# Monitoring & Debug
[group('monitoring')]
status:
    @echo "📊 System Status:"
    @echo "\n🏗️  Infrastructure:"
    cd infrastructure && docker-compose ps
    @echo "\n🏭 Producers:"
    cd producer && docker-compose ps
    @echo "\n📈 Consumers:"
    cd consumer && docker-compose ps
    @echo "\n📋 Fluvio Topics:"
    -fluvio topic list 2>/dev/null || echo "❌ Fluvio CLI not connected"

[group('monitoring')]
logs service="all":
    #!/usr/bin/env bash
    if [ "{{service}}" = "all" ]; then
        echo "📋 Showing all logs..."
        docker-compose -f infrastructure/docker-compose.yaml -f producer/docker-compose.yaml -f consumer/docker-compose.yaml logs -f
    elif [ "{{service}}" = "infra" ]; then
        just infra-logs
    elif [ "{{service}}" = "producer" ]; then
        just producer-logs
    elif [ "{{service}}" = "consumer" ]; then
        just consumer-logs
    else
        echo "❌ Unknown service: {{service}}"
        echo "💡 Available: all, infra, producer, consumer"
    fi

[group('monitoring')]
topics:
    @echo "📋 Fluvio Topics:"
    fluvio topic list

[group('monitoring')]
consume topic partition="0":
    @echo "👂 Consuming from topic: {{topic}}"
    fluvio consume {{topic}} --partition {{partition}} -B -d

# Utility Commands
[group('utility')]
clean: dev-down
    @echo "🧹 Cleaning up Docker resources..."
    docker system prune -f
    docker volume prune -f
    @echo "✅ Cleanup complete"

[group('utility')]
connect-cli:
    @echo "🔗 Connecting Fluvio CLI to cluster..."
    fluvio profile add local 127.0.0.1:9103 local
    fluvio profile switch local
    @echo "✅ Fluvio CLI connected"

[group('utility')]
test-connection:
    @echo "🧪 Testing Fluvio connection..."
    fluvio cluster status
    fluvio topic list

# Quick Start Commands
[group('quickstart')]
start: dev-setup producer-up
    @echo "🎉 Quick start complete!"
    @echo "💡 Use 'just status' to check everything"
    @echo "💡 Use 'just consume btcusdt' to see data"

[group('quickstart')]
stop: dev-down
    @echo "🛑 Everything stopped"