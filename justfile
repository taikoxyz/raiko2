default:
    @just --list

build-guest backend="all" *args:
    cargo run -p xtask -- build-guest {{backend}} {{args}}

build-guest-risc0:
    just build-guest risc0

build-guest-sp1:
    just build-guest sp1

build-guest-all:
    just build-guest all

update-alethia-reth:
    cargo update -p alethia-reth-block
    cargo update -p alethia-reth-block --manifest-path=guests/common/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/risc0/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/sp1/Cargo.toml
