# ── Frontend Builder ──────────────────────────────────────
FROM node:22-slim AS frontend

WORKDIR /app
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci --ignore-scripts

COPY frontend/ ./
RUN npm run build

# ── Rust Builder ──────────────────────────────────────────
FROM rust:slim-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config libssl-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY benches/ benches/
COPY tests/ tests/

RUN cargo build --release --locked \
    && strip target/release/jia

# ── Runtime ──────────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates libssl3 curl \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --uid 1000 --shell /bin/bash jia

COPY --from=builder /app/target/release/jia /usr/local/bin/jia
COPY --from=frontend /app/dist/ /app/frontend/dist/

RUN mkdir -p /data && chown jia:jia /data
WORKDIR /data
USER jia

ENV JIA_PORT=8080
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
    CMD curl -sf http://localhost:8080/health || exit 1

EXPOSE 8080
ENTRYPOINT ["jia"]
CMD ["gateway", "start"]
