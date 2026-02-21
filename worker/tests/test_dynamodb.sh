#!/usr/bin/env bash
set -euo pipefail

# Day 2 DynamoDB smoke test for Engineer 2 worker service.
# Creates test event/config items and verifies they can be fetched.

: "${AWS_REGION:?AWS_REGION is required}"
: "${WEBHOOK_EVENTS_TABLE:?WEBHOOK_EVENTS_TABLE is required}"
: "${WEBHOOK_CONFIGS_TABLE:?WEBHOOK_CONFIGS_TABLE is required}"

EVENT_ID="${EVENT_ID:-evt_test_$(date +%s)}"
CUSTOMER_ID="${CUSTOMER_ID:-cust_test_$(date +%s)}"
TS_NOW="$(date +%s)"

EVENT_PK="EVENT#${EVENT_ID}"
EVENT_SK="v0"
CONFIG_PK="CUSTOMER#${CUSTOMER_ID}"
CONFIG_SK="CONFIG"

echo "[1/4] Writing test event to ${WEBHOOK_EVENTS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$WEBHOOK_EVENTS_TABLE" \
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

echo "[2/4] Writing test config to ${WEBHOOK_CONFIGS_TABLE}"
aws dynamodb put-item \
  --region "$AWS_REGION" \
  --table-name "$WEBHOOK_CONFIGS_TABLE" \
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

echo "[3/4] Fetching event by pk/sk"
EVENT_ID_FETCHED="$(aws dynamodb get-item \
  --consistent-read \
  --region "$AWS_REGION" \
  --table-name "$WEBHOOK_EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"${EVENT_SK}\"}}" \
  --query 'Item.event_id.S' \
  --output text)"

echo "[4/4] Fetching config by pk/sk"
CONFIG_CUSTOMER_ID_FETCHED="$(aws dynamodb get-item \
  --consistent-read \
  --region "$AWS_REGION" \
  --table-name "$WEBHOOK_CONFIGS_TABLE" \
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
