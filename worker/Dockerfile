FROM lukemathwalker/cargo-chef:latest-rust-1-bookworm AS chef
WORKDIR /app

FROM chef AS planner
COPY Cargo.toml Cargo.lock ./
COPY ingestion/Cargo.toml ingestion/Cargo.toml
COPY worker/Cargo.toml worker/Cargo.toml
COPY ingestion/src ingestion/src
COPY worker/src worker/src
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --locked --release --recipe-path recipe.json
COPY Cargo.toml Cargo.lock ./
COPY ingestion ingestion
COPY worker worker
RUN cargo build --locked --release -p worker --bin worker

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/worker /usr/local/bin/worker

ENV RUST_LOG=info

CMD ["worker"]
