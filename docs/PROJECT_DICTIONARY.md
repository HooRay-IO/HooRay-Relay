# Webhook Relay System - Project Dictionary & Reference Guide

**Project Timeline:** 2 weeks (10 business days)  
**Team Size:** 2 backend engineers  
**Tech Stack:** Rust, AWS (DynamoDB, SQS, Lambda), SAM/Terraform  
**Goal:** Production-ready MVP webhook relay with idempotency, retries, and observability

---

## 📋 Table of Contents

1. [System Architecture](#system-architecture)
2. [DynamoDB Schema Reference](#dynamodb-schema-reference)
3. [API Specifications](#api-specifications)
4. [Core Components](#core-components)
5. [Data Flow](#data-flow)
6. [Code Patterns](#code-patterns)
7. [Testing Strategy](#testing-strategy)
8. [Deployment Guide](#deployment-guide)
9. [Glossary](#glossary)
10. [Decision Log](#decision-log)

---

## System Architecture

### High-Level Flow

```
┌────────────────────────────────────────────────────────┐
│                    INGESTION LAYER                      │
│  [API Gateway] → [Ingestion Lambda]                    │
│       ↓                                                 │
│  1. Idempotency Check (DynamoDB)                       │
│  2. Write Event (DynamoDB)                             │
│  3. Enqueue (SQS)                                      │
│  4. Return 202 Accepted                                │
└────────────────────────────────────────────────────────┘
                         ↓ SQS Queue
┌────────────────────────────────────────────────────────┐
│                   DELIVERY LAYER                        │
│  [SQS] → [Worker Lambda]                               │
│       ↓                                                 │
│  1. Fetch Event + Config (DynamoDB)                    │
│  2. Generate HMAC Signature                            │
│  3. HTTP POST to Customer Endpoint                     │
│  4. Record Attempt (DynamoDB)                          │
│  5. Update Status (DynamoDB)                           │
│  6. Retry via SQS if failed                            │
└────────────────────────────────────────────────────────┘
                         ↓
┌────────────────────────────────────────────────────────┐
│                 OBSERVABILITY LAYER                     │
│  - CloudWatch Logs (structured JSON)                   │
│  - CloudWatch Metrics (delivery rate, latency)         │
│  - CloudWatch Dashboards                               │
│  - X-Ray Tracing (optional for Week 2)                 │
└────────────────────────────────────────────────────────┘
```

### Component Responsibilities

| Component | Responsibility | Owner |
|-----------|---------------|-------|
| Ingestion Lambda | Receive webhooks, validate, store, enqueue | Engineer 1 |
| Worker Lambda | Poll queue, deliver webhooks, handle retries | Engineer 2 |
| DynamoDB Events | Store webhook events and delivery attempts | Shared |
| DynamoDB Idempotency | Prevent duplicate processing | Engineer 1 |
| DynamoDB Configs | Store customer webhook configurations | Engineer 1 |
| SQS Queue | Async delivery queue with retry | Engineer 2 |
| CloudWatch | Monitoring and alerting | Both (Week 2) |

---

## DynamoDB Schema Reference

### Table 1: `webhook_events`

**Purpose:** Store all webhook events and their delivery attempts

#### Primary Key
- **PK (Partition Key):** `EVENT#{event_id}` (e.g., `EVENT#evt_1a2b3c4d`)
- **SK (Sort Key):** `v0` for metadata, `ATTEMPT#{num}` for attempts

#### Metadata Record (SK = "v0")

```json
{
  "pk": "EVENT#evt_1a2b3c4d",
  "sk": "v0",
  "event_id": "evt_1a2b3c4d",
  "customer_id": "cust_xyz123",
  "payload": "{\"order_id\":\"ord_123\",\"amount\":99.99}",
  "status": "pending|delivered|failed",
  "attempt_count": 0,
  "created_at": 1707840000,
  "delivered_at": 1707840300,  // null if not delivered
  "next_retry_at": 1707840600,  // null if delivered or exhausted
  "gsi1pk": "RETRY",  // only set when retry needed
  "gsi1sk": "NEXT#1707840600#EVENT#evt_1a2b3c4d"
}
```

#### Attempt Record (SK = "ATTEMPT#1", "ATTEMPT#2", etc.)

```json
{
  "pk": "EVENT#evt_1a2b3c4d",
  "sk": "ATTEMPT#1",
  "attempt_number": 1,
  "attempted_at": 1707840000,
  "http_status": 503,
  "response_time_ms": 5000,
  "error_message": "Service Unavailable"
}
```

#### Global Secondary Index: GSI1 (Retry Queue)

- **GSI1PK:** `RETRY` (constant for all items needing retry)
- **GSI1SK:** `NEXT#{timestamp}#EVENT#{event_id}`
- **Purpose:** Query events ready for retry by timestamp
- **Query Pattern:** `GSI1PK = "RETRY" AND GSI1SK <= "NEXT#{current_time}"`

#### Status Values

| Status | Meaning | Next Action |
|--------|---------|-------------|
| `pending` | Queued for first delivery | Worker processes |
| `delivered` | Successfully delivered (2xx response) | Terminal state |
| `failed` | Exhausted all retries | Terminal state |

---

### Table 2: `webhook_idempotency`

**Purpose:** Prevent duplicate webhook processing

#### Primary Key
- **PK:** `IDEM#{idempotency_key}` (e.g., `IDEM#req_abc123`)

#### Record Structure

```json
{
  "pk": "IDEM#req_abc123",
  "event_id": "evt_1a2b3c4d",
  "created_at": 1707840000,
  "ttl": 1707926400  // 24 hours from creation
}
```

#### TTL Behavior
- Automatically deleted 24 hours after creation
- Uses DynamoDB's native TTL feature
- No manual cleanup required

---

### Table 3: `webhook_configs`

**Purpose:** Store customer webhook endpoint configurations

#### Primary Key
- **PK:** `CUSTOMER#{customer_id}` (e.g., `CUSTOMER#cust_xyz123`)
- **SK:** `CONFIG` (MVP: single config per customer)

#### Record Structure

```json
{
  "pk": "CUSTOMER#cust_xyz123",
  "sk": "CONFIG",
  "customer_id": "cust_xyz123",
  "url": "https://customer.example.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8",
  "max_retries": 3,
  "created_at": 1707840000,
  "updated_at": 1707840000,
  "active": true
}
```

#### Secret Format
- Prefix: `whsec_`
- Length: 32 characters (random alphanumeric)
- Purpose: HMAC-SHA256 signature generation
- Generation: Use `openssl rand -hex 16` or Rust's `rand` crate

---

## API Specifications

### Endpoint 1: Receive Webhook

**POST** `/webhooks/receive`

#### Request Headers
```
Content-Type: application/json
X-API-Key: <customer_api_key>
```

#### Request Body
```json
{
  "idempotency_key": "req_unique_identifier",
  "customer_id": "cust_xyz123",
  "event_type": "order.created",  // Optional for MVP
  "data": {
    "order_id": "ord_12345",
    "amount": 99.99,
    "customer_email": "user@example.com"
  }
}
```

#### Response 202 Accepted
```json
{
  "event_id": "evt_1a2b3c4d",
  "status": "accepted",
  "created_at": 1707840000
}
```

#### Response 200 OK (Duplicate)
```json
{
  "event_id": "evt_1a2b3c4d",
  "status": "duplicate",
  "created_at": 1707840000
}
```

#### Response 400 Bad Request
```json
{
  "error": "invalid_request",
  "message": "Missing required field: idempotency_key"
}
```

#### Response 429 Too Many Requests (Week 2)
```json
{
  "error": "rate_limit_exceeded",
  "retry_after": 60
}
```

---

### Endpoint 2: Create Webhook Config

**POST** `/webhooks/configs`

#### Request Body
```json
{
  "customer_id": "cust_xyz123",
  "url": "https://customer.example.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8"  // Optional, auto-generated if not provided
}
```

#### Response 201 Created
```json
{
  "customer_id": "cust_xyz123",
  "url": "https://customer.example.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8",
  "created_at": 1707840000
}
```

---

### Endpoint 3: Get Webhook Config

**GET** `/webhooks/configs?customer_id=cust_xyz123`

#### Response 200 OK
```json
{
  "customer_id": "cust_xyz123",
  "url": "https://customer.example.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8",
  "max_retries": 3,
  "active": true,
  "created_at": 1707840000,
  "updated_at": 1707840000
}
```

---

### Webhook Delivery to Customer Endpoint

**POST** `{customer_webhook_url}`

#### Request Headers
```
Content-Type: application/json
X-Webhook-Signature: sha256=<hmac_signature>
X-Webhook-Id: evt_1a2b3c4d
X-Webhook-Timestamp: 1707840000
```

#### Request Body
```json
{
  "order_id": "ord_12345",
  "amount": 99.99,
  "customer_email": "user@example.com"
}
```

#### Expected Customer Response
- **2xx status code:** Delivery successful
- **4xx/5xx status code:** Delivery failed, will retry

---

## Core Components

### Component 1: Ingestion Service (Engineer 1)

**File:** `src/ingestion/main.rs`

**Dependencies:**
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
axum = "0.7"
aws-sdk-dynamodb = "1.0"
aws-sdk-sqs = "1.0"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
nanoid = "0.4"
tracing = "0.1"
tracing-subscriber = "0.3"
```

**Key Functions:**

1. **`receive_webhook()`**
   - Validates request body
   - Checks idempotency
   - Writes event to DynamoDB
   - Enqueues to SQS
   - Returns 202 response

2. **`check_idempotency()`**
   - Conditional PUT to idempotency table
   - Returns existing event_id if duplicate
   - Generates new event_id if unique

3. **`write_event()`**
   - Writes event metadata to Events table
   - Sets initial status to "pending"
   - Records creation timestamp

4. **`enqueue()`**
   - Sends event_id to SQS
   - Includes customer_id as message attribute

---

### Component 2: Delivery Worker (Engineer 2)

**File:** `src/worker/main.rs`

**Dependencies:**
```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
aws-sdk-dynamodb = "1.0"
aws-sdk-sqs = "1.0"
reqwest = { version = "0.11", features = ["json"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
hmac = "0.12"
sha2 = "0.10"
hex = "0.4"
tracing = "0.1"
tracing-subscriber = "0.3"
```

**Key Functions:**

1. **`run()`**
   - Infinite loop polling SQS
   - Long polling with 20s wait time
   - Processes up to 10 messages per batch

2. **`poll_and_process()`**
   - Receives messages from SQS
   - Calls `deliver_event()` for each
   - Deletes message on success
   - Lets visibility timeout expire on retry

3. **`deliver_event()`**
   - Fetches event and config from DynamoDB
   - Generates HMAC signature
   - Makes HTTP POST to customer endpoint
   - Records delivery attempt
   - Updates event status

4. **`generate_signature()`**
   - HMAC-SHA256 of `<timestamp>.<raw_json_body>` with customer secret
   - Format: `sha256={hex_encoded_signature}`

5. **`record_attempt()`**
   - Writes attempt record to DynamoDB
   - Stores HTTP status, latency, error message

6. **`mark_delivered()`**
   - Updates event status to "delivered"
   - Sets delivered_at timestamp
   - Removes from retry queue

---

## Data Flow

### Flow 1: Successful Webhook Delivery

```
1. Customer POSTs to /webhooks/receive
   → Ingestion Lambda receives request

2. Ingestion checks idempotency
   → DynamoDB conditional write to idempotency table
   → New event_id generated (e.g., evt_1a2b3c4d)

3. Ingestion writes event
   → DynamoDB PUT to events table (status: pending)

4. Ingestion enqueues
   → SQS SendMessage with event_id

5. Ingestion responds
   → 202 Accepted with event_id

6. Worker polls SQS
   → Receives event_id message

7. Worker fetches event + config
   → DynamoDB GET from events table
   → DynamoDB GET from configs table

8. Worker delivers webhook
   → HTTP POST to customer endpoint
   → Customer returns 200 OK

9. Worker records attempt
   → DynamoDB PUT attempt record (ATTEMPT#1)

10. Worker marks delivered
    → DynamoDB UPDATE event (status: delivered)

11. Worker deletes SQS message
    → SQS DeleteMessage
```

### Flow 2: Webhook Retry on Failure

```
1-7. Same as successful flow through worker fetch

8. Worker delivers webhook
   → HTTP POST to customer endpoint
   → Customer returns 503 Service Unavailable

9. Worker records attempt
   → DynamoDB PUT attempt record (ATTEMPT#1)

10. Worker increments attempt count
    → DynamoDB UPDATE event (attempt_count: 1)

11. Worker does NOT delete SQS message
    → Message becomes visible again after 5 minutes

12. SQS redelivers message
    → Worker receives same event_id

13. Worker retries delivery (ATTEMPT#2)
    → Customer returns 200 OK

14. Worker marks delivered
    → DynamoDB UPDATE event (status: delivered)

15. Worker deletes SQS message
    → SQS DeleteMessage
```

### Flow 3: Webhook Exhaustion

```
1-7. Same as retry flow

8. Third delivery attempt fails
   → Customer returns 500 Internal Server Error

9. Worker records attempt
   → DynamoDB PUT attempt record (ATTEMPT#3)

10. Worker checks attempt_count >= max_retries
    → Condition is true (3 >= 3)

11. Worker marks failed
    → DynamoDB UPDATE event (status: failed)

12. Worker deletes SQS message
    → Webhook will not retry again
    → Move to DLQ (optional for Week 2)
```

---

## Code Patterns

### Pattern 1: Idempotency Check

```rust
async fn check_idempotency(
    dynamo: &DynamoClient,
    key: &str,
) -> Result<String, IdempotencyError> {
    let event_id = format!("evt_{}", nanoid::nanoid!(16));
    
    let result = dynamo
        .put_item()
        .table_name("webhook_idempotency")
        .item("pk", AttributeValue::S(format!("IDEM#{}", key)))
        .item("event_id", AttributeValue::S(event_id.clone()))
        .item("created_at", AttributeValue::N(now().to_string()))
        .item("ttl", AttributeValue::N((now() + 86400).to_string()))
        .condition_expression("attribute_not_exists(pk)")
        .send()
        .await;
    
    match result {
        Ok(_) => Ok(event_id),
        Err(e) if is_conditional_check_failed(&e) => {
            // Fetch existing event_id
            let existing = get_idempotency_record(dynamo, key).await?;
            Err(IdempotencyError::AlreadyProcessed(existing.event_id))
        }
        Err(e) => Err(e.into()),
    }
}

fn is_conditional_check_failed(e: &SdkError<PutItemError>) -> bool {
    matches!(
        e,
        SdkError::ServiceError(err) 
        if err.err().is_conditional_check_failed_exception()
    )
}
```

### Pattern 2: HMAC Signature Generation

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn generate_signature(secret: &str, timestamp: i64, raw_body: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;

    let signing = format!("{}.{}", timestamp, raw_body);
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(signing.as_bytes());

    let result = mac.finalize();
    let code_bytes = result.into_bytes();

    format!("sha256={}", hex::encode(code_bytes))
}

// Usage in worker
let timestamp = chrono::Utc::now().timestamp();
let signature = generate_signature(&config.secret, timestamp, &event.payload);
let response = http_client
    .post(&config.url)
    .header("Content-Type", "application/json")
    .header("X-Webhook-Signature", signature)
    .header("X-Webhook-Id", &event.event_id)
    .header("X-Webhook-Timestamp", timestamp.to_string())
    .body(event.payload.clone())
    .send()
    .await?;
```

### Pattern 3: Structured Logging

```rust
use tracing::{info, error, instrument};

#[instrument(skip(dynamo), fields(event_id = %event_id))]
async fn deliver_event(
    dynamo: &DynamoClient,
    event_id: &str,
) -> Result<DeliveryResult, Error> {
    info!("Starting delivery");
    
    let event = get_event(dynamo, event_id).await?;
    let config = get_config(dynamo, &event.customer_id).await?;
    
    let start = Instant::now();
    let response = deliver_to_customer(&config, &event).await;
    let elapsed_ms = start.elapsed().as_millis() as u64;
    
    match response {
        Ok(status) if (200..300).contains(&status) => {
            info!(
                status = status,
                latency_ms = elapsed_ms,
                "Delivery successful"
            );
            Ok(DeliveryResult::Success)
        }
        Ok(status) => {
            error!(
                status = status,
                latency_ms = elapsed_ms,
                "Delivery failed"
            );
            Ok(DeliveryResult::Retry)
        }
        Err(e) => {
            error!(error = %e, "HTTP request failed");
            Ok(DeliveryResult::Retry)
        }
    }
}
```

### Pattern 4: Error Handling

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WebhookError {
    #[error("DynamoDB error: {0}")]
    DynamoDB(#[from] aws_sdk_dynamodb::Error),
    
    #[error("SQS error: {0}")]
    Sqs(#[from] aws_sdk_sqs::Error),
    
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    
    #[error("Event not found: {0}")]
    NotFound(String),
    
    #[error("Already processed: {0}")]
    AlreadyProcessed(String),
}

// Usage
async fn handle_webhook(payload: Webhook) -> Result<Response, WebhookError> {
    let event_id = check_idempotency(&payload.idempotency_key).await?;
    write_event(&event_id, &payload).await?;
    enqueue(&event_id).await?;
    Ok(Response::Accepted(event_id))
}
```

---

## Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_generation() {
        let secret = "whsec_test123";
        let timestamp = 1707840000_i64;
        let payload = r#"{"order_id":"ord_123"}"#;
        let sig = generate_signature(secret, timestamp, payload);

        assert!(sig.starts_with("sha256="));
        assert_eq!(sig.len(), 71); // "sha256=" + 64 hex chars
    }

    #[tokio::test]
    async fn test_idempotency_new_key() {
        let dynamo = mock_dynamo_client();
        let result = check_idempotency(&dynamo, "req_new").await;
        
        assert!(result.is_ok());
        let event_id = result.unwrap();
        assert!(event_id.starts_with("evt_"));
    }

    #[tokio::test]
    async fn test_idempotency_duplicate_key() {
        let dynamo = mock_dynamo_client();
        
        // First call succeeds
        let event_id_1 = check_idempotency(&dynamo, "req_dup").await.unwrap();
        
        // Second call returns same event_id
        let result = check_idempotency(&dynamo, "req_dup").await;
        assert!(matches!(result, Err(IdempotencyError::AlreadyProcessed(_))));
    }
}
```

### Integration Tests

```bash
# Test script: tests/integration_test.sh

#!/bin/bash
set -e

API_URL="https://your-api-gateway-url.amazonaws.com"
CUSTOMER_ID="test_customer_$(date +%s)"

echo "1. Creating webhook config..."
curl -X POST "$API_URL/webhooks/configs" \
  -H "Content-Type: application/json" \
  -d "{\"customer_id\":\"$CUSTOMER_ID\",\"url\":\"https://webhook.site/unique-id\"}"

echo "2. Sending webhook..."
IDEMPOTENCY_KEY="req_$(uuidgen)"
RESPONSE=$(curl -X POST "$API_URL/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"$IDEMPOTENCY_KEY\",\"customer_id\":\"$CUSTOMER_ID\",\"data\":{\"test\":true}}")

EVENT_ID=$(echo $RESPONSE | jq -r '.event_id')
echo "Event ID: $EVENT_ID"

echo "3. Testing idempotency..."
RESPONSE_2=$(curl -X POST "$API_URL/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"$IDEMPOTENCY_KEY\",\"customer_id\":\"$CUSTOMER_ID\",\"data\":{\"test\":true}}")

EVENT_ID_2=$(echo $RESPONSE_2 | jq -r '.event_id')

if [ "$EVENT_ID" == "$EVENT_ID_2" ]; then
  echo "✓ Idempotency test passed"
else
  echo "✗ Idempotency test failed"
  exit 1
fi

echo "4. Waiting for delivery (30s)..."
sleep 30

echo "5. Check webhook.site for delivery"
echo "✓ Integration test complete"
```

### Load Tests (k6)

```javascript
// tests/load_test.js
import http from 'k6/http';
import { check, sleep } from 'k6';
import { uuidv4 } from 'https://jslib.k6.io/k6-utils/1.4.0/index.js';

export let options = {
  stages: [
    { duration: '1m', target: 10 },   // Ramp up to 10 users
    { duration: '3m', target: 50 },   // Ramp up to 50 users
    { duration: '5m', target: 100 },  // Stay at 100 users
    { duration: '1m', target: 0 },    // Ramp down
  ],
  thresholds: {
    http_req_duration: ['p(95)<500'],  // 95% of requests < 500ms
    http_req_failed: ['rate<0.01'],    // Error rate < 1%
  },
};

const API_URL = __ENV.API_URL || 'https://your-api-gateway-url.amazonaws.com';
const CUSTOMER_ID = 'load_test_customer';

export default function() {
  const payload = JSON.stringify({
    idempotency_key: `req_${uuidv4()}`,
    customer_id: CUSTOMER_ID,
    data: {
      order_id: `ord_${uuidv4()}`,
      amount: Math.random() * 1000,
      timestamp: Date.now(),
    },
  });

  const params = {
    headers: { 'Content-Type': 'application/json' },
  };

  const response = http.post(`${API_URL}/webhooks/receive`, payload, params);

  check(response, {
    'status is 202': (r) => r.status === 202,
    'has event_id': (r) => JSON.parse(r.body).event_id !== undefined,
  });

  sleep(1);
}
```

**Run load test:**
```bash
k6 run --env API_URL=https://your-api-url.com tests/load_test.js
```

---

## Deployment Guide

### Prerequisites

```bash
# Install AWS SAM CLI
brew install aws-sam-cli  # macOS
# or
pip install aws-sam-cli  # Python

# Install Rust toolchain
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add x86_64-unknown-linux-musl  # For Lambda

# Install Cargo Lambda (for building Lambda functions)
pip install cargo-lambda
```

### SAM Template (template.yaml)

See full template in Infrastructure section above.

### Build and Deploy

```bash
# Build Rust Lambda functions
cd ingestion
cargo lambda build --release --arm64
cd ../worker
cargo lambda build --release --arm64
cd ..

# Deploy with SAM
sam build
sam deploy --guided

# Follow prompts:
# - Stack Name: webhook-relay-mvp
# - AWS Region: us-east-1
# - Confirm changes: Y
# - Allow SAM CLI IAM role creation: Y
# - Save arguments to config: Y
```

### Environment Variables

Create `.env` file for local testing:

```bash
# .env
AWS_REGION=us-east-1
EVENTS_TABLE=webhook_events
IDEMPOTENCY_TABLE=webhook_idempotency
CONFIGS_TABLE=webhook_configs
DELIVERY_QUEUE_URL=https://sqs.us-east-1.amazonaws.com/123456789/webhook-delivery
LOG_LEVEL=info
```

### Monitoring Setup

```bash
# Create CloudWatch dashboard
aws cloudwatch put-dashboard \
  --dashboard-name webhook-relay-mvp \
  --dashboard-body file://monitoring/dashboard.json

# Create alarms
aws cloudwatch put-metric-alarm \
  --alarm-name webhook-delivery-errors \
  --metric-name DeliveryErrors \
  --namespace WebhookRelay \
  --statistic Sum \
  --period 300 \
  --threshold 10 \
  --comparison-operator GreaterThanThreshold \
  --evaluation-periods 1
```

---

## Glossary

| Term | Definition |
|------|------------|
| **Event** | A webhook payload to be delivered to a customer endpoint |
| **Event ID** | Unique identifier for an event (format: `evt_{nanoid}`) |
| **Idempotency Key** | Client-provided unique key to prevent duplicate processing |
| **Delivery Attempt** | A single HTTP POST to a customer endpoint |
| **Worker** | Lambda function that processes delivery queue |
| **Ingestion** | Process of receiving and storing incoming webhooks |
| **HMAC** | Hash-based Message Authentication Code for webhook signing |
| **TTL** | Time To Live - auto-deletion timestamp for DynamoDB items |
| **GSI** | Global Secondary Index in DynamoDB |
| **SQS** | Simple Queue Service - AWS message queue |
| **DLQ** | Dead Letter Queue - for messages that fail repeatedly |
| **Visibility Timeout** | Period during which a message is invisible after being received from SQS |

---

## Decision Log

### Day 0 Decisions

| Decision | Rationale |
|----------|-----------|
| Use Lambda instead of ECS | Faster to deploy for MVP, lower operational overhead |
| Use Standard SQS instead of FIFO | Higher throughput, customer endpoints should be idempotent anyway |
| Use SAM instead of Terraform | Faster AWS-native deployment, less boilerplate |
| Single webhook endpoint per customer | Simplifies MVP, can add multiple endpoints in v2 |
| Fixed retry schedule (1min, 5min, 30min) | Good enough for MVP, customizable schedules in v2 |
| 3 max retry attempts | Industry standard (Stripe uses 3-5) |
| 24-hour idempotency window | Balances safety vs. storage cost |
| 30-day event retention | Compliance-friendly, auto-cleanup via TTL |

### Week 1 Scope Cuts

| Feature | Why Cut | When to Add |
|---------|---------|-------------|
| Rate limiting | Not critical for MVP | Week 3 |
| Circuit breakers | Can handle in retry logic | Week 3 |
| Event filtering by type | Deliver everything for now | Week 4 |
| Customer dashboard UI | API-first approach | Week 5 |
| Webhook replay | Complex feature | Week 6 |
| Multi-region | Single region sufficient | When scale requires |

---

## Quick Reference Commands

### DynamoDB Queries

```bash
# Get event by ID
aws dynamodb get-item \
  --table-name webhook_events \
  --key '{"pk":{"S":"EVENT#evt_123"},"sk":{"S":"v0"}}'

# Query retry queue
aws dynamodb query \
  --table-name webhook_events \
  --index-name GSI1 \
  --key-condition-expression "gsi1pk = :pk AND gsi1sk <= :sk" \
  --expression-attribute-values '{":pk":{"S":"RETRY"},":sk":{"S":"NEXT#1707840000"}}'

# Check idempotency
aws dynamodb get-item \
  --table-name webhook_idempotency \
  --key '{"pk":{"S":"IDEM#req_abc123"}}'
```

### SQS Operations

```bash
# Send test message
aws sqs send-message \
  --queue-url https://sqs.us-east-1.amazonaws.com/123/webhook-delivery \
  --message-body "evt_test123"

# Check queue depth
aws sqs get-queue-attributes \
  --queue-url https://sqs.us-east-1.amazonaws.com/123/webhook-delivery \
  --attribute-names ApproximateNumberOfMessages

# Purge queue (testing only!)
aws sqs purge-queue \
  --queue-url https://sqs.us-east-1.amazonaws.com/123/webhook-delivery
```

### Logs

```bash
# Tail ingestion logs
sam logs --stack-name webhook-relay-mvp --name IngestionFunction --tail

# Tail worker logs
sam logs --stack-name webhook-relay-mvp --name WorkerFunction --tail

# Query logs
aws logs filter-log-events \
  --log-group-name /aws/lambda/webhook-relay-ingestion \
  --filter-pattern "ERROR"
```

---

## Success Metrics (End of Week 2)

- [ ] 500 webhooks/second throughput
- [ ] < 100ms p95 ingestion latency
- [ ] < 5s p95 delivery latency
- [ ] > 99.9% delivery success rate (non-customer errors)
- [ ] 0 duplicate deliveries
- [ ] < 0.1% error rate
- [ ] CloudWatch dashboard with key metrics
- [ ] API documentation published
- [ ] Integration tests passing
- [ ] Load tests passing

---

**Last Updated:** Sprint Start  
**Document Owner:** Both Engineers  
**Review Cadence:** Daily standup
