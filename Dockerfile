# Raiko2 runtime image for Docker and Docker Compose deployments.
# This image intentionally excludes SGX-specific setup.

FROM rust:1.94.0-bookworm AS chef

ARG BIN_FEATURES=""
ARG CARGO_CHEF_VERSION=0.1.77

ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_TOOLCHAIN=1.94.0-x86_64-unknown-linux-gnu

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    libprotobuf-dev \
    libssl-dev \
    protobuf-compiler \
    pkg-config \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

RUN cargo install --locked cargo-chef --version ${CARGO_CHEF_VERSION}

WORKDIR /app

FROM chef AS planner

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY bin ./bin
COPY xtask ./xtask
COPY config ./config
COPY config.example.toml ./
COPY test/guest_inputs ./test/guest_inputs
COPY tests/fixtures ./tests/fixtures

RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY --from=planner /app/recipe.json ./recipe.json
RUN cargo chef cook --release --recipe-path recipe.json -p raiko2 ${BIN_FEATURES}

COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates ./crates
COPY bin ./bin
COPY xtask ./xtask
COPY config ./config
COPY config.example.toml ./
COPY test/guest_inputs ./test/guest_inputs
COPY tests/fixtures ./tests/fixtures

RUN cargo +1.94.0 build --release -p raiko2 ${BIN_FEATURES}

FROM debian:bookworm-slim AS runtime

ARG VCS_REF=unknown
LABEL org.opencontainers.image.revision=$VCS_REF

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    python3 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

RUN mkdir -p /etc/raiko2

COPY --from=builder /app/target/release/raiko2 /usr/local/bin/raiko2
COPY --from=builder /app/crates/guests/elf ./crates/guests/elf
COPY --from=builder /app/config/chain_spec_list_default.json /etc/raiko2/chain_spec_list_default.json
COPY --from=builder /app/config.example.toml /etc/raiko2/config.example.toml

ENV RAIKO2_HOST=0.0.0.0
ENV RAIKO2_PORT=8080
ENV RAIKO2_CONFIG=/etc/raiko2/config.toml
ENV RAIKO2_GUEST_ELF_DIR=/app/crates/guests/elf
ENV RUST_LOG=info

EXPOSE 8080

ENTRYPOINT ["raiko2"]
CMD []
