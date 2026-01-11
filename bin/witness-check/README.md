## witness-check

CLI tool to validate whether an execution witness is sufficient for stateless validation.

### Usage

Build:

```bash
cargo build -p witness-check
```

Run:

```bash
cargo run -p witness-check -- \
  --rpc-url <RPC_URL> \
  --block-number <BLOCK> \
  --debug-witness-supported <true|false> \
  [--chain-id <CHAIN_ID>]
```

### Notes

- `--debug-witness-supported` must be set to match the RPC's support for
  `debug_executionWitness`.
- If `--chain-id` is omitted, the tool calls `eth_chainId`.
