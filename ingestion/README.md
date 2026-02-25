# Ingestion Service — Engineer 1 (Days 1–4)

> **Sprint context:** Engineer 1 owns the webhook ingestion pipeline; Engineer 2
> owns the delivery worker (`../worker/`). This document is the living engineering
> log for all four days of ingestion work — what was built, the thinking behind
> every design decision, the bugs encountered, and how they were solved.

---

## Table of Contents

1. [What Was Built — All Four Days](#1-what-was-built--all-four-days)
2. [Project Structure](#2-project-structure)
3. [Dependency Choices](#3-dependency-choices)
4. [Data Models](#4-data-models)
5. [DynamoDB Key Contracts](#5-dynamodb-key-contracts)
6. [Cross-Team Wire Format](#6-cross-team-wire-format)
7. [Day 3 — SQS Integration & Webhook Handler](#7-day-3--sqs-integration--webhook-handler)
8. [Day 4 — Config Management & Lambda Entry Point](#8-day-4--config-management--lambda-entry-point)
9. [Bugs Encountered & Solutions](#9-bugs-encountered--solutions)
10. [Running the Tests](#10-running-the-tests)
11. [Roadmap — What's Next](#11-roadmap--whats-next)

---

## 1. What Was Built — All Four Days

| Day | Deliverable | File | Tests |
|-----|-------------|------|-------|
| 1 | Data models, validation, error types | `src/model.rs` | 14 |
| 2 | DynamoDB config + client factory | `src/services/dynamodb.rs` | 6 |
| 2 | Idempotency check-and-record (atomic) | `src/services/idempotency.rs` | 5 |
| 2 | Event persistence (30-day TTL) | `src/services/events.rs` | 7 |
| 3 | SQS enqueue with `customer_id` attribute | `src/services/queue.rs` | 7 |
| 3 | `POST /webhooks/receive` Axum handler | `src/handlers/webhook.rs` | 13 |
| 4 | `POST /webhooks/configs` + `GET /webhooks/configs` handlers | `src/handlers/config.rs` | 11 |
| 4 | DynamoDB upsert + fetch for configs | `src/services/configs.rs` | 4 |
| 4 | Lambda entry point (router + cold-start) | `src/main.rs` | — |

**Total: 69 ingestion tests — 0 failures, 0 warnings.**

---

## 2. Project Structure

```
ingestion/
├── Cargo.toml
└── src/
    ├── main.rs                   ← Lambda entry point (Day 4)
    ├── model.rs                  ← all shared types, validation, error enum
    ├── services/
    │   ├── mod.rs
    │   ├── dynamodb.rs           ← AppConfig::from_env() + DynamoDB client factory
    │   ├── idempotency.rs        ← atomic conditional PutItem dedup (Day 2)
    │   ├── events.rs             ← persist WebhookEvent to DynamoDB (Day 2)
    │   ├── queue.rs              ← enqueue onto SQS with customer_id attribute (Day 3)
    │   └── configs.rs            ← DynamoDB CRUD for webhook_configs table (Day 4)
    └── handlers/
        ├── mod.rs
        ├── webhook.rs            ← POST /webhooks/receive (Day 3)
        └── config.rs             ← POST/GET /webhooks/configs (Day 4)
```

---

## 3. Dependency Choices

```toml
serde            = { version = "1", features = ["derive"] }   # serde round-trips
serde_json       = "1"                                         # JSON value + serialization
thiserror        = "2.0"                                       # typed error enums
tracing          = "0.1"                                       # structured logging
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
tokio            = { version = "1", features = ["full"] }     # async runtime
axum             = "0.8"                                       # HTTP framework
aws-config       = "1.8.14"                                   # ambient credential chain
aws-sdk-dynamodb = "1"                                        # DynamoDB client
aws-sdk-sqs      = "1.95.0"                                   # SQS client (pinned)
serde_dynamo     = { version = "4", features = ["aws-sdk-dynamodb+1"] }
nanoid           = "0.4"                                       # evt_ ID generation
lambda_http      = "0.13"                                      # Lambda HTTP adapter (Day 4)
```

**Why these specific choices:**

| Decision | Rationale |
|---|---|
| `thiserror` not `anyhow` | Every failure mode is a named variant. The handler maps each variant to an HTTP status code — `anyhow` would require string matching. |
| `axum` not `actix-web` | Tower-native, composes cleanly with `lambda_http`'s service adapter. `Arc<AppState>` injected via `State` extractor with zero boilerplate. |
| `nanoid` not `uuid` | `evt_` prefix + 16-char nanoid is shorter in URLs/logs than a UUID, but still has >2^80 collision resistance. |
| `serde_dynamo` | Converts between `serde_json`-compatible structs and DynamoDB `AttributeValue` maps — eliminates a wall of `.item("field", AttributeValue::S(…))` boilerplate. |
| `aws-sdk-sqs = "1.95.0"` pinned | Worker crate pins the same version so `Cargo.lock` compiles the SDK once, not twice. |
| `lambda_http = "0.13"` | Bridges API Gateway events into a standard Axum `Service`, with zero changes to handler code. |

---

## 4. Data Models

All types live in `src/model.rs`.

### API boundary types

| Type | HTTP surface | Purpose |
|---|---|---|
| `WebhookReceiveRequest` | `POST /webhooks/receive` body | Inbound event from caller |
| `WebhookReceiveResponse` | 202 / 200 response | Returns `event_id` + status |
| `ReceiveStatus` | enum field | `accepted` \| `duplicate` |
| `CreateConfigRequest` | `POST /webhooks/configs` body | Register a customer endpoint |
| `WebhookConfigResponse` | 201 / 200 response | Config record DTO |

### DynamoDB entity types

| Type | Table | SK pattern |
|---|---|---|
| `WebhookEvent` | `webhook_events` | `v0` (metadata) · `ATTEMPT#n` (delivery records) |
| `WebhookConfig` | `webhook_configs` | `CONFIG` |
| `IdempotencyRecord` | `webhook_idempotency` | _(PK-only table)_ |

### Error enum

```rust
pub enum IngestionError {
    MissingField(String),          // env var absent at cold-start → 500
    ConfigNotFound(String),        // customer has no registered config → 404
    AlreadyExists(String),         // duplicate event write attempted → 409
    Serialization(String),         // JSON/serde failure → 500
    DynamoDb(String),              // AWS SDK DynamoDB error → 500
    Sqs(String),                   // AWS SDK SQS error → 500
    ItemNotFound { entity, key },  // DynamoDB get returned nothing → 404
    DecodeDynamo(String),          // serde_dynamo decode failure → 500
}
```

HTTP status mapping lives in **one place per handler** — `ingestion_error_response()` in `webhook.rs` and `config_error_response()` in `config.rs`. Nothing in the service layer makes HTTP decisions.

---

## 5. DynamoDB Key Contracts

Enforced by `pk()` / `sk()` helpers on each struct and locked by unit tests.
Changing these is a **breaking change** for both engineers.

| Table | PK | SK | Example |
|---|---|---|---|
| `webhook_events` | `EVENT#{event_id}` | `v0` | `EVENT#evt_1a2b3c / v0` |
| `webhook_events` | `EVENT#{event_id}` | `ATTEMPT#{n}` | `EVENT#evt_1a2b3c / ATTEMPT#1` |
| `webhook_configs` | `CUSTOMER#{customer_id}` | `CONFIG` | `CUSTOMER#cust_xyz / CONFIG` |
| `webhook_idempotency` | `IDEM#{idempotency_key}` | _(PK-only)_ | `IDEM#req_abc123` |

---

## 6. Cross-Team Wire Format

`WebhookEvent` (ingestion) and `Event` (worker) must serialize to **identical JSON**.
Verified by the cross-team fixture test on both sides:

```json
{
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

`EventStatus` is `snake_case` in both crates. Any rename here is a silent data
corruption — the worker would fail to deserialize existing DynamoDB records.

**SQS message contract** (ingestion writes, worker reads):

```json
{ "event_id": "evt_1a2b3c4d" }
```

`customer_id` travels as a `MessageAttribute` (DataType: `String`), not in the
body. This lets the worker read the routing key without deserializing JSON.

---

## 7. Day 3 — SQS Integration & Webhook Handler

### What was built

**`services/queue.rs` — SQS enqueue**

```rust
pub async fn enqueue_event(
    client: &SqsClient,
    queue_url: &str,
    event_id: &str,
    customer_id: &str,
) -> Result<(), IngestionError>
```

Key design: `customer_id` is a `MessageAttribute`, not in the JSON body. The
worker needs it to look up `webhook_configs` — putting it in the attribute means
zero extra DynamoDB reads on the delivery hot path.

**`handlers/webhook.rs` — the 5-step pipeline**

```
1. req.validate()                  → 422 on empty key, bad chars, payload > 400KB
2. idempotency::check_and_record() → 200 if duplicate (no further writes)
3. events::create_event()          → DynamoDB PutItem, 30d TTL
4. queue::enqueue_event()          → SQS SendMessage + customer_id attribute
5. return 202 Accepted             → { event_id, status: "accepted", created_at }
```

`AppState` wraps both AWS clients + `AppConfig` in an `Arc<AppState>` injected
via Axum's `State` extractor — no global state, no `lazy_static`, fully testable.

### Thinking behind the idempotency design

The naive approach is **read-then-write**: check if the key exists, then write if
not. This has a TOCTOU race — two concurrent requests with the same key both read
"not found" before either writes. Instead we use a **conditional PutItem**
(`attribute_not_exists(pk)`). This is atomic at the DynamoDB level — only one
succeeds; the other gets `ConditionalCheckFailedException`. No locking, no
transactions, no second round-trip.

### Thinking behind 202 vs 200

202 Accepted = "I received your request and will act on it." The event is in SQS
— not yet delivered. Returning 200 OK would imply the work is done. Callers who
need to track delivery status use the returned `event_id` to poll (future Day 5
endpoint). The semantic distinction protects callers from assuming delivered =
accepted.

---

## 8. Day 4 — Config Management & Lambda Entry Point

### What was built

**`services/configs.rs` — DynamoDB CRUD**

```rust
pub async fn put_config(client, table, config) -> Result<(), IngestionError>
pub async fn fetch_config(client, table, customer_id) -> Result<WebhookConfig, IngestionError>
```

`put_config` is an unconditional `PutItem` (upsert). This is intentional — callers
can rotate their signing secret or update their delivery URL by POSTing again.
The PK/SK are injected into the serialized `AttributeValue` map manually, since
`serde_dynamo` serializes struct fields but not the DynamoDB key attributes which
aren't on the struct.

**`handlers/config.rs` — config handlers**

`POST /webhooks/configs`: Accepts `{ customer_id, url, secret? }`. If `secret` is
omitted or empty, generates `whsec_{32 alphanumeric chars}` using `nanoid` with a
custom alphabet. Returns 201 with the full config record.

`GET /webhooks/configs?customer_id=…`: Returns 200 with the config, or 404 if
none registered.

**`src/main.rs` — Lambda entry point**

Cold-start sequence:

```
1. tracing_subscriber::fmt().json()  → structured JSON logs for CloudWatch
2. AppConfig::from_env()             → fail fast if any env var is missing
3. aws_config::load_defaults()       → load execution role credentials
4. build_dynamo_client() + SqsClient → reused across all warm invocations
5. Arc::new(AppState { dynamo, sqs, config })
6. Router::new()
     .route("/webhooks/receive",   post(receive_webhook))
     .route("/webhooks/configs",   post(create_config))
     .route("/webhooks/configs",   get(get_config))
     .with_state(state)
7. lambda_http::run(app)             → blocks; wakes per API Gateway invocation
```

**Why `lambda_http` not raw `lambda_runtime`?** `lambda_http` handles the
API Gateway → HTTP request translation automatically. Handlers are written as
pure Axum handlers — they never see the Lambda event envelope. No code change
is needed to move from API Gateway REST to HTTP API or even to a local dev server.

### Thinking behind secret generation

Secrets use `nanoid` with a custom 62-character alphabet (a-z, A-Z, 0-9) — no
ambiguous symbols (`+`, `/`, `=`) that could cause encoding bugs when the worker
interpolates the secret into an HMAC computation. The `whsec_` prefix makes
secrets distinguishable in logs from other credential types (API keys, tokens).

The caller can supply their own secret (useful for migrating an existing customer
from another platform). An empty string is treated as "generate one" — not as a
valid empty secret — because an empty HMAC key would accept any payload.

---

## 9. Bugs Encountered & Solutions

### Bug 1 — Duplicate `mod tests` block in `worker/src/model.rs` (Day 1)

**Symptom:** `cargo test -p ingestion` failed to compile with:
```
error[E0428]: the name `tests` is defined multiple times
```

**Root cause:** `worker/src/model.rs` had two `#[cfg(test)] mod tests { … }`
blocks — one after the `Event` impl, one at the end of the file. Rust disallows
a module name being defined twice in the same namespace.

**Fix:** Merged into a single `mod tests` block at the bottom. Restored the
accidentally overwritten test name `event_serialization_round_trip`.

**Lesson:** Keep exactly **one** `mod tests` block per file, at the bottom.
`cargo test` catches this immediately; run it before every commit.

---

### Bug 2 — AWS SQS SDK builder accepts missing `string_value` silently (Day 3)

**Symptom:** A test named `customer_id_attribute_rejects_missing_string_value`
failed — the SDK builder did **not** error when `string_value` was omitted.

**Root cause:** Wrong assumption about the SDK. `MessageAttributeValue::builder()`
with only `.data_type("String")` builds successfully. Calling `.string_value()`
on the result returns `None` — it is not validated at build time.

**Fix:** Renamed the test to `customer_id_attribute_string_value_is_none_when_omitted`
and documented the actual SDK behavior. The production code still always provides
`string_value` — only the test expectation was wrong.

**Lesson:** Don't assume SDK builders validate their inputs. Write a test that
documents the actual behavior, not the assumed behavior.

---

### Bug 3 — Copilot review introduced a call to a non-existent function (Day 3 PR)

**Symptom:** CI failed with:
```
error[E0425]: cannot find function `get_existing_event_created_at` in module `idempotency`
help: a function with a similar name exists: `get_existing_event_id`
```

**Root cause:** A Copilot code review suggestion rewrote the duplicate-path
handler to fetch the original `created_at` via `idempotency::get_existing_event_created_at()`
— a function it invented that was never written. The actual function is
`get_existing_event_id()` and returns only the `event_id` string.

**Fix:** Removed the entire invented 23-line block. The duplicate response
uses `unix_now_secs()` for `created_at`, which is the correct semantic —
"when this duplicate request was received", not the original event's timestamp.
Returning the original timestamp would require fetching the full `IdempotencyRecord`,
which is a separate scoped change.

**Lesson:** Always run `cargo build` after accepting AI review suggestions before
merging. AI code review can introduce plausible-looking but non-existent function
names. Never auto-merge without a passing CI build.

---

### Bug 4 — `Cargo.lock` not committed after adding `lambda_http` (Day 4)

**Symptom:** CI `cargo clippy --locked` failed with:
```
error: cannot update the lock file because --locked was passed
```

**Root cause:** `lambda_http = "0.13"` was added to `Cargo.toml` and compiled
locally (which updates `Cargo.lock`), but `Cargo.lock` was not staged in the
commit. CI's `--locked` flag requires the lockfile to exactly match `Cargo.toml`.

**Fix:** `git add Cargo.lock && git commit -m "chore: update Cargo.lock"`.

**Lesson:** Any `Cargo.toml` dependency change must be paired with a `Cargo.lock`
commit. A simple pre-push rule: `cargo build` → `git status` → if `Cargo.lock`
appears, stage it.

---

### Bug 5 — `rustfmt` import ordering and line-length violations (Day 3 CI)

**Symptom:** `cargo fmt --all -- --check` exited 1 in CI with diffs across
`webhook.rs` and `queue.rs`.

**Root cause:** `rustfmt` sorts `use` statements by full module path
alphabetically — `use crate::services::dynamodb::AppConfig` must come before
`use crate::services::{events, idempotency, queue}` because `d` < `{`. Also,
`assert_eq!` calls with three arguments and struct constructors exceeded the
100-character line limit and needed to be expanded.

**Fix:** `cargo fmt --all` (auto-applies all corrections), then verified
`--check` exits 0, then committed the formatted files.

**Lesson:** Run `cargo fmt --all` locally before every push — never hand-format
imports. Add `cargo fmt --all -- --check` to CI so formatting is enforced on
every PR.

---

## 10. Running the Tests

```bash
# All crates from repo root
cargo test --workspace

# Ingestion only
cargo test -p ingestion

# With output (useful for debugging)
cargo test -p ingestion -- --nocapture
```

Expected output (Day 4 state):

```
running 69 tests
test handlers::config::tests::caller_supplied_secret_is_used_as_is ... ok
test handlers::config::tests::config_not_found_maps_to_404 ... ok
test handlers::config::tests::dynamodb_error_maps_to_500 ... ok
test handlers::config::tests::empty_secret_triggers_auto_generation ... ok
test handlers::config::tests::item_not_found_maps_to_404 ... ok
test handlers::config::tests::none_secret_triggers_auto_generation ... ok
test handlers::config::tests::secret_has_whsec_prefix ... ok
test handlers::config::tests::secret_random_part_is_alphanumeric ... ok
test handlers::config::tests::secret_total_length_is_38 ... ok
test handlers::config::tests::secrets_are_unique ... ok
test handlers::config::tests::serialization_error_maps_to_500 ... ok
test handlers::config::tests::unix_now_secs_is_reasonable ... ok
... (57 more)

test result: ok. 69 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

CI checks (must all pass before merge):

```bash
cargo test --workspace                                                       # 79 total (incl. worker)
cargo fmt --all -- --check                                                   # 0 diffs
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
```

---

## 11. Roadmap — What's Next

### Day 5 — Integration Testing & Handoff

| Task | Description |
|---|---|
| Deploy to AWS | `sam build && sam deploy` — verify all 3 tables, SQS queue, both Lambdas created |
| Integration test script | `tests/integration_test.sh` — curl the live API, verify DynamoDB records and SQS messages |
| Verify idempotency end-to-end | POST same `idempotency_key` twice → second returns 200 with original `event_id` |
| Verify worker handoff | Events appear in SQS → worker Lambda receives them → delivery attempts written to DynamoDB |
| Handoff document for Engineer 2 | SQS message format, DynamoDB schemas, sample queries |

### Week 2

| Day | Topic | Goal |
|---|---|---|
| 6 | CloudWatch dashboards | Webhook receive rate, idempotency hit %, error rates, p95 latency |
| 7 | API documentation | OpenAPI spec, customer onboarding guide, Postman collection |
| 8 | CI/CD pipeline | GitHub Actions: test → fmt → clippy → `sam deploy` to staging on merge |
| 9 | Load testing | k6 at 500 req/sec, < 100ms p95, < 0.1% error rate |
| 10 | Final polish | README, demo script, tag v1.0.0 |

### Known technical debt

| Item | Priority | Notes |
|---|---|---|
| `WebhookEvent` TTL field missing from DynamoDB serialization | High | `ttl` attribute needs to be injected at write time (same pattern as configs PK/SK) |
| Config validation (URL format, secret length) | Medium | `CreateConfigRequest` has no `validate()` method yet |
| `GET /webhooks/configs` authentication | Medium | Currently anyone can read any customer's config by guessing `customer_id` |
| `duplicate` response `created_at` returns "now" not original timestamp | Low | Would require fetching full `IdempotencyRecord`; scoped for Week 2 |
| Integration tests | High | Needed before any production traffic |
