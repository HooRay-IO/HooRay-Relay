#!/usr/bin/env bash
set -euo pipefail

# Day 2 DynamoDB smoke test for Engineer 2 worker service.
# Creates test event/config items and verifies they can be fetched.

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

: "${AWS_REGION:?AWS_REGION is required}"
EVENTS_TABLE="$(require_one_of "EVENTS_TABLE" "WEBHOOK_EVENTS_TABLE")"
CONFIGS_TABLE="$(require_one_of "CONFIGS_TABLE" "WEBHOOK_CONFIGS_TABLE")"

EVENT_ID="${EVENT_ID:-evt_test_$(date +%s%N)}"
CUSTOMER_ID="${CUSTOMER_ID:-cust_test_$(date +%s%N)}"
TS_NOW="$(date +%s)"

EVENT_PK="EVENT#${EVENT_ID}"
EVENT_SK="v0"
CONFIG_PK="CUSTOMER#${CUSTOMER_ID}"
CONFIG_SK="CONFIG"
KEEP_TEST_DATA="${KEEP_TEST_DATA:-false}"
EVENT_WRITTEN=false
CONFIG_WRITTEN=false

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$KEEP_TEST_DATA" == "true" ]]; then
    echo "[CLEANUP] KEEP_TEST_DATA=true, skipping item deletion"
    return "$exit_code"
  fi

  if [[ "$EVENT_WRITTEN" == "true" ]]; then
    echo "[CLEANUP] Deleting test event from ${EVENTS_TABLE}"
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${EVENT_SK}\"}}" \
      >/dev/null 2>&1 || echo "[CLEANUP] WARN: failed to delete event item"
  fi

  if [[ "$CONFIG_WRITTEN" == "true" ]]; then
    echo "[CLEANUP] Deleting test config from ${CONFIGS_TABLE}"
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$CONFIGS_TABLE" \
      --key "{\"pk\":{\"S\":\"${CONFIG_PK}\"},\"sk\":{\"S\":\"${CONFIG_SK}\"}}" \
      >/dev/null 2>&1 || echo "[CLEANUP] WARN: failed to delete config item"
  fi

  if [[ "$EVENT_WRITTEN" == "true" || "$CONFIG_WRITTEN" == "true" ]]; then
    echo "[CLEANUP] Completed cleanup (exit_code=${exit_code})"
  fi

  return "$exit_code"
}

trap cleanup EXIT INT TERM

echo "[1/4] Writing test event to ${EVENTS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$EVENTS_TABLE" \
  --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
  --item "{
    \"pk\": {\"S\": \"${EVENT_PK}\"},
    \"sk\": {\"S\": \"${EVENT_SK}\"},
    \"event_id\": {\"S\": \"${EVENT_ID}\"},
    \"customer_id\": {\"S\": \"${CUSTOMER_ID}\"},
    \"payload\": {\"S\": \"{\\\"order_id\\\":\\\"ord_test\\\",\\\"amount\\\":99.99}\"},
    \"status\": {\"S\": \"pending\"},
    \"attempt_count\": {\"N\": \"0\"},
    \"created_at\": {\"N\": \"${TS_NOW}\"}
  }"
EVENT_WRITTEN=true

echo "[2/4] Writing test config to ${CONFIGS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$CONFIGS_TABLE" \
  --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
  --item "{
    \"pk\": {\"S\": \"${CONFIG_PK}\"},
    \"sk\": {\"S\": \"${CONFIG_SK}\"},
    \"customer_id\": {\"S\": \"${CUSTOMER_ID}\"},
    \"url\": {\"S\": \"https://webhook.site/example\"},
    \"secret\": {\"S\": \"whsec_test_secret\"},
    \"max_retries\": {\"N\": \"3\"},
    \"active\": {\"BOOL\": true},
    \"created_at\": {\"N\": \"${TS_NOW}\"},
    \"updated_at\": {\"N\": \"${TS_NOW}\"}
  }"
CONFIG_WRITTEN=true

echo "[3/4] Fetching event by pk/sk"
EVENT_ID_FETCHED="$(aws dynamodb get-item \
  --consistent-read \
  --region "$AWS_REGION" \
  --table-name "$EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${EVENT_SK}\"}}" \
  --query 'Item.event_id.S' \
  --output text)"

echo "[4/4] Fetching config by pk/sk"
CONFIG_CUSTOMER_ID_FETCHED="$(aws dynamodb get-item \
  --consistent-read \
  --region "$AWS_REGION" \
  --table-name "$CONFIGS_TABLE" \
  --key "{\"pk\":{\"S\":\"${CONFIG_PK}\"},\"sk\":{\"S\":\"${CONFIG_SK}\"}}" \
  --query 'Item.customer_id.S' \
  --output text)"

if [[ "$EVENT_ID_FETCHED" != "$EVENT_ID" ]]; then
  echo "ERROR: event fetch mismatch expected=${EVENT_ID} got=${EVENT_ID_FETCHED}"
  exit 1
fi

if [[ "$CONFIG_CUSTOMER_ID_FETCHED" != "$CUSTOMER_ID" ]]; then
  echo "ERROR: config fetch mismatch expected=${CUSTOMER_ID} got=${CONFIG_CUSTOMER_ID_FETCHED}"
  exit 1
fi

echo "SUCCESS: DynamoDB test data created and fetch verified"
echo "EVENT_ID=${EVENT_ID} CUSTOMER_ID=${CUSTOMER_ID}"
