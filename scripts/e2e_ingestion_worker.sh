#!/usr/bin/env bash
set -euo pipefail

# Full integration e2e:
# Ingestion API -> DynamoDB + SQS -> ECS worker -> DynamoDB delivery state
#
# Defaults:
# - AWS_REGION=us-west-2
# - AWS_PROFILE=hooray-dev
# - STACK_NAME=hooray-dev
#
# Optional:
# - DELIVERY_URL=https://httpbin.org/post
# - DELIVERY_SECRET=whsec_e2e_test
# - POLL_TIMEOUT_SECS=180
# - POLL_INTERVAL_SECS=5
# - EXPECTED_STATUS=delivered
# - KEEP_TEST_DATA=false

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

require_cmd aws
require_cmd curl
require_cmd jq

AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_PROFILE="${AWS_PROFILE:-hooray-dev}"
STACK_NAME="${STACK_NAME:-hooray-dev}"

DELIVERY_URL="${DELIVERY_URL:-https://httpbin.org/post}"
DELIVERY_SECRET="${DELIVERY_SECRET:-whsec_e2e_test}"
POLL_TIMEOUT_SECS="${POLL_TIMEOUT_SECS:-180}"
POLL_INTERVAL_SECS="${POLL_INTERVAL_SECS:-5}"
EXPECTED_STATUS="${EXPECTED_STATUS:-delivered}"
KEEP_TEST_DATA="${KEEP_TEST_DATA:-false}"

STACK_OUTPUTS_JSON="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --query "Stacks[0].Outputs" \
  --output json)"

API_URL="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="IngestionApiUrl") | .OutputValue')"
EVENTS_TABLE="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="EventsTableName") | .OutputValue')"
CONFIGS_TABLE="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="ConfigsTableName") | .OutputValue')"
IDEMPOTENCY_TABLE="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="IdempotencyTableName") | .OutputValue')"

if [[ -z "$API_URL" || -z "$EVENTS_TABLE" || -z "$CONFIGS_TABLE" || -z "$IDEMPOTENCY_TABLE" ]]; then
  echo "ERROR: missing required stack outputs from $STACK_NAME" >&2
  exit 1
fi

TS="$(date +%s)"
RAND="$(date +%s%N | tail -c 7)"
CUSTOMER_ID="cust_e2e_${TS}_${RAND}"
IDEMPOTENCY_KEY="req_e2e_${TS}_${RAND}"
EVENT_ID=""

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$KEEP_TEST_DATA" == "true" ]]; then
    echo "[CLEANUP] KEEP_TEST_DATA=true, skipping delete"
    return "$exit_code"
  fi

  if [[ -n "$EVENT_ID" ]]; then
    local event_pk="EVENT#${EVENT_ID}"
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${event_pk}\"},\"sk\":{\"S\":\"ATTEMPT#1\"}}" >/dev/null 2>&1 || true
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${event_pk}\"},\"sk\":{\"S\":\"v0\"}}" >/dev/null 2>&1 || true
  fi

  aws dynamodb delete-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$CONFIGS_TABLE" \
    --key "{\"pk\":{\"S\":\"CUSTOMER#${CUSTOMER_ID}\"},\"sk\":{\"S\":\"CONFIG\"}}" >/dev/null 2>&1 || true

  aws dynamodb delete-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$IDEMPOTENCY_TABLE" \
    --key "{\"pk\":{\"S\":\"IDEM#${IDEMPOTENCY_KEY}\"}}" >/dev/null 2>&1 || true

  return "$exit_code"
}

trap cleanup EXIT INT TERM

echo "[1/5] Creating webhook config via ingestion API"
CONFIG_CODE="$(curl -sS -o /tmp/e2e_config_resp.json -w "%{http_code}" \
  -X POST "${API_URL}webhooks/configs" \
  -H 'content-type: application/json' \
  -d "{\"customer_id\":\"${CUSTOMER_ID}\",\"url\":\"${DELIVERY_URL}\",\"secret\":\"${DELIVERY_SECRET}\"}")"

if [[ "$CONFIG_CODE" != "201" && "$CONFIG_CODE" != "200" ]]; then
  echo "ERROR: config API failed with status $CONFIG_CODE"
  cat /tmp/e2e_config_resp.json
  exit 1
fi

echo "[2/5] Sending webhook event via ingestion API"
RECEIVE_CODE="$(curl -sS -o /tmp/e2e_receive_resp.json -w "%{http_code}" \
  -X POST "${API_URL}webhooks/receive" \
  -H 'content-type: application/json' \
  -d "{\"idempotency_key\":\"${IDEMPOTENCY_KEY}\",\"customer_id\":\"${CUSTOMER_ID}\",\"data\":{\"test\":\"e2e\",\"source\":\"ingestion-worker\"}}")"

if [[ "$RECEIVE_CODE" != "202" && "$RECEIVE_CODE" != "200" ]]; then
  echo "ERROR: receive API failed with status $RECEIVE_CODE"
  cat /tmp/e2e_receive_resp.json
  exit 1
fi

EVENT_ID="$(jq -r '.event_id' /tmp/e2e_receive_resp.json)"
if [[ -z "$EVENT_ID" || "$EVENT_ID" == "null" ]]; then
  echo "ERROR: missing event_id in ingestion response"
  cat /tmp/e2e_receive_resp.json
  exit 1
fi
EVENT_PK="EVENT#${EVENT_ID}"

echo "[3/5] Polling DynamoDB for ATTEMPT#1 and status=${EXPECTED_STATUS}"
deadline=$(( $(date +%s) + POLL_TIMEOUT_SECS ))
attempt_seen=false
final_status=""

while [[ "$(date +%s)" -lt "$deadline" ]]; do
  got_attempt_pk="$(aws dynamodb get-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$EVENTS_TABLE" \
    --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"ATTEMPT#1\"}}" \
    --query 'Item.pk.S' \
    --output text 2>/dev/null || true)"

  final_status="$(aws dynamodb get-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$EVENTS_TABLE" \
    --key "{\"pk\":{\"S\":\"${EVENT_PK}\"},\"sk\":{\"S\":\"v0\"}}" \
    --query 'Item.status.S' \
    --output text 2>/dev/null || true)"

  if [[ "$got_attempt_pk" == "$EVENT_PK" ]]; then
    attempt_seen=true
  fi

  if [[ "$attempt_seen" == "true" && "$final_status" == "$EXPECTED_STATUS" ]]; then
    break
  fi
  sleep "$POLL_INTERVAL_SECS"
done

echo "[4/5] Verifying assertions"
if [[ "$attempt_seen" != "true" ]]; then
  echo "ERROR: ATTEMPT#1 was not written for EVENT_ID=$EVENT_ID"
  exit 1
fi
if [[ "$final_status" != "$EXPECTED_STATUS" ]]; then
  echo "ERROR: expected status=$EXPECTED_STATUS, got status=$final_status for EVENT_ID=$EVENT_ID"
  exit 1
fi

echo "[5/5] SUCCESS"
echo "EVENT_ID=$EVENT_ID"
echo "CUSTOMER_ID=$CUSTOMER_ID"
echo "FINAL_STATUS=$final_status"
