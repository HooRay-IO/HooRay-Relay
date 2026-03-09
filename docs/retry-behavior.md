# Worker Retry Behavior

## Scope

This document defines the current delivery retry behavior implemented by the worker as of March 9, 2026.

## Summary

- Successful `2xx` delivery: mark event `delivered`, delete SQS message.
- Terminal delivery failure: mark event `failed`, delete SQS message.
- Retryable delivery failure: keep the SQS message, update visibility timeout, and try again later.

## Retryable vs Terminal Outcomes

### Retryable

These classes are retried:
- `http_rate_limited`
- `http_server_error`
- `http_other`
- `network_timeout`
- `network_connect`
- `network_request`
- `dynamodb_error`
- `sqs_error`

### Terminal

These classes are terminal:
- `http_client_error`
- `transport_other`
- `event_missing`
- `config_missing`
- `config_inactive`
- `invalid_queue_message`
- `serialization_error`
- `internal_error`

## Max Attempts

Current worker default:
- `retry_attempts = 5`

This comes from [`worker/src/resilience/mod.rs`](/Users/yizehu/Workspaces/HooRay-Relay/worker/src/resilience/mod.rs).

Important nuance:
- the worker records every delivery attempt in DynamoDB,
- retry decisions are based on resilience state and delivery classification,
- customer config still contains `max_retries`, but the current Week 2 worker path is governed by the resilience config in code.

If this distinction is not desired, align the code and contract before production release.

## Backoff Strategy

The worker uses exponential backoff with jitter:
- base delay: `5s`
- multiplier: `2`
- max delay: `5m`
- jitter: up to `1s`

The effective retry delay is converted into an SQS visibility timeout with clamps:
- minimum visibility timeout: `30s`
- maximum visibility timeout: `1h`
- processing overhead added before clamp: `15s`

## Circuit Breaker Interaction

The worker also applies a per-endpoint circuit breaker:
- failure threshold: `5` consecutive failures
- recovery timeout: `1m`

When the breaker is open:
- new requests to that endpoint are blocked,
- the message is kept for retry,
- the next retry delay is based on the breaker probe time.

## Event State Transitions

For a retryable failure:
1. write `ATTEMPT#n`
2. increment `attempt_count`
3. keep event status as `pending`
4. update SQS visibility timeout for the next attempt

For a terminal failure:
1. write `ATTEMPT#n` when a delivery was attempted
2. increment `attempt_count`
3. mark event `failed`
4. delete the SQS message

For success:
1. write `ATTEMPT#n`
2. increment `attempt_count`
3. mark event `delivered`
4. set `delivered_at`
5. delete the SQS message

## Operator Guidance

- Do not force retries by directly editing `attempt_count` or `status`.
- Fix the root cause first.
- Replay from the DLQ using [`docs/runbook.md`](/Users/yizehu/Workspaces/HooRay-Relay/docs/runbook.md).
- Verify the latest behavior against logs and DynamoDB attempt rows after any replay.
