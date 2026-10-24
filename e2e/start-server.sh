#!/bin/bash
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
export GHOST_ALPHA_FINNHUB_API_KEY=test-key-not-real
export GHOST_ALPHA_DB_PATH="$PROJECT_DIR/e2e_test.db"
exec "$PROJECT_DIR/target/release/ghost-alpha" -c /dev/null serve --port 3333 --bind 127.0.0.1
