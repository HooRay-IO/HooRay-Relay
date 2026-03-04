# HooRay-Relay — Deployment Guide

This guide covers both CI/CD (Day 8) and manual deployment steps. The CI/CD
workflow is the recommended path for staging and production.

---

## 1. CI/CD Pipeline (Day 8)

The workflow in `.github/workflows/deploy.yml` deploys the SAM stack to
**staging** on every push to `main`. It can also be triggered manually for
`staging` or `prod`.

**Pipeline steps:**
1. Run Rust tests (`cargo test --workspace --all-features --locked`).
2. Build the SAM app (`sam build --parallel`).
3. Deploy via `sam deploy` with `--resolve-s3` and parameter overrides.

### Required GitHub secret

- `AWS_DEPLOY_ROLE_ARN` — IAM role ARN for GitHub OIDC to assume.

The role must allow CloudFormation, S3, Lambda, DynamoDB, SQS, API Gateway,
and IAM (named role creation).

### Manual trigger

From GitHub Actions: **Deploy (staging)** → choose `staging` or `prod`.

---

## 2. Tooling Status

All tools confirmed installed and working on this machine:

| Tool | Version | How installed |
|---|---|---|
| Rust / Cargo | `1.93.1` | `rustup` |
| AWS CLI v2 | `2.33.27` | official `.pkg` from aws.amazon.com |
| AWS SAM CLI | `1.154.0` | `brew install aws-sam-cli` |
| Cargo Lambda | `1.9.0` | `brew install cargo-lambda/tap/cargo-lambda` |

---

## 3. What Is Already in the Repo

### `samconfig.toml`

Pre-configured shared defaults (committed, safe for CI):

```toml
version = 0.1

[default.deploy.parameters]
stack_name        = "hooray-dev"
region            = "us-west-2"
profile           = "hooray-dev"     # ← AWS profile name (see Step 3b)
confirm_changeset = true
capabilities      = "CAPABILITY_IAM CAPABILITY_NAMED_IAM"
disable_rollback  = false
parameter_overrides = "Environment=dev SqsVisibilityTimeoutSeconds=60 SqsMaxReceiveCount=4 EnableEcsWorker=false"
```

### `samconfig.local.toml` (local only)

Account-specific ECS values should live in local config, not in committed config.

```bash
cp samconfig.local.toml.example samconfig.local.toml
# Edit WorkerImageUri, EcsSubnetIds, EcsSecurityGroupIds, and profile as needed.
```

### `template.yaml`

SAM template that provisions all AWS infrastructure on deploy:

| Resource | Type | Notes |
|---|---|---|
| `webhook_events_dev` | DynamoDB | GSI1 for retry queue, 30-day TTL, PITR |
| `webhook_idempotency_dev` | DynamoDB | 24-hour TTL |
| `webhook_configs_dev` | DynamoDB | PITR enabled |
| `webhook_delivery_dev` | SQS | Redrive → DLQ |
| `webhook_delivery_dlq_dev` | SQS DLQ | 14-day retention |
| `DLQDepthAlarm` | CloudWatch | Fires when DLQ depth > 0 |
| `IngestionFunction` | Lambda | arm64/al2023, built from `ingestion/` |

### `Makefile`

```makefile
build-IngestionFunction:  # cargo lambda build -p ingestion --arm64 → $ARTIFACTS_DIR/bootstrap
package-worker:           # cargo build -p worker --release (non-Lambda runtime)
```

`sam build` calls `build-IngestionFunction` automatically.
The worker is not deployed via SAM as Lambda in the current architecture.

### `.envrc.example`

Template for local environment variables:

```bash
export AWS_PROFILE="your-aws-profile"   # fill in: hooray-dev
export AWS_REGION="us-west-2"
export AWS_DEFAULT_REGION="$AWS_REGION"
```

---

## 4. Remaining Steps Before First Deploy

### Step 3a — Get access via AWS SSO / IAM Identity Center ⏳ PENDING

Need from the platform/admin team:

- [ ] AWS SSO start URL
- [ ] AWS SSO region (for the Identity Center instance)
- [ ] AWS account ID that contains the `hooray-dev` deploy role
- [ ] SSO role name to assume for deployment (e.g. `HoorayDevDeploymentRole`)
- [ ] Confirm the deploy region (`us-west-2`)
- [ ] The deploy role must allow: CloudFormation, DynamoDB, SQS, Lambda, API Gateway, S3, IAM

### Step 3b — Configure the `hooray-dev` profile via SSO ⏳ PENDING

```bash
aws configure sso --profile hooray-dev
# SSO session name:             hooray-dev
# SSO start URL:                <from Step 3a>
# SSO region:                   <from Step 3a>
# SSO account ID:               <from Step 3a>
# SSO role name:                <from Step 3a>
# CLI default client Region:    us-west-2
# CLI default output format:    json
```

Verify credentials work:

```bash
aws sts get-caller-identity --profile hooray-dev
# { "Account": "...", "UserId": "...", "Arn": "..." }
```

### Step 3c — Set up local `.envrc`

```bash
cp .envrc.example .envrc
# Edit: set AWS_PROFILE=hooray-dev
source .envrc
```

### Step 3d — Build

```bash
source "$HOME/.cargo/env"
sam build
# Expected: "Build Succeeded"
# Artifacts: .aws-sam/build/IngestionFunction/bootstrap
```

### Step 3e — First deploy ⏳ PENDING

```bash
sam build && sam deploy --resolve-s3
```

`--resolve-s3` creates and manages the S3 artifact bucket automatically.
All other settings come from `samconfig.toml`. Confirm the changeset with `y`.

### Step 3f — Verify

```bash
aws cloudformation describe-stacks \
  --stack-name hooray-dev \
  --profile hooray-dev \
  --query "Stacks[0].Outputs" \
  --output table
```

Expected outputs: `EventsTableName`, `IdempotencyTableName`, `ConfigsTableName`,
`QueueUrl`, `DLQUrl`, `IngestionApiUrl`.

---

## 5. Subsequent Deployments

```bash
sam build && sam deploy
```

For ECS worker-enabled deploys (local/dev), use local overrides:

```bash
./scripts/deploy_dev.sh
```

This script uses `samconfig.local.toml` and keeps account-specific values out of CI.

### Known-Good ECS Deploy Checklist (validated on February 26, 2026)

1. Ensure worker image tag exists in ECR.

```bash
aws ecr describe-images \
  --region us-west-2 \
  --repository-name hooray-relay-worker-dev \
  --image-ids imageTag=<IMAGE_TAG>
```

2. Set `WorkerImageUri` in `samconfig.local.toml` to that exact tag.

3. Deploy with local config:

```bash
./scripts/deploy_dev.sh
```

4. Verify ECS service health:

```bash
aws ecs describe-services \
  --region us-west-2 \
  --profile hooray-dev \
  --cluster hooray-relay-worker-dev \
  --services hooray-relay-worker-dev \
  --query "services[0].{desired:desiredCount,running:runningCount,pending:pendingCount,rollout:deployments[0].rolloutState}" \
  --output table
```

Expected result: `desired=1`, `running=1`, `pending=0`, `rollout=COMPLETED`.

## 5. Worker Deployment (ECS/Fargate Recommended)

Default recommendation: deploy worker container on ECS/Fargate.

Build worker binary locally (optional local validation):

```bash
cargo build --release -p worker --bin worker
```

Run worker:

```bash
RUST_LOG=info ./target/release/worker
```

Required worker environment variables:
- `AWS_REGION`
- `QUEUE_URL` (or `WEBHOOK_QUEUE_URL`)
- `EVENTS_TABLE` (or `WEBHOOK_EVENTS_TABLE`)
- `CONFIGS_TABLE` (or `WEBHOOK_CONFIGS_TABLE`)

Container build file:
- `worker/Dockerfile`

Primary rollout steps (ECS):
1. Build + push image to ECR.
2. Create ECS task definition with required env vars and IAM task role.
3. Run ECS service (desired count `1` for MVP), then add autoscaling by SQS depth.

See `docs/WORKER_RUNTIME.md` for exact commands and e2e checks.

## 6. Running Tests Locally (No AWS Credentials Required)

```bash
source "$HOME/.cargo/env"
cargo test
# All tests should pass.
```
