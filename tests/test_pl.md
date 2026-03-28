## Webhook Module Load & Reliability Test Plan

This plan defines the load-testing and reliability evaluation for HooRay Relay’s webhook ingestion + delivery pipeline. It is tailored for interview-ready storytelling and production readiness.

### Goals
- Validate ingestion throughput and latency under steady, peak, and burst load.
- Confirm delivery reliability with retries, backoff, and circuit breaker behavior.
- Measure queue stability (ingress vs egress), DLQ rate, and recovery time.
- Provide cost-awareness metrics ($/1k events, retries).

### System assumptions
- **Mock server latency:** 50ms average response time.
- **Worker runtime:** ECS tasks (sequential delivery loop per task).
- **Retry behavior:** exponential backoff + jitter, visibility timeout clamp (min 30s, max 1h).
- **Circuit breaker:** open after 5 consecutive failures, recovery timeout 1m.
- **Max retries:** 5 attempts (worker resilience config).

### Test environments
- **Mock server** for delivery endpoint (controlled 2xx/5xx/timeout responses).
- **k6** for ingestion load (HTTP POST to ingestion endpoint).
- **CloudWatch** + EMF for delivery metrics (queue depth, retry delay, breaker states).

---

## Key metrics

### Ingestion
- Accept rate (2xx / total)
- 4xx vs 5xx ratio
- p95/p99 latency
- Idempotency hit rate (duplicates)
- Enqueue success rate

### Delivery
- Attempt → success ratio
- Retry success rate by attempt
- DLQ rate per 1k events
- End-to-end latency (ingest → delivered)

### System stability
- Queue depth trend over time
- Ingress vs egress rate
- Recovery time after burst

### Cost
- $/1k events
- cost per retry

---

## Test scenarios

### 1) Baseline steady load
- **Workers:** 1 (baseline capacity)
- **RPS:** 20
- **Duration:** 5–10 min
- **Expected:**
	- Ingest p95 < 200ms
	- Delivery p95 < 2s
	- <1% 5xx
	- Queue depth stable

### 2) Normal prod load
- **Workers:** 2–4
- **RPS:** 40–80
- **Duration:** 5–10 min
- **Expected:**
	- Queue depth stable or slowly oscillating
	- Retry delays track backoff policy
	- DLQ rate < 0.1%

### 3) Peak load
- **Workers:** 5
- **RPS:** 100
- **Duration:** 3–5 min
- **Expected:**
	- Queue depth rises then stabilizes
	- No sustained error spikes

### 4) Burst + recovery
- **Workers:** 5
- **RPS:** 200 for 30–60s
- **Then:** drop to 20 RPS for 5 min
- **Expected:**
	- Queue depth spikes, then drains within 2–5 min
	- DLQ rate < 0.1%

### 5) Failure injection
- **Workers:** 2–4
- **RPS:** 40–80
- **Failure profile:** 10–20% 500s + timeouts
- **Expected:**
	- Retry attempts increase
	- Breaker opens after 5 failures
	- Recovery within 1–2 backoff windows

### 6) Duplicate storm
- **Workers:** 1–2
- **RPS:** 50
- **Same idempotency key** for all requests
- **Expected:**
	- 200 responses for duplicates
	- No queue explosion

---

## Acceptance criteria (interview-ready)
- Ingestion p95 < 200ms, p99 < 500ms
- End-to-end delivery p95 < 10s, p99 < 30s
- DLQ rate < 0.1% under normal failure conditions
- Queue depth returns to baseline within 5 minutes after burst
- Retry delays match backoff policy (base 5s, multiplier 2x, max 5m)

---

## Deliverables
- k6 summary outputs per scenario (JSON + human summary)
- CloudWatch metrics screenshots (queue depth, retry delay, breaker events)
- Cost estimates per 1k events

---

## k6 scenario mapping

Use the existing `tests/load_test.js` script with environment variables to switch scenario inputs. Replace `API_URL` with the ingestion endpoint.

### Baseline steady load
```bash
API_URL="<INGESTION_URL>" RATE=20 DURATION=10m MODE=steady k6 run tests/load_test.js
```

### Normal prod load
```bash
API_URL="<INGESTION_URL>" RATE=60 DURATION=10m MODE=steady k6 run tests/load_test.js
```

### Peak load
```bash
API_URL="<INGESTION_URL>" RATE=100 DURATION=5m MODE=steady k6 run tests/load_test.js
```

### Burst + recovery
```bash
API_URL="<INGESTION_URL>" BURST_RATE=200 BURST_DURATION=60s RATE=20 DURATION=5m MODE=burst k6 run tests/load_test.js
```

### Failure injection (mock server)
```bash
API_URL="<INGESTION_URL>" RATE=60 DURATION=5m MODE=steady k6 run tests/load_test.js
```
Use the mock server settings to return 10–20% 500s/timeouts.

### Duplicate storm
To simulate a surge of duplicate events, first update `tests/load_test.js` so that it checks for a `FIXED_IDEMPOTENCY_KEY` environment variable and, when present, uses that value for the `idempotency_key` on every request instead of generating a random one.
```

---

## Mock server usage

Run the mock destination for delivery tests (50ms baseline latency):

```bash
python3 tests/mock_server.py
```

Failure injection (10% 500s, 10% timeouts):

```bash
MOCK_FAIL_RATE=0.1 MOCK_TIMEOUT_RATE=0.1 MOCK_TIMEOUT_MS=2000 python3 tests/mock_server.py
```

Environment variables:
- `MOCK_PORT` (default `8089`)
- `MOCK_LATENCY_MS` (default `50`)
- `MOCK_FAIL_RATE` (0.0–1.0)
- `MOCK_TIMEOUT_RATE` (0.0–1.0)
- `MOCK_TIMEOUT_MS` (default `2000`)

---

## Notes & interview narrative
- Emphasize sequential worker loop and scaling via ECS task count.
- Highlight idempotency safeguards and orphaned event recovery considerations.
- Explain why backoff + visibility timeouts reduce thundering herd risk.
- Include recovery time after burst as a resiliency metric.
