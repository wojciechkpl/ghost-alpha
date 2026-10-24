# Plan 02: Docker One-Liner Deployment

**Effort:** Small (1-2 days)
**Impact:** High
**Priority:** 2

## Problem

Building Ghost Alpha requires the Rust toolchain, and the sentiment feature needs ONNX Runtime libraries. Most potential users don't have Rust installed and won't install it to try a tool.

## Solution

Publish a multi-stage Docker image that:
- Builds the binary with all features (including sentiment)
- Bakes in the FinBERT ONNX model
- Runs with a single `docker run` command

## Implementation

### 1. Multi-stage Dockerfile

```dockerfile
# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.77-bookworm AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY migrations/ migrations/
COPY data/ data/

RUN cargo build --release --features sentiment

# ── Stage 2: Model download ──────────────────────────────────
FROM python:3.12-slim AS model
RUN pip install huggingface_hub
RUN huggingface-cli download ProsusAI/finbert --local-dir /models/finbert \
    --include "*.onnx" "tokenizer.json" "config.json"

# ── Stage 3: Runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ghost-alpha /usr/local/bin/
COPY --from=model /models/finbert /models/finbert

ENV GHOST_ALPHA_SENTIMENT_MODEL_PATH=/models/finbert

ENTRYPOINT ["ghost-alpha"]
CMD ["screen"]
```

### 2. Docker Compose for scheduled operation

Create `docker-compose.yml`:

```yaml
version: "3.8"
services:
  ghost-alpha:
    image: ghcr.io/user/ghost-alpha:latest
    environment:
      - GHOST_ALPHA_FINNHUB_API_KEY=${FINNHUB_API_KEY}
      - GHOST_ALPHA_TELEGRAM_BOT_TOKEN=${TELEGRAM_BOT_TOKEN}
      - GHOST_ALPHA_TELEGRAM_CHAT_ID=${TELEGRAM_CHAT_ID}
      - GHOST_ALPHA_DISCORD_WEBHOOK_URL=${DISCORD_WEBHOOK_URL}
      - GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db
    volumes:
      - ghost-alpha-data:/data
    command: ["screen", "--top", "20"]
    # Run daily at 18:00 UTC (after US market close)
    # Use cron or systemd timer to trigger: docker compose run ghost-alpha

volumes:
  ghost-alpha-data:
```

### 3. GitHub Actions for image publishing

Create `.github/workflows/docker.yml`:

```yaml
name: Docker
on:
  push:
    tags: ["v*"]
  workflow_dispatch:

jobs:
  build:
    runs-on: ubuntu-latest
    permissions:
      contents: read
      packages: write
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/build-push-action@v5
        with:
          push: true
          tags: ghcr.io/${{ github.repository }}:latest
          cache-from: type=gha
          cache-to: type=gha,mode=max
```

### 4. Usage examples for README

```bash
# Quick screen (no setup needed)
docker run --rm \
  -e GHOST_ALPHA_FINNHUB_API_KEY=your_key \
  ghcr.io/user/ghost-alpha screen --top 10

# With persistent database
docker run --rm \
  -v ghost-alpha-data:/data \
  -e GHOST_ALPHA_FINNHUB_API_KEY=your_key \
  -e GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db \
  ghcr.io/user/ghost-alpha screen

# Demo mode (no API key needed)
docker run --rm ghcr.io/user/ghost-alpha demo
```

## Files to create/modify

| File | Action |
|------|--------|
| `Dockerfile` | Create |
| `docker-compose.yml` | Create |
| `.dockerignore` | Create |
| `.github/workflows/docker.yml` | Create |
| `README.md` | Modify — add Docker quickstart section |

## Image size target

- Build stage: ~2GB (Rust toolchain, discarded)
- Final image: **~150MB** (binary + ONNX model + minimal Debian)
- Without sentiment: ~30MB

## Testing

- CI builds image on every tag push
- Smoke test: `docker run ghost-alpha demo` exits 0
- Integration test: `docker run ghost-alpha screen` with test API key

## Acceptance criteria

- [ ] `docker run ghcr.io/.../ghost-alpha demo` works with zero config
- [ ] `docker run -e GHOST_ALPHA_FINNHUB_API_KEY=... ghcr.io/.../ghost-alpha screen` works
- [ ] Image published automatically on git tag
- [ ] Image size < 200MB (with sentiment), < 50MB (without)
- [ ] docker-compose.yml works for scheduled operation
