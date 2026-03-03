# HooRay Relay — Customer Onboarding Guide

Welcome to HooRay Relay. This guide walks you through registering your endpoint,
sending your first event, and verifying deliveries end-to-end.

---

## Table of Contents

1. [Prerequisites](#1-prerequisites)
2. [Quick Start (5 minutes)](#2-quick-start-5-minutes)
3. [Step 1 — Register your endpoint](#3-step-1--register-your-endpoint)
4. [Step 2 — Send a webhook event](#4-step-2--send-a-webhook-event)
5. [Idempotency](#5-idempotency)
6. [Signature Verification](#6-signature-verification)
7. [Delivery & Retry Behaviour](#7-delivery--retry-behaviour)
8. [Error Reference](#8-error-reference)
9. [Code Snippets](#9-code-snippets)

---

## 1. Prerequisites

| What you need | Where to get it |
|---|---|
| API key (`apk_…`) | Issued by the HooRay team at onboarding |
| A publicly reachable HTTPS endpoint | Your own infrastructure |
| A `customer_id` agreed with the HooRay team | Provided at onboarding |

**Base URL**

```
https://<api-id>.execute-api.us-west-2.amazonaws.com/dev
```

Replace `<api-id>` and `/dev` with your deployed values. The HooRay team will
provide these at onboarding.

---

## 2. Quick Start (5 minutes)

```bash
# 1. Register your endpoint (run once)
curl -s -X POST https://<base-url>/webhooks/configs \
  -H "Content-Type: application/json" \
  -H "X-API-Key: apk_YOUR_KEY" \
  -d '{
    "customer_id": "cust_xyz123",
    "url": "https://your.domain.com/webhooks"
  }' | jq .

# 2. Send an event
curl -s -X POST https://<base-url>/webhooks/receive \
  -H "Content-Type: application/json" \
  -H "X-API-Key: apk_YOUR_KEY" \
  -d '{
    "idempotency_key": "req_test_001",
    "customer_id": "cust_xyz123",
    "data": { "hello": "world" }
  }' | jq .

# 3. Check your endpoint received the delivery
# → HooRay POSTs the `data` object to your registered URL within seconds.
```

---

## 3. Step 1 — Register your endpoint

Before sending events you must register your delivery endpoint. This tells
HooRay where to POST events and provides the secret used to sign deliveries.

**`POST /webhooks/configs`**

```json
{
  "customer_id": "cust_xyz123",
  "url": "https://your.domain.com/webhooks",
  "secret": "whsec_optional_bring_your_own"
}
```

- `customer_id` — your identifier, agreed at onboarding.
- `url` — must be HTTPS and publicly reachable.
- `secret` — optional. If omitted, HooRay generates a secure `whsec_`-prefixed
  secret automatically. **Store the returned secret** — it is used to verify
  incoming deliveries and cannot be retrieved in plaintext later.

**Successful response (201)**

```json
{
  "customer_id": "cust_xyz123",
  "url": "https://your.domain.com/webhooks",
  "secret": "whsec_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
  "max_retries": 3,
  "active": true,
  "created_at": 1707840000,
  "updated_at": 1707840000
}
```

**Rotating your secret**

Re-POST to `/webhooks/configs` with the same `customer_id` and a new `secret`
value. The config is replaced immediately (last-write-wins). Deliveries
in-flight will use the old secret; new deliveries will use the new one.

---

## 4. Step 2 — Send a webhook event

**`POST /webhooks/receive`**

```json
{
  "idempotency_key": "req_01HX5Y3Z9ABCDEF",
  "customer_id": "cust_xyz123",
  "data": {
    "order_id": "ord_12345",
    "amount": 99.99,
    "customer_email": "user@example.com"
  }
}
```

| Field | Required | Description |
|---|---|---|
| `idempotency_key` | ✅ | Unique token for this event. Max 256 chars. |
| `customer_id` | ✅ | Must match a registered config. |
| `data` | ✅ | Arbitrary JSON. Max 400 KB serialised. |

**Accepted response (202)**

```json
{
  "event_id": "evt_1a2b3c4d",
  "status": "accepted",
  "created_at": 1707840000
}
```

Save the `event_id` — you can use it to correlate CloudWatch logs and delivery
attempt records.

**Duplicate response (200)**

```json
{
  "event_id": "evt_1a2b3c4d",
  "status": "duplicate",
  "created_at": 1707840000
}
```

The event was already received. No re-delivery occurs. See [Idempotency](#5-idempotency).

---

## 5. Idempotency

HooRay guarantees **exactly-once delivery** within a 24-hour idempotency window
via the `idempotency_key` field.

### Rules

- Use a value that is **naturally unique** for each logical event — e.g. your
  database primary key, a UUID you generate at send time, or a composite like
  `order.created:{order_id}:{timestamp}`.
- Retrying a failed HTTP request with the **same key** within 24 hours is safe —
  you will get `200` with `"status": "duplicate"` and the original `event_id`.
  The event will not be delivered again.
- After 24 hours the idempotency record expires (DynamoDB TTL). Sending the same
  key again after expiry creates a new event.

### What counts as a duplicate?

Only the `idempotency_key` is checked. The `data` payload is **not** compared —
if you resend with the same key but different data, the original payload is
returned and the new data is ignored.

### Generating good idempotency keys

```
req_{uuid_v4}               # e.g. req_01907c2f-4c3b-7e1a-b123-9d4e5f6a7b8c
order.created:{order_id}    # domain-scoped, human-readable
{source_system}:{event_id}  # federation-friendly
```

---

## 6. Signature Verification

HooRay signs every delivery with **HMAC-SHA256** using your registered `secret`.

### Headers sent with each delivery

| Header | Example value |
|---|---|
| `X-Webhook-Signature` | `sha256=a3f9b2c1...` |
| `X-Webhook-Id` | `evt_1a2b3c4d` |
| `X-Webhook-Timestamp` | `1707840000` |

### Verification algorithm

1. Read the raw request body as bytes (do **not** parse JSON first).
2. Compute `HMAC-SHA256(key=secret, message=body_bytes)`.
3. Hex-encode the result.
4. Compare `sha256={hex_digest}` to the `X-Webhook-Signature` header using a
   **constant-time** string comparison to prevent timing attacks.
5. Reject the request if they do not match.

### Code examples

**Python**

```python
import hashlib
import hmac

def verify_signature(body: bytes, secret: str, signature_header: str) -> bool:
    expected = "sha256=" + hmac.new(
        secret.encode(),
        body,
        hashlib.sha256,
    ).hexdigest()
    return hmac.compare_digest(expected, signature_header)

# In your Flask/FastAPI handler:
body = request.get_data()  # raw bytes, before JSON parsing
sig  = request.headers.get("X-Webhook-Signature", "")
if not verify_signature(body, "whsec_your_secret_here", sig):
    return Response("Invalid signature", status=401)
```

**Node.js**

```js
const crypto = require("crypto");

function verifySignature(bodyBuffer, secret, signatureHeader) {
  const expected =
    "sha256=" +
    crypto.createHmac("sha256", secret).update(bodyBuffer).digest("hex");

  // Normalize and validate the incoming header
  if (typeof signatureHeader !== "string") {
    return false;
  }
  const normalized = signatureHeader.trim();
  if (!normalized.startsWith("sha256=")) {
    return false;
  }

  const expectedBuf = Buffer.from(expected, "utf8");
  const receivedBuf = Buffer.from(normalized, "utf8");

  // timingSafeEqual throws if buffer lengths differ, so guard first
  if (expectedBuf.length !== receivedBuf.length) {
    return false;
  }

  // constant-time comparison
  return crypto.timingSafeEqual(expectedBuf, receivedBuf);
}

// Express middleware example:
app.post("/webhooks", express.raw({ type: "application/json" }), (req, res) => {
  const sig = req.headers["x-webhook-signature"] ?? "";
  if (!verifySignature(req.body, "whsec_your_secret_here", sig)) {
    return res.status(401).send("Invalid signature");
  }
  const payload = JSON.parse(req.body.toString());
  // ... handle payload
  res.sendStatus(200);
});
```

**curl (manual inspection)**

```bash
# Compute expected signature from a known payload
echo -n '{"order_id":"ord_12345","amount":99.99}' | \
  openssl dgst -sha256 -hmac "whsec_your_secret_here" | \
  awk '{print "sha256=" $2}'
```

---

## 7. Delivery & Retry Behaviour

| Setting | Default | Notes |
|---|---|---|
| Max retries | 3 | Configurable via `max_retries` at config creation |
| Retry strategy | Exponential back-off via SQS visibility timeout | Managed by HooRay |
| Success condition | Any `2xx` HTTP response from your endpoint | |
| Failure condition | Non-2xx response or network timeout | |
| Final failure action | Event marked `failed` in DynamoDB | No further retries |

### What your endpoint must do

- Respond with a `2xx` status code within **30 seconds**.
- Return `2xx` even if you have already processed the event (idempotency on your
  side is your responsibility for the delivery layer).
- Do **not** return `2xx` if you want HooRay to retry (e.g. temporarily
  unavailable) — return `503` instead.

### Delivery payload

HooRay POSTs the contents of the `data` field you submitted:

```http
POST https://your.domain.com/webhooks HTTP/1.1
Content-Type: application/json
X-Webhook-Signature: sha256=a3f9b2c1...
X-Webhook-Id: evt_1a2b3c4d
X-Webhook-Timestamp: 1707840000

{"order_id":"ord_12345","amount":99.99,"customer_email":"user@example.com"}
```

---

## 8. Error Reference

### Ingestion errors (`POST /webhooks/receive`)

| HTTP status | `error` code | Cause | Action |
|---|---|---|---|
| 200 | — | Duplicate `idempotency_key` | No action needed |
| 422 | `unprocessable_entity` | Validation failed (missing field, key too long, payload too large) | Fix request and retry |
| 500 | `internal_error` | Transient DynamoDB or SQS failure | Retry with the same `idempotency_key` — safe |

### Config errors

| HTTP status | `error` code | Cause | Action |
|---|---|---|---|
| 404 | `not_found` | No config for given `customer_id` | Register config first |
| 500 | `internal_error` | Transient DynamoDB failure | Retry |

### General guidance

- **`422`** errors are your problem — fix the request.
- **`500`** errors are transient — retry with exponential back-off. Because
  `idempotency_key` is checked first, retrying a `500` is always safe.
- If you receive a network error (no response), retry with the **same**
  `idempotency_key`.

---

## 9. Code Snippets

### curl

```bash
# Register endpoint
curl -s -X POST https://<base-url>/webhooks/configs \
  -H "Content-Type: application/json" \
  -H "X-API-Key: apk_YOUR_KEY" \
  -d '{
    "customer_id": "cust_xyz123",
    "url": "https://your.domain.com/webhooks"
  }' | jq .

# Send event
curl -s -X POST https://<base-url>/webhooks/receive \
  -H "Content-Type: application/json" \
  -H "X-API-Key: apk_YOUR_KEY" \
  -d '{
    "idempotency_key": "req_$(uuidgen | tr -d - | tr A-Z a-z)",
    "customer_id": "cust_xyz123",
    "data": { "order_id": "ord_001", "amount": 49.99 }
  }' | jq .

# Get config
curl -s "https://<base-url>/webhooks/configs?customer_id=cust_xyz123" \
  -H "X-API-Key: apk_YOUR_KEY" | jq .
```

### Python

```python
import uuid
import requests

BASE_URL = "https://<base-url>"
API_KEY  = "apk_YOUR_KEY"

HEADERS = {
    "Content-Type": "application/json",
    "X-API-Key": API_KEY,
}

# Register endpoint (run once)
resp = requests.post(
    f"{BASE_URL}/webhooks/configs",
    json={
        "customer_id": "cust_xyz123",
        "url": "https://your.domain.com/webhooks",
    },
    headers=HEADERS,
)
resp.raise_for_status()
config = resp.json()
print(f"Secret: {config['secret']}")  # store this!

# Send an event
resp = requests.post(
    f"{BASE_URL}/webhooks/receive",
    json={
        "idempotency_key": f"req_{uuid.uuid4().hex}",
        "customer_id": "cust_xyz123",
        "data": {
            "order_id": "ord_12345",
            "amount": 99.99,
            "customer_email": "user@example.com",
        },
    },
    headers=HEADERS,
)
resp.raise_for_status()
result = resp.json()
print(f"Event ID: {result['event_id']}  Status: {result['status']}")
```

### Node.js

```js
const https = require("https");
const crypto = require("crypto");

const BASE_URL = "https://<base-url>";
const API_KEY  = "apk_YOUR_KEY";

async function sendEvent(customerId, idempotencyKey, data) {
  const res = await fetch(`${BASE_URL}/webhooks/receive`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-API-Key": API_KEY,
    },
    body: JSON.stringify({
      idempotency_key: idempotencyKey,
      customer_id: customerId,
      data,
    }),
  });

  if (!res.ok) {
    const err = await res.json();
    const message = (err && (err.error || err.message)) || "Unknown error";
    throw new Error(`${res.status} ${message}`);
  }

  return res.json(); // { event_id, status, created_at }
}

// Usage
const result = await sendEvent(
  "cust_xyz123",
  `req_${crypto.randomUUID().replace(/-/g, "")}`,
  { order_id: "ord_001", amount: 49.99 }
);
console.log(result);
```

---

*For questions or support open an issue or contact the HooRay engineering team.*
