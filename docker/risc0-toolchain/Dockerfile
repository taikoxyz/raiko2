# Keep in sync with rust-toolchain.toml.
ARG RUST_VERSION=1.90
FROM rust:${RUST_VERSION}-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ENV HOME=/root
ENV PATH="${HOME}/.cargo/bin:${HOME}/.risc0/bin:${PATH}"

# Keep in sync with Cargo.toml's risc0-zkvm version.
ARG RISC0_VERSION=3.0.4
# Toolchain tags from RISC0 release repositories.
ARG RISC0_RUST_VERSION=r0.1.85.0
ARG RISC0_CPP_VERSION=2024.01.05
ARG RISC0_INSTALL_URL=https://risczero.com/install
ARG RISC0_INSTALL_SHA256=5699878af779351ec0f931fa84c3d5e35263279f66bd915af225f530a77341bf

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        build-essential \
        pkg-config \
        libssl-dev \
        docker.io \
    && rm -rf /var/lib/apt/lists/*

# docker.io provides the docker CLI; the container expects a host docker socket.
RUN set -eux; \
    curl -fsSL "${RISC0_INSTALL_URL}" -o /tmp/risc0-install.sh; \
    echo "${RISC0_INSTALL_SHA256}  /tmp/risc0-install.sh" | sha256sum -c -; \
    bash /tmp/risc0-install.sh; \
    rzup install rust "${RISC0_RUST_VERSION}"; \
    rzup install cpp "${RISC0_CPP_VERSION}"; \
    rzup install r0vm "v${RISC0_VERSION}"; \
    rzup install cargo-risczero "v${RISC0_VERSION}"; \
    rm -f /tmp/risc0-install.sh
