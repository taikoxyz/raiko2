# Keep in sync with rust-toolchain.toml.
ARG RUST_VERSION=1.93

# Keep in sync with guests/sp1/Cargo.lock (sp1-zkvm version).
ARG SP1_DOCKER_TAG=v5.2.4
FROM ghcr.io/succinctlabs/sp1:${SP1_DOCKER_TAG}

ENV DEBIAN_FRONTEND=noninteractive

# Re-declare args after FROM so they are available to subsequent RUN steps.
ARG RUST_VERSION
ARG SP1_DOCKER_TAG

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
    && rm -rf /var/lib/apt/lists/*

# Install cargo-prove to enable running `cargo prove build` inside this image.
# By default, it matches the SP1 docker tag (e.g. v5.2.4).
ARG CARGO_PROVE_VERSION
ARG TARGETARCH
RUN set -eux; \
    version="${CARGO_PROVE_VERSION:-${SP1_DOCKER_TAG}}"; \
    arch="${TARGETARCH:-amd64}"; \
    case "$arch" in \
        amd64) arch="amd64" ;; \
        arm64) arch="arm64" ;; \
        *) echo "unsupported TARGETARCH: $arch" >&2; exit 1 ;; \
    esac; \
    url="https://github.com/succinctlabs/sp1/releases/download/${version}/cargo_prove_${version}_linux_${arch}.tar.gz"; \
    curl -fsSL "$url" | tar -xzC /usr/local/bin; \
    chmod +x /usr/local/bin/cargo-prove; \
    cargo-prove prove --version; \
    rustc +succinct --version

# Pre-install the host Rust toolchain for this repo to avoid downloading it on every guest build.
RUN rustup toolchain install "${RUST_VERSION}" --profile minimal
