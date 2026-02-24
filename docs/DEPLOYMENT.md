# HooRay-Relay — Deployment Guide


---

## 1. Tooling Status

All tools confirmed installed and working on this machine:

| Tool | Version | How installed |
|---|---|---|
| Rust / Cargo | `1.93.1` | `rustup` |
| AWS CLI v2 | `2.33.27` | official `.pkg` from aws.amazon.com |
| AWS SAM CLI | `1.154.0` | `brew install aws-sam-cli` |
| Cargo Lambda | `1.9.0` | `brew install cargo-lambda/tap/cargo-lambda` |

---

## 2. What Is Already in the Repo

### `samconfig.toml`

Pre-configured deployment defaults (committed, do not edit locally):

```toml
version = 0.1

[default.deploy.parameters]
stack_name        = "hooray-dev"
region            = "us-west-2"
profile           = "hooray-dev"     # ← AWS profile name (see Step 3b)
confirm_changeset = true
capabilities      = "CAPABILITY_IAM"
disable_rollback  = false
parameter_overrides = "Environment=dev SqsVisibilityTimeoutSeconds=60 SqsMaxReceiveCount=4"
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
| `WorkerFunction` | Lambda | arm64/al2023, built from `worker/` |

### `Makefile`

```makefile
build-IngestionFunction:  # cargo build -p ingestion → $ARTIFACTS_DIR/bootstrap
build-WorkerFunction:     # cargo build -p worker    → $ARTIFACTS_DIR/bootstrap
```

`sam build` calls these automatically — no manual `cargo` invocation needed.

### `.envrc.example`

Template for local environment variables:

```bash
export AWS_PROFILE="your-aws-profile"   # fill in: hooray-dev
export AWS_REGION="us-west-2"
export AWS_DEFAULT_REGION="$AWS_REGION"
```

---

## 3. Remaining Steps Before First Deploy

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
#            .aws-sam/build/WorkerFunction/bootstrap
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

## 4. Subsequent Deployments

```bash
sam build && sam deploy
```

---

## 5. Running Tests Locally (No AWS Credentials Required)

```bash
source "$HOME/.cargo/env"
cargo test
# All tests should pass.
```
