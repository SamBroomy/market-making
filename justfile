# Market Making Trading System Commands
# Run `just` to see all available commands
set dotenv-required

set dotenv-load := true
# Default recipe - show help
default:
    @just --list



up:
    docker compose -f docker-compose-iggy.yaml up --force-recreate --build
