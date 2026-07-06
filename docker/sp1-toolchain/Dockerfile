# Keep in sync with rust-toolchain.toml.
ARG RUST_VERSION=1.94.0

# Keep in sync with the sp1-zkvm version in guests/sp1/Cargo.lock.
# For example, sp1-zkvm 6.3.0 in Cargo.lock corresponds to Docker tag v6.3.0 here.
ARG SP1_DOCKER_TAG=v6.3.0
FROM ghcr.io/succinctlabs/sp1:${SP1_DOCKER_TAG}

ENV DEBIAN_FRONTEND=noninteractive

# Re-declare args after FROM so they are available to subsequent RUN steps.
ARG RUST_VERSION
ARG SP1_DOCKER_TAG
ARG SCCACHE_VERSION=v0.16.0
ARG SCCACHE_SHA256_AMD64=aec995a83ad3dff3d14b6314e08858b7b73d35ca85a5bcf3d3a9ec07dee35588
ARG SCCACHE_SHA256_ARM64=f73a5c39f96bb6ebb89cc7915cf182260d4cbf30765322c5e793d0fe8bd80784
ARG TARGETARCH

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        binutils-riscv64-unknown-elf \
        gcc-riscv64-unknown-elf \
        picolibc-riscv64-unknown-elf \
    && rm -rf /var/lib/apt/lists/*

RUN set -eux; \
    arch="${TARGETARCH:-amd64}"; \
    case "$arch" in \
        amd64) target="x86_64-unknown-linux-musl"; expected="${SCCACHE_SHA256_AMD64}" ;; \
        arm64) target="aarch64-unknown-linux-musl"; expected="${SCCACHE_SHA256_ARM64}" ;; \
        *) echo "unsupported TARGETARCH: $arch" >&2; exit 1 ;; \
    esac; \
    archive="sccache-${SCCACHE_VERSION}-${target}.tar.gz"; \
    url="https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${archive}"; \
    tmp="$(mktemp)"; \
    curl -fsSL "$url" -o "$tmp"; \
    echo "${expected}  ${tmp}" | sha256sum -c -; \
    tar -xzf "$tmp" -C /tmp; \
    mv "/tmp/sccache-${SCCACHE_VERSION}-${target}/sccache" /usr/local/bin/sccache; \
    chmod +x /usr/local/bin/sccache; \
    rm -rf "$tmp" "/tmp/sccache-${SCCACHE_VERSION}-${target}"; \
    sccache --version

# Default cross compiler for the guest target. This avoids relying on host toolchains and enables
# building crates with C/C++ sources (e.g. `c-kzg`) under `riscv64im-succinct-zkvm-elf`.
ENV CC_riscv64im_succinct_zkvm_elf="riscv64-unknown-elf-gcc -specs=picolibc.specs"
ENV CXX_riscv64im_succinct_zkvm_elf="riscv64-unknown-elf-g++ -specs=picolibcpp.specs"
ENV AR_riscv64im_succinct_zkvm_elf="riscv64-unknown-elf-ar"

# Install cargo-prove to enable running `cargo prove build` inside this image.
# By default, it matches the SP1 docker tag (e.g. v6.1.0).
ARG CARGO_PROVE_VERSION
#
# Security note: verify the downloaded release artifact checksum to mitigate supply-chain attacks.
# Keep these checksums in sync with the cargo-prove release assets for the default SP1 docker tag.
ARG CARGO_PROVE_SHA256_AMD64=9d25d5ab786a508d8f3e216caa2b51ec46127b3b5b57119ec588f8e3d06401a6
ARG CARGO_PROVE_SHA256_ARM64=5f5b4db7f76ff168c171118dd08c241a7f1bdff28812b2d2b7cd121b0bcbaa49
ARG TARGETARCH
RUN set -eux; \
    version="${CARGO_PROVE_VERSION:-${SP1_DOCKER_TAG}}"; \
    arch="${TARGETARCH:-amd64}"; \
    case "$arch" in \
        amd64) arch="amd64"; expected="${CARGO_PROVE_SHA256_AMD64}" ;; \
        arm64) arch="arm64"; expected="${CARGO_PROVE_SHA256_ARM64}" ;; \
        *) echo "unsupported TARGETARCH: $arch" >&2; exit 1 ;; \
    esac; \
    if [ -z "$expected" ]; then \
        echo "missing cargo-prove checksum for TARGETARCH=$arch" >&2; \
        exit 1; \
    fi; \
    url="https://github.com/succinctlabs/sp1/releases/download/${version}/cargo_prove_${version}_linux_${arch}.tar.gz"; \
    tmp="$(mktemp)"; \
    curl -fsSL "$url" -o "$tmp"; \
    echo "${expected}  ${tmp}" | sha256sum -c -; \
    tar -xzC /usr/local/bin -f "$tmp"; \
    rm "$tmp"; \
    chmod +x /usr/local/bin/cargo-prove; \
    cargo prove --version; \
    rustc +succinct --version

# Pre-install the host Rust toolchain for this repo to avoid downloading it on every guest build.
RUN rustup toolchain install "${RUST_VERSION}" --profile minimal
