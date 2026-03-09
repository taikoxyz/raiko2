# Raiko2 runtime image for Docker and Docker Compose deployments.
# This image intentionally excludes SGX-specific setup.

FROM rust:1.93.0-bookworm AS builder

ARG BIN_FEATURES=""

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    libssl-dev \
    pkg-config \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock rust-toolchain ./
COPY crates ./crates
COPY bin ./bin
COPY xtask ./xtask
COPY config ./config
COPY config.example.toml ./

RUN cargo build --release -p raiko2 ${BIN_FEATURES}

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    curl && \
    rm -rf /var/lib/apt/lists/*

RUN mkdir -p /etc/raiko2

COPY --from=builder /app/target/release/raiko2 /usr/local/bin/raiko2
COPY --from=builder /app/config/chain_spec_list_default.json /etc/raiko2/chain_spec_list_default.json
COPY --from=builder /app/config.example.toml /etc/raiko2/config.example.toml

ENV RAIKO2_HOST=0.0.0.0
ENV RAIKO2_PORT=8080
ENV RAIKO2_CONFIG=/etc/raiko2/config.toml
ENV RUST_LOG=info

EXPOSE 8080

ENTRYPOINT ["raiko2"]
CMD []
