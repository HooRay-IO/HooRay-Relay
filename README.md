# HooRay-Relay
Production-grade webhook delivery system built with Rust and AWS. Reliable, scalable, and easy to deploy.

Current runtime split:
- Ingestion API runs as Lambda (SAM-managed).
- Delivery worker runs as a long-running SQS poller (non-Lambda) for MVP.
- See `docs/WORKER_RUNTIME.md` for worker deployment and e2e steps.

Release readiness:
- [Release 1.0.0 Go / No-Go Checklist](docs/release-1.0.0-go-no-go-checklist.md)
- [Release 1.0.0 Notes](docs/release-1.0.0-notes.md)
- [Networking Hardening Checklist](docs/networking-hardening-checklist.md)
- [Rollback Runbook](docs/rollback.md)

## Worker Workflow

```mermaid
flowchart TD
    A[Worker run loop] --> B[Poll SQS<br/>wait=20s, max=10]
    B --> C{Messages received?}
    C -->|No| A
    C -->|Yes| D[Process each message]

    D --> E{Valid receipt_handle/body/JSON?}
    E -->|No| F[Delete poison message]
    F --> A
    E -->|Yes| G[Extract event_id]

    G --> H[Fetch event from DynamoDB]
    H --> I{Event exists and non-terminal?}
    I -->|Missing or terminal| J[Delete message]
    J --> A
    I -->|Yes| K[Fetch webhook config]

    K --> L{Config exists and active?}
    L -->|No| M[Mark event failed]
    M --> N[Delete message]
    N --> A
    L -->|Yes| O[Deliver webhook HTTP request]

    O --> P[Record delivery attempt]
    P --> Q[Increment attempt_count]
    Q --> R{Delivery result}

    R -->|Success| S[Mark delivered]
    S --> T[Delete message]
    T --> A

    R -->|Retry| U{Retries exhausted?}
    U -->|No| V[Keep message for SQS retry]
    V --> A
    U -->|Yes| W[Mark failed]
    W --> X[Delete message]
    X --> A

    R -->|Exhausted| Y[Mark failed]
    Y --> Z[Delete message]
    Z --> A
```

## Load Testing (k6)

The shared load test script lives in `tests/load_test.js` and supports multiple
profiles:

- `MODE=steady` for constant-arrival-rate tests (default)
- `MODE=ramping` for ramp-up/ramp-down VU stages
- `MODE=seed` + `TARGET_EVENTS=...` for fixed-volume seeding

Key environment variables:

- `API_URL` (or `BASE_URL`) — ingestion API base URL
- `API_KEY` — API Gateway key (if required)
- `CUSTOMER_ID` — test customer ID
- `SUMMARY_JSON_PATH` — optional JSON summary output path

Example runs:

```bash
# Constant-arrival test at 500 req/sec for 2 minutes
API_URL="https://<api-id>.execute-api.<region>.amazonaws.com/Prod" \
MODE=steady RATE=500 DURATION=2m \
SUMMARY_JSON_PATH="tests/loadtest-summary.json" \
k6 run tests/load_test.js

# Fixed-volume seed run (useful for worker-side throughput checks)
API_URL="https://<api-id>.execute-api.<region>.amazonaws.com/Prod" \
MODE=seed TARGET_EVENTS=1000 ITERATION_VUS=50 \
SUMMARY_JSON_PATH="tests/loadtest-summary.json" \
k6 run tests/load_test.js
```

For Day 9 artifact generation, prefer the committed wrapper:

```bash
API_URL="https://<api-id>.execute-api.<region>.amazonaws.com/Prod" \
TARGET_EVENTS=1000 ITERATION_VUS=50 \
bash scripts/day9_seed_and_report.sh
```

This writes `meta.env`, `k6-summary.json`, and a sanitized `env.snapshot` under `artifacts/day9/<test_run_id>/`.
