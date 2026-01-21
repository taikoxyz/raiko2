FROM rust:1.90-bookworm

ENV DEBIAN_FRONTEND=noninteractive
ENV PATH="/root/.cargo/bin:/root/.risc0/bin:${PATH}"

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

RUN curl -L https://risczero.com/install | bash \
    && /root/.risc0/bin/rzup install
