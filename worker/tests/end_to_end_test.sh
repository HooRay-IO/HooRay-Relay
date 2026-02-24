#!/usr/bin/env bash
set -euo pipefail

# End-to-end integration test for Engineer 2 worker flow.
# Flow:
# 1) Seed event + webhook config in DynamoDB
# 2) Send SQS message with {"event_id":"..."}
# 3) Optionally trigger worker command (for local/dev workflows)
# 4) Poll DynamoDB for ATTEMPT#1 and final event status
#
# Required env:
#   AWS_REGION
#   QUEUE_URL (or WEBHOOK_QUEUE_URL)
#   EVENTS_TABLE (or WEBHOOK_EVENTS_TABLE)
#   CONFIGS_TABLE (or WEBHOOK_CONFIGS_TABLE)
#   DELIVERY_URL (required unless SKIP_DELIVERY=true)
#
# Optional env:
#   DELIVERY_SECRET=whsec_test_secret
#   MAX_RETRIES=3
#   EXPECTED_EVENT_STATUS=delivered
#   POLL_TIMEOUT_SECS=90
#   POLL_INTERVAL_SECS=3
#   RUN_WORKER_CMD=""   # e.g. "cargo run -p worker -- --once"
#   SKIP_DELIVERY=false # true => skip worker trigger + delivery assertions
#   KEEP_TEST_DATA=false

require_one_of() {
  local var_name_a="$1"
  local var_name_b="$2"
  local value_a="${!var_name_a:-}"
  local value_b="${!var_name_b:-}"

  if [[ -n "$value_a" ]]; then
    printf '%s' "$value_a"
    return 0
  fi
  if [[ -n "$value_b" ]]; then
    printf '%s' "$value_b"
    return 0
  fi

  echo "ERROR: set ${var_name_a} or ${var_name_b}" >&2
  exit 1
}

require_set() {
  local var_name="$1"
  if [[ -z "${!var_name:-}" ]]; then
    echo "ERROR: ${var_name} is required" >&2
    exit 1
  fi
}

require_set "AWS_REGION"

QUEUE_URL="$(require_one_of "QUEUE_URL" "WEBHOOK_QUEUE_URL")"
EVENTS_TABLE="$(require_one_of "EVENTS_TABLE" "WEBHOOK_EVENTS_TABLE")"
CONFIGS_TABLE="$(require_one_of "CONFIGS_TABLE" "WEBHOOK_CONFIGS_TABLE")"

SKIP_DELIVERY="${SKIP_DELIVERY:-false}"
DELIVERY_URL="${DELIVERY_URL:-}"
if [[ -z "$DELIVERY_URL" ]]; then
  if [[ "$SKIP_DELIVERY" != "true" ]]; then
    echo "[INIT] DELIVERY_URL not set; forcing SKIP_DELIVERY=true"
    SKIP_DELIVERY="true"
  fi
  # Placeholder only for config shape validation when delivery is skipped.
  DELIVERY_URL="https://example.invalid/webhook"
fi

DELIVERY_SECRET="${DELIVERY_SECRET:-whsec_test_secret}"
MAX_RETRIES="${MAX_RETRIES:-3}"
EXPECTED_EVENT_STATUS="${EXPECTED_EVENT_STATUS:-delivered}"
POLL_TIMEOUT_SECS="${POLL_TIMEOUT_SECS:-90}"
POLL_INTERVAL_SECS="${POLL_INTERVAL_SECS:-3}"
RUN_WORKER_CMD="${RUN_WORKER_CMD:-}"
KEEP_TEST_DATA="${KEEP_TEST_DATA:-false}"

EVENT_ID="${EVENT_ID:-evt_e2e_$(date +%s%N)}"
CUSTOMER_ID="${CUSTOMER_ID:-cust_e2e_$(date +%s%N)}"
TS_NOW="$(date +%s)"

EVENT_PK="EVENT#${EVENT_ID}"
EVENT_SK="v0"
ATTEMPT_SK="ATTEMPT#1"
CONFIG_PK="CUSTOMER#${CUSTOMER_ID}"
CONFIG_SK="CONFIG"

EVENT_WRITTEN=false
CONFIG_WRITTEN=false
ATTEMPT_WRITTEN=false

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$KEEP_TEST_DATA" == "true" ]]; then
    echo "[CLEANUP] KEEP_TEST_DATA=true, skipping item deletion"
    return "$exit_code"
  fi

  if [[ "$ATTEMPT_WRITTEN" == "true" ]]; then
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${ATTEMPT_SK}\"}}" \
      >/dev/null 2>&1 || true
  fi

  if [[ "$EVENT_WRITTEN" == "true" ]]; then
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${EVENT_SK}\"}}" \
      >/dev/null 2>&1 || true
  fi

  if [[ "$CONFIG_WRITTEN" == "true" ]]; then
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$CONFIGS_TABLE" \
      --key "{\"pk\":{\"S\":\"${CONFIG_PK}\"},\"sk\":{\"S\":\"${CONFIG_SK}\"}}" \
      >/dev/null 2>&1 || true
  fi

  return "$exit_code"
}

trap cleanup EXIT INT TERM

echo "[1/6] Writing event metadata to ${EVENTS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$EVENTS_TABLE" \
  --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
  --item "{
    \"pk\": {\"S\": \"${EVENT_PK}\"},
    \"sk\": {\"S\": \"${EVENT_SK}\"},
    \"event_id\": {\"S\": \"${EVENT_ID}\"},
    \"customer_id\": {\"S\": \"${CUSTOMER_ID}\"},
    \"payload\": {\"S\": \"{\\\"test\\\":\\\"e2e\\\",\\\"event_id\\\":\\\"${EVENT_ID}\\\"}\"},
    \"status\": {\"S\": \"pending\"},
    \"attempt_count\": {\"N\": \"0\"},
    \"created_at\": {\"N\": \"${TS_NOW}\"}
  }" >/dev/null
EVENT_WRITTEN=true

echo "[2/6] Writing webhook config to ${CONFIGS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$CONFIGS_TABLE" \
  --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
  --item "{
    \"pk\": {\"S\": \"${CONFIG_PK}\"},
    \"sk\": {\"S\": \"${CONFIG_SK}\"},
    \"customer_id\": {\"S\": \"${CUSTOMER_ID}\"},
    \"url\": {\"S\": \"${DELIVERY_URL}\"},
    \"secret\": {\"S\": \"${DELIVERY_SECRET}\"},
    \"max_retries\": {\"N\": \"${MAX_RETRIES}\"},
    \"active\": {\"BOOL\": true},
    \"created_at\": {\"N\": \"${TS_NOW}\"},
    \"updated_at\": {\"N\": \"${TS_NOW}\"}
  }" >/dev/null
CONFIG_WRITTEN=true

echo "[3/6] Sending SQS message for event ${EVENT_ID}"
aws sqs send-message \
  --region "$AWS_REGION" \
  --queue-url "$QUEUE_URL" \
  --message-body "{\"event_id\":\"${EVENT_ID}\"}" \
  --message-attributes "{
    \"customer_id\":{\"DataType\":\"String\",\"StringValue\":\"${CUSTOMER_ID}\"}
  }" >/dev/null

if [[ "$SKIP_DELIVERY" == "true" ]]; then
  echo "[4/6] SKIP_DELIVERY=true, not triggering worker or asserting attempts/status"
  echo "[5/6] Contract checks complete (seed + enqueue succeeded)"
  echo "[6/6] SUCCESS: partial integration flow passed"
  echo "EVENT_ID=${EVENT_ID} CUSTOMER_ID=${CUSTOMER_ID} SKIP_DELIVERY=true"
  exit 0
fi

echo "[4/6] Triggering worker (if RUN_WORKER_CMD is provided)"
if [[ -n "$RUN_WORKER_CMD" ]]; then
  echo "      Running: ${RUN_WORKER_CMD}"
  eval "$RUN_WORKER_CMD"
else
  echo "      RUN_WORKER_CMD not set; waiting for deployed SQS-triggered worker."
fi

echo "[5/6] Polling for ATTEMPT#1 (timeout=${POLL_TIMEOUT_SECS}s)"
deadline=$((TS_NOW + POLL_TIMEOUT_SECS))
attempt_seen=false
while [[ "$(date +%s)" -lt "$deadline" ]]; do
  got_attempt_pk="$(aws dynamodb get-item \
    --region "$AWS_REGION" \
    --table-name "$EVENTS_TABLE" \
    --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${ATTEMPT_SK}\"}}" \
    --query 'Item.pk.S' \
    --output text 2>/dev/null || true)"

  if [[ "$got_attempt_pk" == "$EVENT_PK" ]]; then
    attempt_seen=true
    ATTEMPT_WRITTEN=true
    break
  fi
  sleep "$POLL_INTERVAL_SECS"
done

if [[ "$attempt_seen" != "true" ]]; then
  echo "ERROR: ATTEMPT#1 was not recorded for ${EVENT_ID} within ${POLL_TIMEOUT_SECS}s"
  exit 1
fi

echo "[6/6] Validating event status == ${EXPECTED_EVENT_STATUS}"
final_status="$(aws dynamodb get-item \
  --region "$AWS_REGION" \
  --table-name "$EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${EVENT_SK}\"}}" \
  --query 'Item.status.S' \
  --output text)"

if [[ "$final_status" != "$EXPECTED_EVENT_STATUS" ]]; then
  echo "ERROR: status mismatch expected=${EXPECTED_EVENT_STATUS} actual=${final_status}"
  exit 1
fi

echo "SUCCESS: end-to-end flow passed"
echo "EVENT_ID=${EVENT_ID} CUSTOMER_ID=${CUSTOMER_ID} STATUS=${final_status}"
