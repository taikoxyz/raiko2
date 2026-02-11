# Raiko V2 API Documentation

## Overview

Raiko V2 provides a REST API for requesting and managing zkVM proofs for Taiko's Shasta hardfork.

## Base URL

```
http://localhost:8080
```

## Authentication

Currently no authentication is required. Production deployments should add authentication.

## Endpoints

### Health Check

```http
GET /health
```

#### Response

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

### Readiness Check

```http
GET /ready
```

#### Response

```json
{
  "status": "ok",
  "reth": { "ok": true },
  "queue": { "ok": true }
}
```

If dependencies are unavailable, returns HTTP 503 with error details per subsystem.

### Server Info

```http
GET /v1/info
```

#### Response

```json
{
  "version": "0.1.0",
  "prover": "Risc0",
  "supported_provers": ["risc0", "sp1", "native", "agent-risc0"]
}
```

`supported_provers` is computed from currently registered pipelines and may differ by deployment.

### Request Proposal Proof

```http
POST /v1/proof/proposal
Content-Type: application/json
```

#### Request Body

| Field         | Type     | Required | Description                                                                      |
| ------------- | -------- | -------- | -------------------------------------------------------------------------------- |
| `proposal_id` | `u64`    | Yes      | The proposal ID to prove                                                         |
| `prover_type` | `string` | No       | Prover type: "risc0", "sp1", "native", or "agent-risc0" (defaults to config)    |

Unknown request fields are rejected.

#### Example Request

```json
{
  "proposal_id": 12345,
  "prover_type": "agent-risc0"
}
```

#### Response

```json
{
  "id": "<proof_id>",
  "status": "pending"
}
```

The `proof_id` is an opaque URL-safe base64 string. Treat it as an opaque identifier.

#### Status Codes

| Code | Description                |
| ---- | -------------------------- |
| 200  | Proof request accepted     |
| 400  | Invalid request parameters |
| 500  | Internal server error      |

### Get Proof Status

```http
GET /v1/proof/{proof_id}
```

#### Path Parameters

| Parameter  | Type     | Description                                  |
| ---------- | -------- | -------------------------------------------- |
| `proof_id` | `string` | The proof ID returned from the proposal request (opaque URL-safe base64) |

#### Response

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "completed",
  "proof": "0x...",
  "error": null
}
```

#### Proof Status Values

| Status      | Description                                     |
| ----------- | ----------------------------------------------- |
| `pending`   | Proof request received, waiting to be processed |
| `proving`   | Proof generation in progress                    |
| `completed` | Proof successfully generated                    |
| `failed`    | Proof generation failed                         |
| `cancelled` | Proof request was cancelled                     |

### Cancel Proof

```http
POST /v1/proof/{proof_id}/cancel
```

#### Response

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "cancelled"
}
```

Note: the status reflects the task state after the cancel attempt.

## Configuration

### Environment Variables

| Variable                              | Default   | Description                                  |
| ------------------------------------- | --------- | -------------------------------------------- |
| `RAIKO2_HOST`                         | `0.0.0.0` | Server bind address                          |
| `RAIKO2_PORT`                         | `8080`    | Server port                                  |
| `RAIKO2_L1_RPC`                       | -         | L1 RPC endpoint URL                          |
| `RAIKO2_L2_RPC`                       | -         | L2 RPC endpoint URL                          |
| `RAIKO2_PROVER`                       | `risc0`   | Default prover type                          |
| `RAIKO2_L1_CHAIN_ID`                  | `1`       | L1 chain ID                                  |
| `RAIKO2_L2_CHAIN_ID`                  | `167000`  | L2 chain ID (Taiko Mainnet)                  |
| `RAIKO2_RPC_TIMEOUT_MS`               | `10000`   | RPC request timeout (ms)                     |
| `RAIKO2_RPC_CONCURRENCY_LIMIT`        | `32`      | RPC concurrency limit                        |
| `RAIKO2_RPC_RETRY_MAX_ATTEMPTS`       | `3`       | RPC retry max attempts (0 disables retry)    |
| `RAIKO2_RPC_RETRY_INITIAL_BACKOFF_MS` | `500`     | RPC retry initial backoff (ms)               |
| `RAIKO2_RPC_RETRY_CU_PER_SECOND`      | `1000`    | RPC retry CU budget per second               |
| `RAIKO2_CONFIG`                       | -         | Path to config file                          |
| `RAIKO2_QUEUE_BACKEND`                | -         | Queue backend (memory, redis)                |
| `RAIKO2_REDIS_URL`                    | -         | Redis URL (required for redis)               |
| `RAIKO2_QUEUE_NAMESPACE`              | -         | Redis key namespace                          |
| `RAIKO2_QUEUE_WORKERS`                | -         | Worker count                                 |
| `RAIKO2_QUEUE_MAINTENANCE_INTERVAL_MS`| -         | Scheduler maintenance interval (ms)          |
| `RAIKO2_QUEUE_RETRY_STRATEGY`         | -         | none, fixed, exponential                     |
| `RAIKO2_QUEUE_RETRY_MAX_ATTEMPTS`     | -         | Max retry attempts                           |
| `RAIKO2_QUEUE_RETRY_FIXED_DELAY_MS`   | -         | Fixed retry delay (ms)                       |
| `RAIKO2_QUEUE_RETRY_BASE_DELAY_MS`    | -         | Exponential base delay (ms)                  |
| `RAIKO2_QUEUE_RETRY_MAX_DELAY_MS`     | -         | Exponential max delay (ms)                   |
| `RUST_LOG`                            | `info`    | Log level                                    |

### Config File (TOML)

```toml
[server]
host = "0.0.0.0"
port = 8080

[rpc]
l1_rpc = "https://ethereum-rpc.example.com"
l2_rpc = "https://taiko-rpc.example.com"
l1_chain_id = 1
l2_chain_id = 167000

[rpc.client]
timeout_ms = 10000
concurrency_limit = 32

[rpc.client.retry]
max_attempts = 3
initial_backoff_ms = 500
compute_units_per_second = 1000

[prover]
prover_type = "risc0"

[prover.risc0]
bonsai = true
snark = true

[prover.sp1]
network = true
plonk = true

[prover.agent]
url = "http://localhost:9999"
api_key = "optional-api-key"
poll_interval_ms = 1000
timeout_ms = 300000
prover_type = "boundless"

[queue]
backend = "memory"
namespace = "raiko2:queue"
workers = 1
maintenance_interval_ms = 200

[queue.retry]
strategy = "exponential"
max_attempts = 3
fixed_delay_ms = 1000
base_delay_ms = 1000
max_delay_ms = 30000
```

## CLI Usage

```bash
# Start with default settings
raiko2

# Start with custom port
raiko2 --port 9090

# Start with config file
raiko2 --config /etc/raiko/config.toml

# Start with environment overrides
RAIKO2_L1_RPC=https://... RAIKO2_L2_RPC=https://... raiko2

# Enable verbose logging
raiko2 --verbose

# Output JSON logs
raiko2 --json-logs

# Select memory queue backend (default behavior)
raiko2 --queue-backend memory

# Select Redis queue backend (requires build with --features redis-queue)
raiko2 --queue-backend redis --redis-url redis://localhost:6379/ --queue-namespace raiko2:queue
```

## Error Responses

All error responses follow this format:

```json
{
  "error": "Proposal ID 12345 not found on L1"
}
```

## Docker

```bash
# Build image
docker build -f Dockerfile.raiko2 -t raiko2:latest .

# Run container
docker run -d \
  -p 8080:8080 \
  -e RAIKO2_L1_RPC=https://... \
  -e RAIKO2_L2_RPC=https://... \
  raiko2:latest
```

## Examples

### cURL

```bash
# Health check
curl http://localhost:8080/health

# Server info
curl http://localhost:8080/v1/info

# Request proof
curl -X POST http://localhost:8080/v1/proof/proposal \
  -H "Content-Type: application/json" \
  -d '{"proposal_id": 12345, "prover_type": "risc0"}'

# Get proof status
curl http://localhost:8080/v1/proof/<proof_id>

# Cancel proof
curl -X POST http://localhost:8080/v1/proof/<proof_id>/cancel
```

### Python

```python
import requests

# Request proof
response = requests.post(
    "http://localhost:8080/v1/proof/proposal",
    json={
        "proposal_id": 12345,
        "prover_type": "risc0"
    }
)
proof_id = response.json()["id"]

# Poll for completion
while True:
    status = requests.get(f"http://localhost:8080/v1/proof/{proof_id}").json()
    if status["status"] in ["completed", "failed"]:
        break
```
