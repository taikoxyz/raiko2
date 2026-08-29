default:
    @just --list

install-git-hooks:
    git config core.hooksPath .githooks

check-secrets *args:
    uv run --with cryptography --with eth-utils python scripts/security/check_evm_private_keys.py {{args}}

build-guest backend="all" *args:
    CARGO_PROFILE_RELEASE_OPT_LEVEL=1 \
        cargo run -r -p xtask-build-guest --bin xtask-build-guest -- {{backend}} {{args}}

build-guest-risc0:
    just build-guest risc0

build-risc0-toolchain-image tag="raiko2-risc0-toolchain:local" *args:
    docker build -f docker/risc0-toolchain/Dockerfile -t {{tag}} docker/risc0-toolchain {{args}}

build-guest-sp1:
    just build-guest sp1

build-guest-all:
    just build-guest all

build-sp1-toolchain-image tag="raiko2-sp1-toolchain:local" *args:
    docker build -f docker/sp1-toolchain/Dockerfile -t {{tag}} docker/sp1-toolchain {{args}}

bench-guest backend="sp1" *args:
    cargo run -r -p xtask -- bench-guest {{backend}} {{args}}

check-risc0-zkgas-model:
    PYTHONDONTWRITEBYTECODE=1 "${PYTHON_BIN:?set PYTHON_BIN to an existing virtualenv Python}" scripts/modeling/risc0_zkgas_model.py check

test-risc0-zkgas-model:
    PYTHONDONTWRITEBYTECODE=1 "${PYTHON_BIN:?set PYTHON_BIN to an existing virtualenv Python}" -m unittest discover -s scripts/modeling/tests -v
    PYTHONDONTWRITEBYTECODE=1 "${PYTHON_BIN:?set PYTHON_BIN to an existing virtualenv Python}" -m unittest discover -s experiments/risc0-zkgas/tests -v

update-risc0-zkgas-model fixture_dir config hoodi_samples mainnet_samples:
    PYTHONDONTWRITEBYTECODE=1 "${PYTHON_BIN:?set PYTHON_BIN to an existing virtualenv Python}" scripts/modeling/risc0_zkgas_model.py update \
        --fixture-dir "{{fixture_dir}}" \
        --config "{{config}}" \
        --hoodi-samples "{{hoodi_samples}}" \
        --mainnet-samples "{{mainnet_samples}}"

release-image backend tag repository="us-docker.pkg.dev/evmchain/images/raiko2" *args:
    @if [ "{{backend}}" = "host" ]; then \
        cargo run -r -p xtask --no-default-features --features image-release -- release-image {{backend}} --tag {{tag}} --repository {{repository}} {{args}}; \
    else \
        cargo run -r -p xtask -- release-image {{backend}} --tag {{tag}} --repository {{repository}} {{args}}; \
    fi

update-alethia-reth:
    cargo update -p alethia-reth-block
    cargo update -p alethia-reth-chainspec
    cargo update -p alethia-reth-consensus
    cargo update -p alethia-reth-block --manifest-path=guests/common/Cargo.toml
    cargo update -p alethia-reth-chainspec --manifest-path=guests/common/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/risc0/Cargo.toml
    cargo update -p alethia-reth-chainspec --manifest-path=guests/risc0/Cargo.toml
    cargo update -p alethia-reth-block --manifest-path=guests/sp1/Cargo.toml
    cargo update -p alethia-reth-chainspec --manifest-path=guests/sp1/Cargo.toml
