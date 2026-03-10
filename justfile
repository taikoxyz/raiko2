default:
    @just --list

build-guest backend="all" *args:
    cargo run -r -p xtask -- build-guest {{backend}} {{args}}

build-guest-risc0:
    just build-guest risc0

build-guest-sp1:
    just build-guest sp1

build-guest-all:
    just build-guest all

build-sp1-toolchain-image tag="raiko2-sp1-toolchain:local" *args:
    docker build -f docker/sp1-toolchain/Dockerfile -t {{tag}} docker/sp1-toolchain {{args}}

bench-guest backend="sp1" *args:
    cargo run -r -p xtask -- bench-guest {{backend}} {{args}}

release-image backend tag repository="us-docker.pkg.dev/evmchain/images/raiko2" *args:
    cargo run -r -p xtask -- release-image {{backend}} --tag {{tag}} --repository {{repository}} {{args}}

update-alethia-reth:
    cargo update -p alethia-reth-block
    cargo update -p alethia-reth-block --manifest-path=guests/common/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/risc0/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/sp1/Cargo.toml
