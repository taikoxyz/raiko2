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

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
        binutils-riscv64-unknown-elf \
        gcc-riscv64-unknown-elf \
        picolibc-riscv64-unknown-elf \
    && rm -rf /var/lib/apt/lists/*

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
ARG CARGO_PROVE_SHA256_AMD64=99f0087f581798ec510573baedcd51b97a32c6f6a1f5942612ed252e5285954b
ARG CARGO_PROVE_SHA256_ARM64=8262b0ab26004795c162e8c5bd013265f7cc145aed8d109cd935a3f6b9cd789b
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
