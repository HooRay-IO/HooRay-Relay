# Contract Confirmation List (Eng1 + Eng2)

Please confirm each item with explicit `✅ yes` / `❌ change needed`.

1. **SQS Message Contract**
- Queue message body is exactly `event_id` string (example: `evt_1a2b3c4d`).
- If JSON is used instead, define exact schema now (`{"event_id":"..."}`).
- Message attributes required/optional (e.g., `customer_id`) are finalized.
- SQS settings (fixed for MVP): visibility timeout = 60s; `max_receive_count = max_retries + 1` (initial attempt + `max_retries`); DLQ attached with CloudWatch alarm when DLQ message count > 0.

2. **`webhook_events` Metadata Schema**
- PK/SK format is fixed: `pk=EVENT#{event_id}`, `sk=v0`.
- Required attributes and types:
  - `event_id` (String)
  - `customer_id` (String)
  - `payload` (String, raw JSON payload)
  - `status` (String: `pending|delivered|failed`)
  - `attempt_count` (Number)
  - `created_at` (Number, unix seconds)
  - `delivered_at` (Optional Number, unix seconds; attribute omitted until event is delivered)
  - `next_retry_at` (Optional Number, unix seconds; attribute omitted when no retry is scheduled)
  - `gsi1pk`, `gsi1sk` when retryable
- `status` state machine (MVP, immutable):
  - Initial state for all new events is `pending`.
  - On successful delivery (`2xx` HTTP response): `pending → delivered`.
  - On retryable failure (per agreed error classes) *before* exhaustion: `pending → pending` with `attempt_count` incremented and `next_retry_at` updated as per retry schedule.
  - On retry exhaustion (`attempt_count >= max_retries`) or non‑retryable terminal condition (including missing/inactive config): `pending → failed`.
  - `delivered` and `failed` are terminal states for MVP: no further automatic or manual transitions (no retries) from these states.

3. **`webhook_events` Attempt Record Schema**
- Attempt key format: `pk=EVENT#{event_id}`, `sk=ATTEMPT#{attempt_number}`.
- Required attributes and types:
  - `attempt_number` (Number)
  - `attempted_at` (Number, unix seconds)
  - `http_status` (Number)
  - `response_time_ms` (Number)
  - `error_message` (String)
- Attempt numbering starts at `1` and increments per delivery try.

4. **`webhook_configs` Contract**
- Key format: `pk=CUSTOMER#{customer_id}`, `sk=CONFIG`.
- Required fields: `url`, `secret`, `max_retries`, `active`.
- `secret` format `whsec_...` and used directly for HMAC.
- Inactive/missing config behavior: worker marks event `failed` and deletes SQS message (confirm).

5. **Webhook Delivery HTTP Contract**
- Method: `POST` to config `url`.
- Headers:
  - `Content-Type: application/json`
  - `X-Webhook-Signature: sha256=<hex>`
  - `X-Webhook-Id: <event_id>`
  - `X-Webhook-Timestamp: <unix_seconds>`
- Signature string format confirmed as `"{timestamp}.{payload}"`.

6. **Success/Retry/Exhaustion Rules**
- `2xx` => success: record attempt, mark `delivered`, delete SQS message.
- HTTP error classification:
  - Terminal (no retry): `400`, `401`, `403`, `404`, `422`, and other non-retryable `4xx` codes indicating an invalid request; on terminal error mark event `failed` and delete SQS message.
  - Retryable: `408`, `429`, `409` (when due to transient conflict), all `5xx` responses, and network timeouts/connection errors; on retryable error keep SQS message and schedule next attempt.
- Retry exhaustion condition: `attempt_count >= max_retries` => mark `failed`, delete SQS message.

7. **Retry Schedule**
- Week 1 policy (final): **Visibility-timeout-only retries**.
  - Worker relies on SQS visibility timeout for all retries.
  - `next_retry_at` and `gsi1pk` / `gsi1sk` fields are **not** used by the Week 1 worker logic and are reserved for a future scheduled-retry design.
- Week 2+ may introduce a different retry algorithm (e.g., fixed schedule `1m, 5m, 30m` or exponential backoff using `next_retry_at` + GSI), but any change must go through the **Change Control Rule** in section 12.
8. **Idempotency and Duplicate Processing**
- Eng1 guarantees unique `event_id` creation for non-duplicates.
- Eng2 processing is safe under SQS at-least-once delivery (duplicate message handling defined).

9. **Observability Contract**
- Log format: JSON structured logging; each log line MUST be a single JSON object (see `PROJECT_DICTIONARY.md` examples).
- Required structured log fields (minimum JSON properties): `event_id`, `customer_id`, `attempt_number`, `result`, `http_status`, `latency_ms`, `error`.
- Required CloudWatch metrics (both teams MUST emit with these exact names):
  - `webhook.delivery.success` (count)
  - `webhook.delivery.failure` (count)
  - `webhook.delivery.latency_ms` (distribution of end-to-end delivery latency in milliseconds)
  - `webhook.queue.depth` (current SQS queue depth / number of messages visible)
- Required metric dimensions (applied consistently across all metrics where applicable):
  - `customer_id`
  - `status_code` (HTTP status code for the delivery attempt, e.g., `200`, `500`)
  - `queue_name` (SQS queue name)
  - `environment` (e.g., `dev`, `staging`, `prod`)

10. **Integration Test Acceptance (Day 5)**
- Test cases both teams agree to pass:
  - Happy path delivery
  - Retry then success
  - Retry exhausted
  - Missing config
  - Inactive config
  - Duplicate SQS message handling
- Pass criteria: correct DynamoDB state + correct SQS delete/non-delete behavior + expected logs.

11. **Ownership Boundaries**
- Eng1 owns ingestion payload shape, event write, and enqueue behavior.
- Eng2 owns worker poll/process/delivery/retry/attempt-recording behavior.
- Shared ownership: schema evolution and cross-team contract changes.

12. **Change Control Rule**
- Any schema/message/header/status contract change requires:
  - PR note tagged `contract-change`
  - Both Eng1 + Eng2 approval before merge.
