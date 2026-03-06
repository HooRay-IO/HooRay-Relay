# Worker Runtime Guide (Non-Lambda)

## Decision

The delivery worker is currently designed as a long-running SQS poller (`run()` + long poll loop), so it should run on non-Lambda compute for MVP.

Valid runtime options:
- ECS/Fargate (recommended managed option for MVP)
- EC2 + `systemd`
- Kubernetes
- Local process for dev/testing

## Required Environment Variables

- `AWS_REGION`
- `QUEUE_URL` (or `WEBHOOK_QUEUE_URL`)
- `EVENTS_TABLE` (or `WEBHOOK_EVENTS_TABLE`)
- `CONFIGS_TABLE` (or `WEBHOOK_CONFIGS_TABLE`)

Optional for e2e testing:
- `DELIVERY_URL`
- `SKIP_DELIVERY=false`

## Build and Run

Build:

```bash
cargo build --release -p worker --bin worker
```

Run:

```bash
RUST_LOG=info ./target/release/worker
```

## ECS/Fargate MVP Rollout

### 1. Build and push image to ECR

```bash
export AWS_REGION=us-west-2
export AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
export ECR_REPO=hooray-relay-worker
export IMAGE_TAG="$(git rev-parse --short HEAD)"

aws ecr describe-repositories --repository-names "$ECR_REPO" --region "$AWS_REGION" >/dev/null 2>&1 || \
aws ecr create-repository --repository-name "$ECR_REPO" --region "$AWS_REGION"

aws ecr get-login-password --region "$AWS_REGION" | \
docker login --username AWS --password-stdin "${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"

docker build -f worker/Dockerfile -t "$ECR_REPO:$IMAGE_TAG" .
docker tag "$ECR_REPO:$IMAGE_TAG" "${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com/${ECR_REPO}:$IMAGE_TAG"
docker push "${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com/${ECR_REPO}:$IMAGE_TAG"
```

### 2. Create ECS task role permissions

Worker task role requires:
- `sqs:ReceiveMessage`, `sqs:DeleteMessage`, `sqs:ChangeMessageVisibility`, `sqs:GetQueueAttributes` on delivery queue
- `dynamodb:GetItem`, `dynamodb:PutItem`, `dynamodb:UpdateItem` on events/configs tables
- `logs:CreateLogStream`, `logs:PutLogEvents` for container logs (if needed by your log driver setup)

### 3. Run as ECS service

- Launch type: Fargate
- Desired count: `1` for MVP
- Networking: private subnets + NAT (or equivalent outbound internet path)
- CPU/Memory start point: `0.25 vCPU / 512 MiB`
- Container env vars: `AWS_REGION`, `QUEUE_URL`, `EVENTS_TABLE`, `CONFIGS_TABLE`
- Autoscaling target: scale out on SQS queue depth
- Use helper artifacts in `infra/ecs/`:
  - `infra/ecs/worker-task-policy.template.json`
  - `infra/ecs/task-definition.template.json`
  - `infra/ecs/deploy_worker_service.sh`

## Full Integration E2E

Preferred one-command full integration flow:

```bash
./scripts/e2e_ingestion_worker.sh
```

Expected success signals:
- Delivery attempt row exists: `pk=EVENT#<event_id>, sk=ATTEMPT#1`
- Event status transitions to `delivered` (or expected failure state)
- Main queue depth does not grow
- DLQ depth does not increase for happy-path runs

Contract-only fallback (seed + enqueue + worker assertions helper):

```bash
SKIP_DELIVERY=false ./worker/tests/end_to_end_test.sh
```

### Full API -> Queue -> Worker -> DynamoDB E2E (validated on February 26, 2026)

```bash
REGION=us-west-2
PROFILE=hooray-dev
STACK=hooray-dev

API_URL=$(aws cloudformation describe-stacks \
  --stack-name "$STACK" --region "$REGION" --profile "$PROFILE" \
  --query "Stacks[0].Outputs[?OutputKey=='IngestionApiUrl'].OutputValue" --output text)

EVENTS_TABLE=$(aws cloudformation describe-stacks \
  --stack-name "$STACK" --region "$REGION" --profile "$PROFILE" \
  --query "Stacks[0].Outputs[?OutputKey=='EventsTableName'].OutputValue" --output text)

TS=$(date +%s)
RAND=$(date +%s%N | tail -c 7)
CUSTOMER_ID="cust_e2e_${TS}_${RAND}"
IDEMPOTENCY_KEY="req_e2e_${TS}_${RAND}"

curl -sS -X POST "${API_URL}webhooks/configs" \
  -H 'content-type: application/json' \
  -d "{\"customer_id\":\"${CUSTOMER_ID}\",\"url\":\"https://httpbin.org/post\",\"secret\":\"whsec_e2e_test\"}"

RESP=$(curl -sS -X POST "${API_URL}webhooks/receive" \
  -H 'content-type: application/json' \
  -d "{\"idempotency_key\":\"${IDEMPOTENCY_KEY}\",\"customer_id\":\"${CUSTOMER_ID}\",\"data\":{\"hello\":\"world\"}}")

EVENT_ID=$(echo "$RESP" | jq -r '.event_id')
EVENT_PK="EVENT#${EVENT_ID}"
```

Poll for worker result:

```bash
aws dynamodb get-item \
  --region "$REGION" --profile "$PROFILE" --table-name "$EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"ATTEMPT#1\"}}" \
  --query 'Item.pk.S' --output text

aws dynamodb get-item \
  --region "$REGION" --profile "$PROFILE" --table-name "$EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"v0\"}}" \
  --query 'Item.status.S' --output text
```

Expected values:
- Attempt item query returns `EVENT#<event_id>`
- Status query returns `delivered`

## Operational Checks

- Service logs: verify message processing and delivery result logging.
- SQS metrics:
  - `ApproximateNumberOfMessagesVisible`
  - `ApproximateNumberOfMessagesNotVisible`
- DLQ metric:
  - `ApproximateNumberOfMessagesVisible` should remain stable on happy path.
- ECS health:
  - task count remains at desired
  - no frequent task restarts

## DLQ Operations

For DLQ triage and replay workflow, see `docs/runbook.md`.

## Future Migration Note

If moving to SQS-triggered Lambda later, refactor worker from infinite loop to per-batch/per-message Lambda handler first. Keep this runtime until that refactor is complete.
