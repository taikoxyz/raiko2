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
