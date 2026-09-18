FROM rust:1-bookworm AS builder
WORKDIR /app

COPY Cargo.toml ./
COPY backend/Cargo.toml backend/Cargo.toml
COPY backend/src backend/src
COPY migrations migrations

RUN cargo build --release -p bidradar-api

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/bidradar-api /usr/local/bin/bidradar-api

EXPOSE 8080
CMD ["bidradar-api"]
