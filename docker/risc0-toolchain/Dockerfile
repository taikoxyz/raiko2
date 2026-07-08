# Keep in sync with the upstream cargo-risczero 3.0.5 default guest-builder image.
ARG RISC0_GUEST_BUILDER_TAG=r0.1.91.1
FROM risczero/risc0-guest-builder:${RISC0_GUEST_BUILDER_TAG}

ARG RISC0_GUEST_BUILDER_TAG
LABEL org.raiko2.risc0.guest-builder-tag="${RISC0_GUEST_BUILDER_TAG}"

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TERM_COLOR=always

ARG SCCACHE_VERSION=v0.16.0
ARG SCCACHE_SHA256_AMD64=aec995a83ad3dff3d14b6314e08858b7b73d35ca85a5bcf3d3a9ec07dee35588
ARG SCCACHE_SHA256_ARM64=f73a5c39f96bb6ebb89cc7915cf182260d4cbf30765322c5e793d0fe8bd80784
ARG TARGETARCH

# The base image already contains the custom RISC0 toolchain and target.
# We only need git so Cargo can resolve git dependencies from the mounted workspace.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
        git \
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
