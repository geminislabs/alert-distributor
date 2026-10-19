FROM rust:bookworm AS chef

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      build-essential \
      cmake \
      pkg-config \
      libssl-dev \
      libsasl2-dev \
      libcurl4-openssl-dev \
      zlib1g-dev \
      librdkafka-dev \
      clang \
      make \
    && rm -rf /var/lib/apt/lists/*

RUN cargo install cargo-chef

WORKDIR /app

FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
      ca-certificates \
      curl \
      libssl3 \
      libsasl2-2 \
      librdkafka1 \
      tzdata \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/alert-distributor /usr/local/bin/alert-distributor

EXPOSE 8080

# 503 si el consumidor de control-plane-events no puede leer (ADR-008 §9 de
# siscom-admin-api). El puerto sale de WS_BIND_ADDR (host:puerto).
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 \
    CMD curl -fsS "http://127.0.0.1:${WS_BIND_ADDR##*:}/health" > /dev/null || exit 1

CMD ["/usr/local/bin/alert-distributor"]
