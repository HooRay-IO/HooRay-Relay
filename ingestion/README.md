# Ingestion Service — Engineer 1, Day 1

> **Sprint context:** This is the ingestion-side Lambda of the HooRay-Relay webhook
> delivery system. Engineer 1 owns the ingestion pipeline; Engineer 2 owns the
> delivery worker (`../worker/`). This document covers everything that was built
> and debugged on Day 1 of the sprint.

---

## Table of Contents

1. [What Was Built Today](#1-what-was-built-today)
2. [Project Structure](#2-project-structure)
3. [Dependency Choices](#3-dependency-choices)
4. [Data Models](#4-data-models)
5. [DynamoDB Key Contracts](#5-dynamodb-key-contracts)
6. [Cross-Team Wire Format](#6-cross-team-wire-format)
7. [Running the Tests](#7-running-the-tests)
8. [Debug Log — Engineer 2 Fix](#8-debug-log--engineer-2-fix)
9. [Day 2 Preview](#9-day-2-preview)

---

## 1. What Was Built Today

| Deliverable | File | Status |
|---|---|---|
| Rust project scaffold | `Cargo.toml` | ✅ |
| Data models | `src/model.rs` | ✅ |
| Module entry point | `src/main.rs` | ✅ (stub) |
| Unit tests | `src/model.rs` (`mod tests`) | ✅ 11/11 passing |

**Day 1 goal (from `ENGINEER_1_TIMELINE.md`):**
> Initialize Rust ingestion project · Update `Cargo.toml` · Define data models
> in `src/models.rs` · Initialize git repository with first commit

---

## 2. Project Structure

```
ingestion/
├── Cargo.toml          ← dependencies & crate metadata
└── src/
    ├── main.rs         ← crate root; module declarations (stub for now)
    └── model.rs        ← all data types, DTOs, and error enums
```

Future days will add:

```
src/
├── handlers/
│   ├── mod.rs
│   ├── webhook.rs      ← Day 3: POST /webhooks/receive
│   └── config.rs       ← Day 4: POST/GET /webhooks/configs
└── services/
    ├── mod.rs
    ├── idempotency.rs  ← Day 2
    ├── events.rs       ← Day 2
    └── queue.rs        ← Day 3
```

---

## 3. Dependency Choices

```toml
# serde — serialization to/from JSON and DynamoDB
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# thiserror — typed, ergonomic error enums
thiserror = "2.0"

# tracing / tracing-subscriber — structured JSON logging for CloudWatch
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }

# tokio — async runtime required by all AWS SDKs and axum
tokio = { version = "1", features = ["full"] }

# axum — HTTP framework for the Lambda API server
axum = "0.8"                            # ← ingestion-only; worker has no HTTP server

# AWS SDKs — identical versions to worker/ to keep the lock-file in sync
aws-config        = "1.8.14"
aws-sdk-dynamodb  = "1"
aws-sdk-sqs       = "1.94.0"
serde_dynamo      = { version = "4", features = ["aws-sdk-dynamodb+1"] }

# nanoid — generates evt_… event IDs
nanoid = "0.4"                          # ← ingestion-only; worker only reads IDs
```

**Version alignment with `worker/Cargo.toml`:**

| Crate | `worker/` | `ingestion/` |
|---|---|---|
| `serde` | `1` | `1` |
| `thiserror` | `2.0` | `2.0` |
| `tracing` | `0.1` | `0.1` |
| `tokio` | `1` (full) | `1` (full) |
| `aws-config` | `1.8.14` | `1.8.14` |
| `aws-sdk-dynamodb` | `1` | `1` |
| `aws-sdk-sqs` | `1.94.0` | `1.94.0` |
| `serde_dynamo` | `4` + feature | `4` + feature |

Both crates resolve to the same dependency graph for the shared AWS SDK crates,
which means `Cargo.lock` avoids duplicate compilations of the heavy AWS SDK.

---

## 4. Data Models

All types live in `src/model.rs`. Here is a quick reference:

### API boundary types

| Type | HTTP surface | Purpose |
|---|---|---|
| `WebhookReceiveRequest` | `POST /webhooks/receive` body | Inbound event from caller |
| `WebhookReceiveResponse` | Response (202 or 200) | Returns `event_id` + status |
| `ReceiveStatus` | enum field inside response | `accepted` \| `duplicate` |
| `CreateConfigRequest` | `POST /webhooks/configs` body | Register a new customer endpoint |
| `WebhookConfigResponse` | Response (201 or 200) | Config record DTO |

### DynamoDB entity types

| Type | Table | SK pattern |
|---|---|---|
| `WebhookEvent` | `webhook_events` | `v0` (metadata) |
| `WebhookConfig` | `webhook_configs` | `CONFIG` |
| `IdempotencyRecord` | `webhook_idempotency` | _(PK-only table)_ |

### Queue type

| Type | Where used | Purpose |
|---|---|---|
| `QueueMessage` | Written by ingestion → read by worker | Carries `event_id` over SQS |

### Error enum

```rust
pub enum IngestionError {
    MissingField(String),
    ConfigNotFound(String),
    AlreadyExists(String),
    Serialization(String),
    DynamoDb(String),
    Sqs(String),
    ItemNotFound { entity: &'static str, key: String },
    DecodeDynamo(String),   // From<serde_dynamo::Error> impl included
}
```

Mirrors the shape of `WorkerError` in `worker/src/model.rs` so both crates
handle errors consistently.

---

## 5. DynamoDB Key Contracts

These are enforced by the `pk()` / `sk()` helper methods on each struct and
verified by unit tests — changing them is a breaking change for both engineers.

| Table | PK format | SK format | Example |
|---|---|---|---|
| `webhook_events` | `EVENT#{event_id}` | `v0` (metadata) | `EVENT#evt_1a2b3c4d` / `v0` |
| `webhook_events` | `EVENT#{event_id}` | `ATTEMPT#{n}` | `EVENT#evt_1a2b3c4d` / `ATTEMPT#1` |
| `webhook_configs` | `CUSTOMER#{customer_id}` | `CONFIG` | `CUSTOMER#cust_xyz123` / `CONFIG` |
| `webhook_idempotency` | `IDEM#{idempotency_key}` | _(none)_ | `IDEM#req_abc123` |

---

## 6. Cross-Team Wire Format

`WebhookEvent` (ingestion) and `Event` (worker) serialize to **identical JSON**.
This is verified by `webhook_event_deserializes_from_worker_fixture` — the
ingestion model must deserialize the exact fixture that the worker model produces:

```json
{
  "event_id":     "evt_1a2b3c4d",
  "customer_id":  "cust_xyz123",
  "payload":      "{\"order_id\":\"ord_123\",\"amount\":99.99}",
  "status":       "pending",
  "attempt_count": 0,
  "created_at":   1707840000,
  "delivered_at": null,
  "next_retry_at": null
}
```

`EventStatus` serializes as **`snake_case`** in both crates (`pending`,
`delivered`, `failed`). Any change here must be coordinated with Engineer 2.

**`QueueMessage`** is the SQS handoff contract:

```json
{ "event_id": "evt_1a2b3c4d" }
```

`customer_id` is sent alongside as an SQS `MessageAttribute` (set in
`services/queue.rs`, Day 3) so the worker can route without a DynamoDB read.

---

## 7. Running the Tests

**Prerequisites — install Rust once:**

```zsh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
# Add to ~/.zshrc to make permanent:
echo 'source "$HOME/.cargo/env"' >> ~/.zshrc
source "$HOME/.cargo/env"
```

**Run ingestion tests:**

```zsh
source "$HOME/.cargo/env"
cd ingestion
cargo test
```

Expected output:

```
running 11 tests
test model::tests::idempotency_ttl_is_24_hours_after_created_at ............. ok
test model::tests::event_status_serializes_as_snake_case ..................... ok
test model::tests::idempotency_record_pk_matches_dynamodb_contract ........... ok
test model::tests::receive_status_serializes_as_snake_case ................... ok
test model::tests::webhook_config_key_helpers_match_dynamodb_contract ........ ok
test model::tests::webhook_config_to_response_copies_all_fields .............. ok
test model::tests::webhook_event_key_helpers_match_dynamodb_contract ......... ok
test model::tests::webhook_event_new_has_pending_status_and_zero_attempts .... ok
test model::tests::webhook_event_deserializes_from_worker_fixture ............ ok
test model::tests::webhook_event_serialization_round_trip .................... ok
test model::tests::webhook_receive_request_round_trip ........................ ok

test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

**Run worker tests (Engineer 2 baseline):**

```zsh
cd ../worker
cargo test
```

Expected output:

```
running 6 tests
test model::tests::event_key_helpers_match_dynamodb_contract ................. ok
test model::tests::event_deserializes_from_ingestion_fixture ................. ok
test model::tests::event_serialization_round_trip ............................ ok
test model::tests::retry_and_terminal_transitions ............................ ok
test model::tests::mark_failed_clears_next_retry_and_sets_status_failed ...... ok
test model::tests::status_serializes_as_snake_case ........................... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
```

---

## 8. Debug Log — Engineer 2 Fix

Before any ingestion tests could run, a bug in `worker/src/model.rs` was found
and fixed. This section documents what happened and why, so both engineers
understand the change.

### Root cause

`worker/src/model.rs` had **two `mod tests` blocks** at the top level of the
same module. Rust does not allow a module to be defined twice in the same
namespace — the compiler rejects it with:

```
error[E0428]: the name `tests` is defined multiple times
   --> src/model.rs:211:1
    |
 82 | mod tests {
    | --------- previous definition of the module `tests` here
211 | mod tests {
    | ^^^^^^^^^ `tests` redefined here
```

### What happened

The file was written with one `#[cfg(test)] mod tests { … }` block right after
the `Event` impl (line 82), and a second one at the very bottom of the file
(line 211). This is consistent with how tests are sometimes incrementally added
to a file — one block per struct — but Rust requires all tests for a single
module to live in a **single** `mod tests` block.

### The fix (diff summary)

**`worker/src/model.rs`**

1. **Removed** the first `#[cfg(test)] mod tests { … }` block (which contained
   only `mark_failed_clears_next_retry_and_sets_status_failed`).
2. **Moved** `mark_failed_clears_next_retry_and_sets_status_failed` into the
   single unified `mod tests` block at the bottom of the file, as the first
   test in the block.
3. Fixed the second block's first test name — it had accidentally been renamed
   to `mark_failed_clears_next_retry_and_sets_status_failed` during editing,
   shadowing the real name `event_serialization_round_trip`. Restored the
   correct name.

### Final state of `worker/src/model.rs`

One unified `#[cfg(test)] mod tests { … }` block at the bottom containing all
six tests in logical order:

| # | Test name | What it checks |
|---|---|---|
| 1 | `mark_failed_clears_next_retry_and_sets_status_failed` | Terminal state transition |
| 2 | `event_serialization_round_trip` | Full serde round-trip |
| 3 | `event_deserializes_from_ingestion_fixture` | Wire-format cross-team compat |
| 4 | `status_serializes_as_snake_case` | `snake_case` serde contract |
| 5 | `event_key_helpers_match_dynamodb_contract` | PK/SK key format |
| 6 | `retry_and_terminal_transitions` | State machine transitions |

### How to prevent this in future

- Keep **one** `mod tests` block per file, at the bottom, no matter how many
  structs are in the file.
- Run `cargo test` locally before committing — the compiler catches this
  immediately.
- Add `cargo test` as a required check in the CI pipeline (planned for Day 8).

---

## 9. Day 2 Preview

Per `ENGINEER_1_TIMELINE.md`, tomorrow's work:

| Task | File | Description |
|---|---|---|
| Idempotency service | `src/services/idempotency.rs` | Conditional DynamoDB `PutItem` with `attribute_not_exists(pk)` |
| Event storage service | `src/services/events.rs` | Write `WebhookEvent` to DynamoDB with 30-day TTL |
| Service module | `src/services/mod.rs` | Export both services |
| Unit tests | both service files | Verify event ID format, TTL math |

The idempotency service is the most critical component — it is the only thing
that prevents the same external event from being delivered to a customer twice.
The conditional write pattern looks like this:

```rust
dynamo.put_item()
    .table_name(&self.idempotency_table)
    .item("pk",          AttributeValue::S(IdempotencyRecord::pk_for(key)))
    .item("event_id",    AttributeValue::S(event_id.clone()))
    .item("created_at",  AttributeValue::N(now.to_string()))
    .item("ttl",         AttributeValue::N((now + 86_400).to_string()))
    .condition_expression("attribute_not_exists(pk)")
    .send()
    .await
```

If a record with the same `pk` already exists, DynamoDB returns a
`ConditionalCheckFailedException` — the service catches this, looks up the
existing `event_id`, and returns it to the handler so the API can respond
`200 Duplicate` instead of `202 Accepted`.
