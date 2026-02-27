# HooRay-Relay
Production-grade webhook delivery system built with Rust and AWS. Reliable, scalable, and easy to deploy.

Current runtime split:
- Ingestion API runs as Lambda (SAM-managed).
- Delivery worker runs as a long-running SQS poller (non-Lambda) for MVP.
- See `docs/WORKER_RUNTIME.md` for worker deployment and e2e steps.

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
