# Day 9 Baseline Report

- Run ID: day9_baseline_20260307_010222
- Customer ID: cust_load_test_day9_baseline_1772874137
- Generated At: 2026-03-07T09:03:21Z
- Source files: tests/load_test.js, scripts/day9_seed_and_report.sh

## k6 Summary


auto-generated from: artifacts/day9/day9_baseline_20260307_010222/k6-summary.json


affected threshold outcome: FAILED (accepted_rate/http_req_failed/server_error_rate)


accepted_rate: 0.179
server_error_rate: 0.821
http_req_failed_rate: 0.821
http_req_duration_p95_ms: 110.9462
iterations: 1000
status_accepted: 179

## DynamoDB Status Snapshot

- Table: webhook_events_dev
- Metadata rows (sk=v0): 179
- Delivered: 179
- Failed: 0
- Pending: 0
- Delivered with null delivered_at: 0
- Sum(attempt_count): 179

## Notes
- This baseline used TARGET_EVENTS=1000 and ITERATION_VUS=50 (shared-iterations).
- The ingestion layer was overloaded at this burst level (high 5xx), so seed acceptance was partial.
- Use this as stress baseline; for controlled baseline comparison, rerun with tuned VU/target profile.
