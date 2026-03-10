# Operations Runbook

## Scope

This runbook is the primary Day 10 operations guide for the delivery worker.

Use it for:
- worker health checks,
- failure investigation,
- safe manual retries from the DLQ,
- webhook config verification and updates.

Related docs:
- [troubleshooting.md](/Users/yizehu/Workspaces/HooRay-Relay/docs/troubleshooting.md)
- [WORKER_RUNTIME.md](/Users/yizehu/Workspaces/HooRay-Relay/docs/WORKER_RUNTIME.md)
- [retry-behavior.md](/Users/yizehu/Workspaces/HooRay-Relay/docs/retry-behavior.md)

## Worker Health Checks

### Required runtime configuration

The worker currently requires:
- `AWS_REGION`
- `QUEUE_URL` or `WEBHOOK_QUEUE_URL`
- `EVENTS_TABLE` or `WEBHOOK_EVENTS_TABLE`
- `CONFIGS_TABLE` or `WEBHOOK_CONFIGS_TABLE`
- `BREAKER_STATES_TABLE` or `BREAKER_STATE_TABLE`

### ECS service health

```bash
aws ecs describe-services \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE" \
  --query "services[0].{desired:desiredCount,running:runningCount,pending:pendingCount,status:status,rollout:deployments[0].rolloutState}" \
  --output table
```

Healthy baseline:
- `desired` matches `running`
- `pending=0`
- `status=ACTIVE`
- `rollout=COMPLETED`

### Queue health

```bash
aws sqs get-queue-attributes \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --queue-url "$MAIN_QUEUE_URL" \
  --attribute-names ApproximateNumberOfMessages ApproximateNumberOfMessagesNotVisible
```

```bash
aws sqs get-queue-attributes \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --queue-url "$DLQ_URL" \
  --attribute-names ApproximateNumberOfMessages
```

Healthy baseline:
- main queue visible depth stays low or drains
- not-visible messages rise only while worker is actively processing
- DLQ depth remains flat on happy path

### Logs and recent delivery activity

```bash
aws logs tail "/ecs/${ECS_SERVICE}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --since 15m
```

Look for:
- steady polling activity,
- delivery attempt logs,
- no repeating auth, network, or config errors.

## Investigating Delivery Failures

1. Confirm worker health and queue depth.
2. Identify the dominant failure class from logs or DLQ inspection.
3. Inspect the DynamoDB event record and latest attempt rows.
4. Fix the root cause before any replay.
5. Replay a small batch first and verify outcomes.

### Inspect one event in DynamoDB

```bash
EVENT_PK="EVENT#<event_id>"

aws dynamodb query \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --table-name "$EVENTS_TABLE" \
  --key-condition-expression "pk = :pk" \
  --expression-attribute-values '{":pk":{"S":"'"$EVENT_PK"'"}}'
```

Check:
- `sk=v0` current `status`
- `attempt_count`
- latest `ATTEMPT#n`
- `http_status`
- `error_message`

## Manual Retry of Failed Events

Manual retry should be done by replaying messages from the DLQ back to the main queue.
Do not use direct DynamoDB edits as the primary recovery path.

### Safe retry workflow

1. Run inspect mode.
2. Confirm the failure is understood and fixed.
3. Dry-run replay for a small set of IDs.
4. Replay for real.
5. Verify queue consumption, attempt rows, and final event state.

Inspect:

```bash
MODE=inspect ./scripts/dlq_ops.sh
```

Dry-run replay:

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=true ./scripts/dlq_ops.sh
```

Real replay:

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=false ./scripts/dlq_ops.sh
```

Replay and delete original DLQ messages only after verification:

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=false DELETE_AFTER_REPLAY=true ./scripts/dlq_ops.sh
```

## Webhook Config Verification and Updates

The worker reads config from `webhook_configs_{env}` on each attempt.
If deliveries fail due to bad URL, inactive config, or wrong secret, fix the config first.

### Check current config

```bash
aws dynamodb get-item \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --table-name "$CONFIGS_TABLE" \
  --key '{"pk":{"S":"CUSTOMER#<customer_id>"},"sk":{"S":"CONFIG"}}'
```

Verify:
- `url`
- `secret`
- `active`
- `max_retries`
- `updated_at`

### Update config through ingestion API

Preferred path is the ingestion API, not ad hoc DynamoDB edits:

```bash
curl -sS -X POST "${API_URL}webhooks/configs" \
  -H 'content-type: application/json' \
  -d '{"customer_id":"<customer_id>","url":"https://example.com/webhooks","secret":"whsec_new","active":true,"max_retries":3}'
```

After config changes:
- verify the stored record,
- submit one small validation event,
- replay only the affected failed events.

## DLQ Triage and Replay (`scripts/dlq_ops.sh`)

### Purpose

Use `scripts/dlq_ops.sh` to:
- inspect DLQ messages,
- summarize root-cause buckets,
- replay selected messages safely (dry-run by default).

### Defaults

The script defaults to dev settings:
- `STACK_NAME=hooray-dev`
- `AWS_REGION=us-west-2`
- `AWS_PROFILE=hooray-dev`
- `MODE=inspect`
- `DRY_RUN=true`

It auto-resolves `DLQ_URL`, `MAIN_QUEUE_URL`, and `EVENTS_TABLE` from CloudFormation stack outputs.

### Inspect DLQ

```bash
MODE=inspect ./scripts/dlq_ops.sh
```

Output includes:
- table of current batch (`MESSAGE_ID`, `EVENT_ID`, `ERROR_CLASS`, receive count),
- root-cause bucket summary.

### Replay (Safe Dry Run)

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=true ./scripts/dlq_ops.sh
```

This validates candidate selection without sending or deleting anything.

### Replay for Real (Keep DLQ Original)

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=false ./scripts/dlq_ops.sh
```

This sends selected messages to the main queue and keeps original DLQ messages.

### Replay and Delete from DLQ

```bash
MODE=replay REPLAY_MESSAGE_IDS="id1,id2" DRY_RUN=false DELETE_AFTER_REPLAY=true ./scripts/dlq_ops.sh
```

This sends selected messages to the main queue, then deletes them from DLQ.

### Useful Overrides

```bash
STACK_NAME=hooray-dev \
AWS_REGION=us-west-2 \
AWS_PROFILE=hooray-dev \
MAX_MESSAGES=10 \
WAIT_TIME_SECS=2 \
VISIBILITY_TIMEOUT_SECS=30 \
MODE=inspect ./scripts/dlq_ops.sh
```

You can bypass stack output lookup by setting:
- `DLQ_URL`
- `MAIN_QUEUE_URL`
- `EVENTS_TABLE`

### Notes and Caveats

- `REPLAY_MESSAGE_IDS` must exist in the script's current receive batch.
- If IDs change between runs, rerun inspect and use current IDs.
- The script uses temporary files in `/tmp` and cleans them via `trap`.

## Preconditions Checklist

- Confirm AWS login/session is valid:
  - `aws sts get-caller-identity --profile "$AWS_PROFILE"`
- Confirm target stack/environment:
  - `STACK_NAME`, `AWS_REGION`, `AWS_PROFILE`
- Confirm required outputs exist for the stack:
  - `DLQUrl`, `QueueUrl`, `EventsTableName`
- Confirm operator has permissions:
  - `sqs:ReceiveMessage`, `sqs:DeleteMessage`, `sqs:SendMessage`
  - `cloudformation:DescribeStacks`
  - `dynamodb:Query`

## Safety Protocol

- Always run `MODE=inspect` first.
- Always run replay with `DRY_RUN=true` before `DRY_RUN=false`.
- Use `DELETE_AFTER_REPLAY=true` only after dry-run validation.
- Prefer replaying a small set of IDs first, then verify outcomes before bulk replay.

## DLQ Triage Decision Tree

1. `error_class=unknown` and `event_id=unknown`:
   - Likely malformed/legacy payload or parsing mismatch.
   - Action: inspect raw message body; avoid bulk replay until understood.
2. Retryable/transient classes (`http_server_error`, `network_*`, `http_rate_limited`):
   - Action: replay after endpoint/service health is restored.
3. Terminal classes (`http_client_error`, config missing/inactive):
   - Action: fix customer config/payload issue first, then replay.
4. Internal/storage classes (`dynamodb_error`, `sqs_error`, `internal_error`):
   - Action: escalate to platform/infra owner before replaying at scale.

## Verification After Replay

- Confirm message sent to main queue:
  - `aws sqs get-queue-attributes --queue-url "$MAIN_QUEUE_URL" --attribute-names ApproximateNumberOfMessages`
- Confirm worker processed it (logs/metrics).
- Confirm DynamoDB event state/attempt:
  - `pk=EVENT#<event_id>, sk=v0`
  - `pk=EVENT#<event_id>, sk=ATTEMPT#<n>`
- Confirm DLQ depth trend after action:
  - should decrease/stabilize, not continue climbing for same root cause.

## Rollback and Containment

- Stop further replay operations immediately if failures spike.
- Keep `DELETE_AFTER_REPLAY=false` until successful verification.
- Temporarily disable failing customer config if endpoint is unhealthy.
- Scale worker cautiously only after confirming downstream endpoint capacity.

## Incident Logging Template

Record for each replay operation:
- Timestamp (UTC)
- Operator
- Stack/region/profile
- Message IDs / event IDs
- Exact command run
- Dry-run result and real replay result
- Verification outcome
- Follow-up owner and ETA

## Batch Replay Playbook

- Start with small batches (`MAX_MESSAGES=10` or less).
- Replay in waves, verify between waves.
- Avoid large bursts to prevent endpoint thundering herd.
- Prefer oldest/highest receive-count items first.

## Known Limitations

- Replay selection is based on IDs from the current receive batch.
- IDs may rotate between runs due to visibility timing.
- `event_id` can appear as `unknown` for bodies that do not match expected decode path.

## Escalation Path

- Platform/Infra owner:
  - AWS permission issues, stack/output mismatches, queue anomalies.
- Delivery worker owner:
  - classification mismatches, replay script issues, worker processing failures.
- Customer integration owner:
  - persistent endpoint 4xx/5xx due to customer-side issues.

## Command Cookbook

Inspect:
```bash
MODE=inspect ./scripts/dlq_ops.sh
```

Dry-run replay for one ID:
```bash
MODE=replay REPLAY_MESSAGE_IDS="id1" DRY_RUN=true ./scripts/dlq_ops.sh
```

Replay one ID (keep in DLQ):
```bash
MODE=replay REPLAY_MESSAGE_IDS="id1" DRY_RUN=false ./scripts/dlq_ops.sh
```

Replay and delete from DLQ:
```bash
MODE=replay REPLAY_MESSAGE_IDS="id1" DRY_RUN=false DELETE_AFTER_REPLAY=true ./scripts/dlq_ops.sh
```

Inspect with explicit stack/profile/region:
```bash
STACK_NAME=hooray-dev AWS_PROFILE=hooray-dev AWS_REGION=us-west-2 MODE=inspect ./scripts/dlq_ops.sh
```

Run Day 8 scenario suite:
```bash
AWS_REGION=us-west-2 AWS_PROFILE=hooray-dev STACK_NAME=hooray-dev ./scripts/e2e_day8_dlq_ops.sh
```

Run Day 8 suite without long DLQ wait:
```bash
RUN_LONG_DLQ_SCENARIO=false ./scripts/e2e_day8_dlq_ops.sh
```

### Day 8 Scenario 4 Note (Long-Running)

`Scenario 4` in `scripts/e2e_day8_dlq_ops.sh` validates:
- endpoint outage (`5xx`) leads to message transition into DLQ.

This is intentionally slower because DLQ redrive happens only after retry cycles.
With current defaults (`SqsVisibilityTimeoutSeconds=60`, `SqsMaxReceiveCount=4`), this can take several minutes.

Use fast mode when you only want non-DLQ-transition checks:
```bash
RUN_LONG_DLQ_SCENARIO=false ./scripts/e2e_day8_dlq_ops.sh
```
