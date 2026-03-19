# Keep in sync with the upstream cargo-risczero 3.0.5 default guest-builder image.
ARG RISC0_GUEST_BUILDER_TAG=r0.1.88.0
FROM risczero/risc0-guest-builder:${RISC0_GUEST_BUILDER_TAG}

ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_TERM_COLOR=always

# The base image already contains the custom RISC0 toolchain and target.
# We only need git so Cargo can resolve git dependencies from the mounted workspace.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates \
        git \
    && rm -rf /var/lib/apt/lists/*
