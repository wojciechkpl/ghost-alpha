# ── Stage 1: Build ────────────────────────────────────────────
FROM rust:1.85-bookworm AS builder

WORKDIR /app

# Copy dependency manifests first for layer caching.
# A rebuild is only triggered when Cargo.toml or Cargo.lock change.
COPY Cargo.toml Cargo.lock ./

# Create a stub main so `cargo build` can resolve and cache all crate deps
# before the real source is copied in.
RUN mkdir -p src/bin && \
    echo 'fn main() {}' > src/main.rs && \
    echo 'fn main() {}' > src/bin/ghost-alpha-mcp.rs && \
    touch src/lib.rs && \
    cargo build --release 2>/dev/null; \
    rm -rf src

# Now copy the real source tree and rebuild (only the workspace crate is dirty)
COPY src/       src/
COPY migrations/ migrations/

RUN touch src/main.rs src/lib.rs && \
    cargo build --release

# ── Stage 2: Runtime ─────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/ghost-alpha /usr/local/bin/ghost-alpha

# Default DB path inside a mounted volume
ENV GHOST_ALPHA_DB_PATH=/data/ghost_alpha.db

VOLUME ["/data"]

ENTRYPOINT ["ghost-alpha"]
CMD ["screen"]
