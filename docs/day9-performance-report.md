# Day 9 Performance Report

## Scope
Performance validation for delivery pipeline using existing tooling:

- `tests/load_test.js`
- `scripts/day9_seed_and_report.sh`

## Test Runs

| Run Type | TEST_RUN_ID | Target Events | Iteration VUs | Endpoint Profile | Artifact Dir |
|---|---|---:|---:|---|---|
| Baseline | `day9_baseline_...` | 1000 | 50 | healthy | `artifacts/day9/<TEST_RUN_ID>/` |
| Post-tuning | `day9_post_tuning_...` | 1000 | 50 | healthy | `artifacts/day9/<TEST_RUN_ID>/` |
| Soak | `day9_soak_...` | (ramping) | n/a | healthy | `artifacts/day9/<TEST_RUN_ID>/` |

## Commands Used

### 1) Baseline

```bash
source .envrc
TEST_RUN_ID=day9_baseline_$(date +%Y%m%d_%H%M%S) \
TARGET_EVENTS=1000 \
ITERATION_VUS=50 \
ENDPOINT_PROFILE=healthy \
OUT_DIR=artifacts/day9/${TEST_RUN_ID} \
./scripts/day9_seed_and_report.sh
```

### 2) Post-tuning

```bash
source .envrc
TEST_RUN_ID=day9_post_tuning_$(date +%Y%m%d_%H%M%S) \
TARGET_EVENTS=1000 \
ITERATION_VUS=50 \
ENDPOINT_PROFILE=healthy \
OUT_DIR=artifacts/day9/${TEST_RUN_ID} \
./scripts/day9_seed_and_report.sh
```

### 3) Soak (30 min)

```bash
source .envrc
TEST_RUN_ID=day9_soak_$(date +%Y%m%d_%H%M%S)
OUT_DIR=artifacts/day9/${TEST_RUN_ID}
mkdir -p "$OUT_DIR"

k6 run \
  --env API_URL="$API_URL" \
  --env API_KEY="${API_KEY:-}" \
  --env CUSTOMER_ID="cust_load_test_${TEST_RUN_ID}" \
  --env TEST_RUN_ID="$TEST_RUN_ID" \
  --env ENDPOINT_PROFILE=healthy \
  --env START_VUS=0 \
  --env STAGE_1_DURATION=5m --env STAGE_1_TARGET=50 \
  --env STAGE_2_DURATION=20m --env STAGE_2_TARGET=100 \
  --env STAGE_3_DURATION=5m --env STAGE_3_TARGET=0 \
  --env SUMMARY_JSON_PATH="$OUT_DIR/k6-summary.json" \
  tests/load_test.js | tee "$OUT_DIR/k6.log"
```

## Artifact Checklist

For each run, verify these files exist in `artifacts/day9/<TEST_RUN_ID>/`:

- `k6-summary.json`
- `k6.log`
- `dynamodb-counts.json` (seed runs)
- `meta.env` (seed runs)
- `env.snapshot` (seed runs)

## Before/After Comparison

| Metric | Baseline | Post-tuning | Soak | Notes |
|---|---:|---:|---:|---|
| accepted_rate |  |  |  |  |
| http_req_failed rate |  |  |  |  |
| http_req_duration p95 (ms) |  |  |  |  |
| Seeded event count |  |  | n/a |  |
| Processed/ack rate (events/min) |  |  |  | CloudWatch/SQS |
| DLQ delta |  |  |  | CloudWatch/SQS |
| ECS CPU (%) |  |  |  |  |
| ECS Memory (%) |  |  |  |  |

## Bottleneck Analysis

### DynamoDB
- Observations:
- Suspected hot paths:
- Mitigations applied:

### HTTP Delivery Path
- Timeout/5xx profile:
- Connection reuse behavior:
- Mitigations applied:

### Worker / SQS Internals
- Poll cadence:
- Concurrency behavior:
- Visibility/backoff interaction:
- Mitigations applied:

## Tuning Delta Log

List exact changes applied between baseline and post-tuning:

1. 
2. 
3. 

## Day 9 Exit Criteria Check

- [ ] Sustained **500+ events/min** on baseline profile
- [ ] **p95 end-to-end delivery latency < 5s** at steady state
- [ ] **99.9% success** for valid endpoints (excluding intentional failures)
- [ ] No unexpected DLQ growth during healthy-endpoint load

## Conclusion

- Result: **Pass / Partial / Fail**
- Key blockers (if any):
- Follow-ups for Day 10:
