# Troubleshooting Guide

## Scope

This guide covers delivery-worker failures, DLQ triage, and escalation for webhook delivery incidents.

Primary operational workflow:
- Use [runbook.md](./runbook.md) for DLQ inspect/replay commands.
- Use this document to classify issues and decide corrective action.

## Quick Triage

1. Check queue health:
   - Main queue depth
   - DLQ depth
2. Check worker health:
   - ECS service/task status
   - worker logs for recent errors
3. Classify failure:
   - retryable transient
   - terminal customer error
   - internal/storage/platform error
4. Apply action:
   - fix root cause first
   - replay from DLQ only after fix is verified

## Symptom Playbooks

### Worker not processing messages

Checks:
- ECS desired/running task count is healthy.
- Worker logs show polling activity.
- Worker has valid `QUEUE_URL`, `EVENTS_TABLE`, `CONFIGS_TABLE`, `BREAKER_STATES_TABLE`.
- IAM for worker role includes SQS + DynamoDB access.

Actions:
- Restart/roll out worker task if unhealthy.
- Fix env vars and redeploy task definition if misconfigured.
- Fix IAM policy and re-run.

Escalate to:
- Platform/Infra owner if ECS/IAM/networking is broken.

### High failure rate

Checks:
- Top error classes from logs/DLQ summary.
- Recent HTTP statuses (4xx vs 5xx).
- Endpoint availability/latency.

Actions:
- If mostly `http_server_error`/`network_*`: treat as transient; replay after endpoint recovery.
- If mostly `http_client_error`: fix payload/config/URL expectations before replay.
- If config-related (`config_missing`/inactive): fix config first.

Escalate to:
- Customer integration owner for persistent endpoint-side failures.

### Queue backing up

Checks:
- Main queue visible/not-visible counts and trend.
- Worker concurrency/capacity.
- Downstream endpoint latency and error rate.

Actions:
- Remove blockers first (endpoint outage, config errors).
- Scale worker conservatively only after downstream can absorb load.
- Replay DLQ in waves after stabilization.

Escalate to:
- Platform/Infra owner if capacity/scaling constraints persist.

### DLQ messages appearing repeatedly

Checks:
- Use `MODE=inspect ./scripts/dlq_ops.sh`.
- Identify whether failures are transient, terminal, or internal.
- Confirm whether same event/message is re-failing after replay.

Actions:
- Stop replay loop until root cause is fixed.
- Replay with `DRY_RUN=true` first, then small real batch.
- Use `DELETE_AFTER_REPLAY=true` only after successful verification.

Escalate to:
- Delivery worker owner for classification or script behavior mismatches.

## Error-Class Playbook

### Retryable / transient

Classes:
- `http_server_error`
- `http_rate_limited`
- `network_timeout`
- `network_connect`
- `network_request`
- `dynamodb_error`
- `sqs_error`

Action:
- Resolve transient cause, then replay from DLQ in small batches.

### Terminal / customer fix required

Classes:
- `http_client_error`
- `config_missing`
- `config_inactive`
- `transport_other` (case-by-case, often terminal)

Action:
- Fix payload/config/endpoint expectations first.
- Replay only after corrective change is confirmed.

### Invalid or malformed input

Classes:
- `invalid_queue_message`
- `serialization_error`

Action:
- Inspect raw message format and producer contract.
- Avoid bulk replay until message validity is confirmed.

### Internal unknown

Classes:
- `internal_error`
- `unknown` (from DLQ utility fallback)

Action:
- Investigate worker logs and DynamoDB attempt history.
- Escalate before broad replay.

## Verification Checklist After Any Replay

- Main queue receives replayed messages.
- Worker consumes and processes messages.
- Event status transitions as expected in `webhook_events`.
- Attempt rows are written (`ATTEMPT#n`).
- DLQ depth decreases or stabilizes.
- Failure rate does not spike again for same root cause.

## Escalation Path

- Platform/Infra owner:
  - AWS access/policy issues, ECS instability, queue anomalies.
- Delivery worker owner:
  - worker logic, classification, breaker/retry behavior, replay script issues.
- Customer integration owner:
  - endpoint contract violations, persistent customer-side 4xx/5xx.

## Incident Record (Minimum)

- Incident start time (UTC)
- Environment/stack/region
- Affected message IDs and event IDs
- Observed error classes
- Commands/actions executed
- Replay mode used (`dry-run`, delete or keep)
- Outcome and verification evidence
- Follow-up owner and ETA
