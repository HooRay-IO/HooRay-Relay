# Engineer 1 → Engineer 2 Handoff Document

**Date:** Day 5 of 10  
**From:** Engineer 1 (Ingestion Pipeline)  
**To:** Engineer 2 (Delivery Worker)  
**Status:** Ingestion API fully deployed and passing integration tests

---

## Table of Contents

1. [What Engineer 1 Has Built](#1-what-engineer-1-has-built)
2. [Infrastructure Overview](#2-infrastructure-overview)
3. [SQS Message Contract](#3-sqs-message-contract)
4. [DynamoDB Schemas](#4-dynamodb-schemas)
5. [API Endpoints](#5-api-endpoints)
6. [Sample AWS CLI Queries](#6-sample-aws-cli-queries)
7. [What Engineer 2 Needs to Build](#7-what-engineer-2-needs-to-build)
8. [Integration Test Acceptance Criteria](#8-integration-test-acceptance-criteria)
9. [Change Control](#9-change-control)

---

## 1. What Engineer 1 Has Built

The ingestion pipeline is complete and deployed. It handles the full lifecycle
from receiving a webhook from an external caller to placing it on the SQS queue
for you to deliver.

| Component | File | Purpose |
|---|---|---|
| Data models + error types | `ingestion/src/model.rs` | All shared types; cross-team wire format |
| DynamoDB client + config | `ingestion/src/services/dynamodb.rs` | `AppConfig::from_env()` + client factory |
| Idempotency service | `ingestion/src/services/idempotency.rs` | Atomic dedup via `attribute_not_exists(pk)` |
| Event storage service | `ingestion/src/services/events.rs` | Writes `webhook_events` with 30d TTL |
| SQS enqueue service | `ingestion/src/services/queue.rs` | Enqueues with `customer_id` message attribute |
| Config CRUD service | `ingestion/src/services/configs.rs` | Reads/writes `webhook_configs` |
| Webhook receive handler | `ingestion/src/handlers/webhook.rs` | `POST /webhooks/receive` |
| Config handlers | `ingestion/src/handlers/config.rs` | `POST/GET /webhooks/configs` |
| Lambda entry point | `ingestion/src/main.rs` | Cold-start, Axum router, `lambda_http::run` |
| SAM template | `template.yaml` | All DynamoDB tables, SQS, Lambdas, API Gateway |
| Integration tests | `ingestion/tests/integration_test.sh` | 10 live-API test cases |

---

## 2. Infrastructure Overview

All infrastructure is defined in `template.yaml` and deployed via AWS SAM.

| Resource | Logical ID | Name pattern |
|---|---|---|
| DynamoDB — events | `WebhookEventsTable` | `webhook_events_{env}` |
| DynamoDB — idempotency | `WebhookIdempotencyTable` | `webhook_idempotency_{env}` |
| DynamoDB — configs | `WebhookConfigsTable` | `webhook_configs_{env}` |
| SQS — main queue | `WebhookDeliveryQueue` | `webhook_delivery_{env}` |
| SQS — dead-letter queue | `WebhookDeliveryDLQ` | `webhook_delivery_dlq_{env}` |
| Lambda — ingestion | `IngestionFunction` | `hooray-relay-ingestion-{env}` |
| Lambda — worker (yours) | `WorkerFunction` | `hooray-relay-worker-{env}` |
| CloudWatch Alarm | `DLQDepthAlarm` | `hooray-relay-dlq-depth-{env}` |

### Environment variables your Lambda receives

These are injected by the `Globals` block in `template.yaml`:

| Variable | Description | Example |
|---|---|---|
| `EVENTS_TABLE` | `webhook_events` table name | `webhook_events_dev` |
| `CONFIGS_TABLE` | `webhook_configs` table name | `webhook_configs_dev` |
| `QUEUE_URL` | SQS delivery queue URL | `https://sqs.us-east-1.amazonaws.com/…` |
| `ENVIRONMENT` | Deployment environment | `dev` |

Note: your worker also reads `IDEMPOTENCY_TABLE` if needed, but Engineer 1 owns
that table — you only need `EVENTS_TABLE` and `CONFIGS_TABLE`.

### Deploy commands

```bash
# First deploy (interactive — sets samconfig.toml)
sam build && sam deploy --guided

# Subsequent deploys
sam build && sam deploy

# With explicit environment
sam build && sam deploy --parameter-overrides Environment=staging
```

---

## 3. SQS Message Contract

This is the interface between Engineer 1 and Engineer 2.
**Do not change this without going through the Change Control process (§9).**

### Message body

```json
{ "event_id": "evt_1a2b3c4d5e6f7g8h" }
```

- The body is **always valid JSON** with exactly one key: `event_id`.
- `event_id` is a string with format `evt_` followed by 16 alphanumeric characters.
- No other fields appear in the body — do not assume they will.

### Message attributes

| Attribute name | DataType | Value |
|---|---|---|
| `customer_id` | `String` | The customer ID (e.g. `cust_xyz123`) |

`customer_id` is in the message attribute (not the body) so you can read the
routing key without a DynamoDB round-trip. Always read it via:

```bash
# CLI example
aws sqs receive-message \
  --queue-url "$QUEUE_URL" \
  --message-attribute-names customer_id \
  --output json
```

```rust
// Rust example — reading from the SDK message
let customer_id = message
    .message_attributes()
    .get("customer_id")
    .and_then(|a| a.string_value())
    .ok_or_else(|| WorkerError::InvalidMessage("missing customer_id attribute".into()))?;
```

### Queue settings (configured in template.yaml)

| Setting | Value | Rationale |
|---|---|---|
| Visibility timeout | 60s | >= max Lambda execution time (55s) |
| Max receive count | 4 | 1 initial attempt + 3 retries = max_retries + 1 |
| Message retention | 4 days | Enough time to investigate failures |
| DLQ retention | 14 days | Investigation window |

### Retry behavior (Week 1 — confirmed in CONTRACT §7)

- Worker relies **entirely on SQS visibility timeout** for retries.
- Do NOT delete the message on failure — let it become visible again.
- `next_retry_at` and `gsi1pk`/`gsi1sk` are **reserved** for a future
  scheduled-retry design; Week 1 worker does not write or read them.

---

## 4. DynamoDB Schemas

### Table 1: `webhook_events_{env}`

This is the shared table. Engineer 1 writes the initial `v0` record;
you write `ATTEMPT#n` records and update the `v0` record on state transitions.

#### Metadata record (SK = `v0`)

Engineer 1 writes this record when a webhook is received.

```json
{
  "pk":            "EVENT#evt_1a2b3c4d",
  "sk":            "v0",
  "event_id":      "evt_1a2b3c4d",
  "customer_id":   "cust_xyz123",
  "payload":       "{\"order_id\":\"ord_123\",\"amount\":99.99}",
  "status":        "pending",
  "attempt_count": 0,
  "created_at":    1707840000,
  "delivered_at":  null,
  "next_retry_at": null
}
```

**Your responsibilities on this record:**

| Transition | When | Fields to update |
|---|---|---|
| `pending → delivered` | 2xx response from customer | `status="delivered"`, `delivered_at=<unix_now>` |
| `pending → pending` (retry) | retryable error, not yet exhausted | `attempt_count+=1` |
| `pending → failed` | retry exhausted OR non-retryable error OR missing/inactive config | `status="failed"` |

**Terminal states:** `delivered` and `failed` are terminal. Once in a terminal
state, do NOT make further status updates. If you receive a duplicate SQS message
for an already-terminal event, skip delivery and delete the message (see §8).

#### Attempt record (SK = `ATTEMPT#n`)

You write one of these per delivery attempt (contract §3).

```json
{
  "pk":              "EVENT#evt_1a2b3c4d",
  "sk":              "ATTEMPT#1",
  "attempt_number":  1,
  "attempted_at":    1707840000,
  "http_status":     503,
  "response_time_ms": 5000,
  "error_message":   "Service Unavailable"
}
```

- `attempt_number` starts at **1** and increments.
- `error_message` is omitted or empty string for successful 2xx attempts.
- Write this **before** updating the `v0` status record.

#### Key format

```
PK = EVENT#{event_id}
SK = v0                   ← metadata
SK = ATTEMPT#1            ← first attempt
SK = ATTEMPT#2            ← second attempt
```

---

### Table 2: `webhook_configs_{env}`

Engineer 1 owns writes. You **read** this table to get the delivery URL and
signing secret before each delivery attempt.

#### Record structure

```json
{
  "pk":          "CUSTOMER#cust_xyz123",
  "sk":          "CONFIG",
  "customer_id": "cust_xyz123",
  "url":         "https://customer.example.com/webhooks",
  "secret":      "whsec_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
  "max_retries": 3,
  "active":      true,
  "created_at":  1707840000,
  "updated_at":  1707840000
}
```

#### Key format

```
PK = CUSTOMER#{customer_id}
SK = CONFIG
```

#### Inactive / missing config behavior (contract §4)

- **Missing config** (`GetItem` returns nothing): mark event `failed`, delete SQS message.
- **`active = false`**: mark event `failed`, delete SQS message.
- Do not retry inactive/missing config — the caller must fix config first.

---

### Table 3: `webhook_idempotency_{env}`

Engineer 1 owns this table entirely. You do not read or write it.
Records expire after 24 hours via DynamoDB TTL.

---

## 5. API Endpoints

The ingestion API is exposed via API Gateway. Base URL is in the SAM Outputs:

```bash
aws cloudformation describe-stacks \
  --stack-name hooray-relay \
  --query "Stacks[0].Outputs[?OutputKey=='IngestionApiUrl'].OutputValue" \
  --output text
```

### POST /webhooks/configs — register a customer endpoint

```bash
curl -X POST "${API_BASE_URL}/webhooks/configs" \
  -H "Content-Type: application/json" \
  -d '{
    "customer_id": "cust_xyz123",
    "url": "https://your-customer.example.com/webhooks",
    "max_retries": 3
  }'
```

Response 201:
```json
{
  "customer_id": "cust_xyz123",
  "url": "https://your-customer.example.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
  "max_retries": 3,
  "active": true,
  "created_at": 1707840000,
  "updated_at": 1707840000
}
```

Notes:
- `secret` is auto-generated if not provided. Use the returned value for HMAC.
- `max_retries` defaults to 3 if not provided.
- Re-posting for the same `customer_id` is an upsert (last write wins).

### GET /webhooks/configs?customer_id=… — fetch a customer config

```bash
curl "${API_BASE_URL}/webhooks/configs?customer_id=cust_xyz123"
```

Response 200 (same schema as above), 404 if not registered.

### POST /webhooks/receive — receive a webhook event

```bash
curl -X POST "${API_BASE_URL}/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d '{
    "idempotency_key": "req_abc123",
    "customer_id": "cust_xyz123",
    "payload": "{\"order_id\":\"ord_123\",\"amount\":99.99}"
  }'
```

Response 202 (new event):
```json
{ "event_id": "evt_1a2b3c4d", "status": "accepted", "created_at": 1707840000 }
```

Response 200 (duplicate):
```json
{ "event_id": "evt_1a2b3c4d", "status": "duplicate", "created_at": 1707840000 }
```

---

## 6. Sample AWS CLI Queries

### Fetch an event by event_id

```bash
aws dynamodb get-item \
  --region "$AWS_REGION" \
  --table-name "webhook_events_dev" \
  --key '{"pk":{"S":"EVENT#evt_1a2b3c4d"},"sk":{"S":"v0"}}' \
  --output json | jq '.Item'
```

### List all attempt records for an event

```bash
aws dynamodb query \
  --region "$AWS_REGION" \
  --table-name "webhook_events_dev" \
  --key-condition-expression "pk = :pk AND begins_with(sk, :prefix)" \
  --expression-attribute-values '{":pk":{"S":"EVENT#evt_1a2b3c4d"},":prefix":{"S":"ATTEMPT#"}}' \
  --output json | jq '.Items'
```

### Fetch a customer config

```bash
aws dynamodb get-item \
  --region "$AWS_REGION" \
  --table-name "webhook_configs_dev" \
  --key '{"pk":{"S":"CUSTOMER#cust_xyz123"},"sk":{"S":"CONFIG"}}' \
  --output json | jq '.Item'
```

### Check DLQ depth

```bash
aws sqs get-queue-attributes \
  --region "$AWS_REGION" \
  --queue-url "https://sqs.${AWS_REGION}.amazonaws.com/520819257503/webhook_delivery_dlq_dev" \
  --attribute-names ApproximateNumberOfMessagesVisible \
  --output json
```

### Manually send a test SQS message

```bash
EVENT_ID="evt_manual_test_$(date +%s)"
CUSTOMER_ID="cust_xyz123"

aws sqs send-message \
  --region "$AWS_REGION" \
  --queue-url "$QUEUE_URL" \
  --message-body "{\"event_id\":\"${EVENT_ID}\"}" \
  --message-attributes "{\"customer_id\":{\"DataType\":\"String\",\"StringValue\":\"${CUSTOMER_ID}\"}}" \
  --output json
```

### Update an event status (for testing terminal-state logic)

```bash
aws dynamodb update-item \
  --region "$AWS_REGION" \
  --table-name "webhook_events_dev" \
  --key '{"pk":{"S":"EVENT#evt_1a2b3c4d"},"sk":{"S":"v0"}}' \
  --update-expression "SET #s = :status, delivered_at = :ts" \
  --expression-attribute-names '{"#s":"status"}' \
  --expression-attribute-values '{":status":{"S":"delivered"},":ts":{"N":"1707840300"}}'
```

---

## 7. What Engineer 2 Needs to Build

The worker Lambda polls SQS and delivers webhooks to customer endpoints.
Here is what remains outstanding as of Day 5:

### Week 1 core (Days 5–6)

| Task | Description |
|---|---|
| SQS long-polling loop | `receive_message` with `wait_time_seconds=20`, `max_number_of_messages=10` |
| Deserialize SQS message | Parse `{"event_id":"..."}` body + `customer_id` attribute |
| Fetch event + config | `GetItem` from `webhook_events` (v0) and `webhook_configs` |
| Guard: terminal state | If event is `delivered` or `failed`, delete message and skip |
| Guard: missing/inactive config | Mark event `failed`, delete message |
| HMAC signature | `sha256=<hex(HMAC-SHA256("<timestamp>.<raw_payload>", secret))>` |
| HTTP POST delivery | Headers: `X-Webhook-Signature`, `X-Webhook-Id`, `X-Webhook-Timestamp`, `Content-Type: application/json` |
| Write attempt record | `PutItem` `ATTEMPT#n` before status update |
| Success path | Mark `status=delivered`, `delivered_at=<now>`, delete SQS message |
| Retryable failure path | Increment `attempt_count`, do NOT delete SQS message |
| Retry exhaustion | `attempt_count >= max_retries` → mark `status=failed`, delete SQS message |
| Terminal error path | 4xx non-retryable → mark `status=failed`, delete SQS message |

### Retryable vs terminal HTTP status codes (contract §6)

| Response | Classification | Action |
|---|---|---|
| 2xx | Success | Mark delivered, delete message |
| 400, 401, 403, 404, 422 | Terminal | Mark failed, delete message |
| 408, 429, 409 | Retryable | Keep message (visibility timeout) |
| 5xx | Retryable | Keep message (visibility timeout) |
| Network timeout / connection error | Retryable | Keep message |

### Delivery HTTP request format (contract §5)

```
POST {config.url} HTTP/1.1
Content-Type: application/json
X-Webhook-Signature: sha256=<hex_encoded_hmac>
X-Webhook-Id: evt_1a2b3c4d
X-Webhook-Timestamp: 1707840000

<raw event.payload string>
```

### HMAC signature algorithm (contract §5)

```
signing_string = "<X-Webhook-Timestamp value>" + "." + "<raw JSON payload>"
signature      = HMAC-SHA256(key=config.secret, data=signing_string)
header_value   = "sha256=" + hex_encode(signature)
```

Example:
```
timestamp   = 1707840000
payload     = {"order_id":"ord_123","amount":99.99}
signing_str = "1707840000.{"order_id":"ord_123","amount":99.99}"
header      = "sha256=a3b4c5d6..."
```

Note: the payload used for signing is the exact string stored in the DynamoDB
`payload` field (for example, the `event.payload` string), without any additional
escaping, re-serialization, or formatting changes.
---

## 8. Integration Test Acceptance Criteria

Per `CONTRACT_CONFIRMATION_LIST.md §10`, both teams agree the following test
cases must pass before Week 2:

| # | Test case | Pass criteria |
|---|---|---|
| 1 | **Happy path delivery** | Event `status=delivered`, `ATTEMPT#1` record exists, SQS message deleted |
| 2 | **Retry then success** | `ATTEMPT#1` with retryable error, `ATTEMPT#2` with 2xx, `status=delivered` |
| 3 | **Retry exhaustion** | `attempt_count >= max_retries`, `status=failed`, SQS message deleted |
| 4 | **Missing config** | `status=failed`, SQS message deleted, no HTTP delivery attempt |
| 5 | **Inactive config** | `status=failed`, SQS message deleted, no HTTP delivery attempt |
| 6 | **Duplicate SQS message** | Event already `delivered` → skip delivery, delete message, DynamoDB state unchanged |

### Duplicate SQS message detail (contract §10)

Precondition: `webhook_events` row has `status='delivered'`, `delivered_at` set.

When duplicate SQS message arrives with same `event_id`:
- ✅ Read the existing event from DynamoDB
- ✅ Detect terminal state (`status=delivered` or `status=failed`)
- ✅ Skip any HTTP delivery attempt
- ✅ Do not increment `attempt_count`
- ✅ Do not modify `status` or `delivered_at`
- ✅ Delete the duplicate SQS message
- ✅ No DLQ entry created

---

## 9. Change Control

**Any change to the following requires a PR tagged `contract-change` and explicit
approval from BOTH Engineer 1 and Engineer 2 before merging:**

- SQS message body structure or attribute names
- DynamoDB table names or key formats (`pk`/`sk` patterns)
- `status` enum values (`pending`, `delivered`, `failed`)
- HTTP delivery header names or HMAC signing algorithm
- Success / retry / terminal HTTP status code classification
- `attempt_number` starting value (currently `1`)

**Safe to change without contract review:**

- Internal implementation details of either Lambda
- Adding new DynamoDB attributes (non-breaking — existing readers ignore them)
- Adding new SQS message attributes (non-breaking)
- CloudWatch alarms, dashboards, metric names

---

## 10. Questions? Contact Engineer 1

For any questions about:
- Ingestion API behavior → check `ingestion/README.md`
- Infrastructure resources → check `template.yaml` Outputs section
- Contract items → check `docs/CONTRACT_CONFIRMATION_LIST.md`
- Schema details → check `docs/PROJECT_DICTIONARY.md`
