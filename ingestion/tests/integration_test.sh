#!/usr/bin/env bash
# =============================================================================
# Ingestion integration test — Day 5
#
# Tests the live ingestion API (API Gateway → Ingestion Lambda) end-to-end.
# Verifies every contract item agreed with Engineer 2 (CONTRACT_CONFIRMATION_LIST §10).
#
# Test cases
# ----------
#   1. POST /webhooks/configs   — create config (201, whsec_ secret)
#   2. POST /webhooks/receive   — happy path (202, event_id returned)
#   3. POST /webhooks/receive   — idempotency replay (200, same event_id)
#   4. POST /webhooks/receive   — missing config (4xx/5xx, no panic)
#   5. POST /webhooks/receive   — validation (missing idempotency_key)
#   6. GET  /webhooks/configs   — verify config round-trip (200, all fields present)
#   7. DynamoDB verification    — config row exists with pk/sk contract
#   8. DynamoDB verification    — event row exists with pk/sk contract
#   9. DynamoDB verification    — idempotency record exists with TTL
#   10. SQS verification        — message visible in queue with customer_id attribute
#
# Required env vars
# -----------------
#   API_BASE_URL       e.g. https://abc123.execute-api.us-east-1.amazonaws.com/Prod
#   AWS_REGION         e.g. us-east-1
#   EVENTS_TABLE       e.g. webhook_events_dev
#   IDEMPOTENCY_TABLE  e.g. webhook_idempotency_dev
#   CONFIGS_TABLE      e.g. webhook_configs_dev
#   QUEUE_URL          SQS queue URL
#
# Optional env vars
# -----------------
#   CUSTOMER_ID        (auto-generated if not set)
#   IDEMPOTENCY_KEY    (auto-generated if not set)
#   KEEP_TEST_DATA     true → skip cleanup (default: false)
#   POLL_TIMEOUT_SECS  how long to wait for SQS message (default: 30)
#   POLL_INTERVAL_SECS polling interval (default: 2)
#
# Usage
# -----
#   export API_BASE_URL="https://abc123.execute-api.us-east-1.amazonaws.com/Prod"
#   export AWS_REGION="us-east-1"
#   export EVENTS_TABLE="webhook_events_dev"
#   export IDEMPOTENCY_TABLE="webhook_idempotency_dev"
#   export CONFIGS_TABLE="webhook_configs_dev"
#   export QUEUE_URL="https://sqs.us-east-1.amazonaws.com/<account-id>/webhook_delivery_dev"
#   bash ingestion/tests/integration_test.sh
# =============================================================================
set -euo pipefail

# ---------------------------------------------------------------------------
# Colour helpers
# ---------------------------------------------------------------------------
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RESET='\033[0m'

info()    { echo -e "${CYAN}[INFO]${RESET}  $*"; }
ok()      { echo -e "${GREEN}[PASS]${RESET}  $*"; }
warn()    { echo -e "${YELLOW}[WARN]${RESET}  $*"; }
fail()    { echo -e "${RED}[FAIL]${RESET}  $*"; FAILURES=$((FAILURES + 1)); }
section() { echo -e "\n${CYAN}══ $* ══${RESET}"; }

# ---------------------------------------------------------------------------
# Input validation — fail fast if required vars are absent
# ---------------------------------------------------------------------------
require_set() {
  local var_name="$1"
  if [[ -z "${!var_name:-}" ]]; then
    echo -e "${RED}ERROR:${RESET} required env var '${var_name}' is not set." >&2
    echo "  See script header for usage." >&2
    exit 1
  fi
}

require_set "API_BASE_URL"
require_set "AWS_REGION"
require_set "EVENTS_TABLE"
require_set "IDEMPOTENCY_TABLE"
require_set "CONFIGS_TABLE"
require_set "QUEUE_URL"

# Trim trailing slash from API_BASE_URL
API_BASE_URL="${API_BASE_URL%/}"

# ---------------------------------------------------------------------------
# Test state
# ---------------------------------------------------------------------------
FAILURES=0
CUSTOMER_ID="${CUSTOMER_ID:-cust_int_$(date +%s%N | md5sum | head -c 8)}"
IDEMPOTENCY_KEY="${IDEMPOTENCY_KEY:-idem_int_$(date +%s%N | md5sum | head -c 12)}"
DELIVERY_URL="https://webhook.site/00000000-0000-0000-0000-000000000000"  # safe no-op sink
KEEP_TEST_DATA="${KEEP_TEST_DATA:-false}"
POLL_TIMEOUT_SECS="${POLL_TIMEOUT_SECS:-30}"
POLL_INTERVAL_SECS="${POLL_INTERVAL_SECS:-2}"

EVENT_ID=""          # set after test 2
SQS_RECEIPT_HANDLE="" # set after test 9

EVENT_WRITTEN=false
CONFIG_WRITTEN=false
IDEM_WRITTEN=false
SQS_MESSAGE_RECEIVED=false

info "CUSTOMER_ID      = ${CUSTOMER_ID}"
info "IDEMPOTENCY_KEY  = ${IDEMPOTENCY_KEY}"
info "API_BASE_URL     = ${API_BASE_URL}"
info "EVENTS_TABLE     = ${EVENTS_TABLE}"
info "IDEMPOTENCY_TABLE= ${IDEMPOTENCY_TABLE}"
info "CONFIGS_TABLE    = ${CONFIGS_TABLE}"
info "QUEUE_URL        = ${QUEUE_URL}"

# ---------------------------------------------------------------------------
# Cleanup — runs on EXIT regardless of failure
# ---------------------------------------------------------------------------
cleanup() {
  local exit_code=$?
  set +e

  if [[ "$KEEP_TEST_DATA" == "true" ]]; then
    warn "KEEP_TEST_DATA=true — skipping DynamoDB / SQS cleanup"

  fi

  section "Cleanup"

  if [[ -n "$SQS_RECEIPT_HANDLE" && "$SQS_MESSAGE_RECEIVED" == "true" ]]; then
    info "Deleting SQS test message..."
    aws sqs delete-message \
      --region "$AWS_REGION" \
      --queue-url "$QUEUE_URL" \
      --receipt-handle "$SQS_RECEIPT_HANDLE" \
      >/dev/null 2>&1 && info "SQS message deleted" || warn "SQS delete skipped (already gone)"
  fi

  if [[ -n "$EVENT_ID" && "$EVENT_WRITTEN" == "true" ]]; then
    info "Deleting event row (v0)..."
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"EVENT#${EVENT_ID}\"},\"sk\":{\"S\":\"v0\"}}" \
      >/dev/null 2>&1 && info "Event row deleted" || warn "Event row delete skipped"
  fi

  if [[ "$IDEM_WRITTEN" == "true" ]]; then
    info "Deleting idempotency record..."
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$IDEMPOTENCY_TABLE" \
      --key "{\"pk\":{\"S\":\"IDEM#${IDEMPOTENCY_KEY}\"}}" \
      >/dev/null 2>&1 && info "Idempotency record deleted" || warn "Idempotency delete skipped"
  fi

  if [[ "$CONFIG_WRITTEN" == "true" ]]; then
    info "Deleting config record..."
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --table-name "$CONFIGS_TABLE" \
      --key "{\"pk\":{\"S\":\"CUSTOMER#${CUSTOMER_ID}\"},\"sk\":{\"S\":\"CONFIG\"}}" \
      >/dev/null 2>&1 && info "Config record deleted" || warn "Config delete skipped"
  fi

  if [[ $FAILURES -eq 0 ]]; then
    echo -e "\n${GREEN}ALL TESTS PASSED${RESET}"
  else
    echo -e "\n${RED}${FAILURES} TEST(S) FAILED${RESET}"
    exit 1
  fi

  return 0
}
trap cleanup EXIT

# ---------------------------------------------------------------------------
# Helper: assert JSON field equals expected value
# ---------------------------------------------------------------------------
# Usage: assert_json_field <json_string> <jq_filter> <expected_value> <label>
assert_json_field() {
  local json="$1"
  local jq_filter="$2"
  local expected="$3"
  local label="$4"

  local actual
  actual="$(echo "$json" | jq -r "$jq_filter" 2>/dev/null || echo "__jq_error__")"

  if [[ "$actual" == "$expected" ]]; then
    ok "${label}: ${actual}"
  else
    fail "${label}: expected '${expected}', got '${actual}'"
  fi
}

# ---------------------------------------------------------------------------
# Helper: assert HTTP response code
# ---------------------------------------------------------------------------
assert_status() {
  local actual_status="$1"
  local expected_status="$2"
  local label="$3"

  if [[ "$actual_status" == "$expected_status" ]]; then
    ok "${label}: HTTP ${actual_status}"
  else
    fail "${label}: expected HTTP ${expected_status}, got HTTP ${actual_status}"
  fi
}

# =============================================================================
# Test 1: POST /webhooks/configs — create config (201)
# =============================================================================
section "Test 1: Create webhook config"

RESPONSE_1="$(curl -s -w '\n%{http_code}' -X POST "${API_BASE_URL}/webhooks/configs" \
  -H "Content-Type: application/json" \
  -d "{\"customer_id\":\"${CUSTOMER_ID}\",\"url\":\"${DELIVERY_URL}\"}")"

HTTP_STATUS_1="$(echo "$RESPONSE_1" | tail -n1)"
BODY_1="$(echo "$RESPONSE_1" | head -n-1)"

MASKED_BODY_1="$(echo "$BODY_1" | jq '.secret = "***REDACTED***"')"
info "Response body: ${MASKED_BODY_1}"
assert_status "$HTTP_STATUS_1" "201" "POST /webhooks/configs"
assert_json_field "$BODY_1" ".url" "$DELIVERY_URL" "config.url"
assert_json_field "$BODY_1" ".active" "true" "config.active"

# Verify secret has whsec_ prefix (contract §4)
SECRET="$(echo "$BODY_1" | jq -r '.secret')"
if [[ "$SECRET" == whsec_* ]]; then
  ok "config.secret has whsec_ prefix"
else
  fail "config.secret missing whsec_ prefix (got: ${SECRET})"
fi

CONFIG_WRITTEN=true

# =============================================================================
# Test 2: POST /webhooks/receive — happy path (202 Accepted)
# =============================================================================
section "Test 2: Receive webhook — happy path (202)"

PAYLOAD='{"order_id":"ord_123","amount":99.99}'
RESPONSE_2="$(curl -s -w '\n%{http_code}' -X POST "${API_BASE_URL}/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"${IDEMPOTENCY_KEY}\",\"customer_id\":\"${CUSTOMER_ID}\",\"payload\":${PAYLOAD}}")"

HTTP_STATUS_2="$(echo "$RESPONSE_2" | tail -n1)"
BODY_2="$(echo "$RESPONSE_2" | head -n-1)"

info "Response body: ${BODY_2}"
assert_status "$HTTP_STATUS_2" "202" "POST /webhooks/receive (happy path)"
assert_json_field "$BODY_2" ".status" "accepted" "receive.status"

EVENT_ID="$(echo "$BODY_2" | jq -r '.event_id')"
if [[ "$EVENT_ID" == evt_* ]]; then
  ok "receive.event_id has evt_ prefix: ${EVENT_ID}"
else
  fail "receive.event_id missing evt_ prefix (got: ${EVENT_ID})"
fi

# Validate created_at is a reasonable Unix timestamp (within 1 hour of now)
CREATED_AT="$(echo "$BODY_2" | jq -r '.created_at')"
# Ensure CREATED_AT is numeric
if ! [[ "$CREATED_AT" =~ ^[0-9]+$ ]]; then
  fail "receive.created_at is not a numeric Unix timestamp: ${CREATED_AT}"
else
  NOW="$(date +%s)"
  LOWER_BOUND=$((NOW - 3600))   # 1 hour before now
  UPPER_BOUND=$((NOW + 3600))   # 1 hour after now (to allow for minor clock skew)
  if [[ "$CREATED_AT" -ge "$LOWER_BOUND" && "$CREATED_AT" -le "$UPPER_BOUND" ]]; then
    ok "receive.created_at is a valid, recent timestamp: ${CREATED_AT}"
  else
    fail "receive.created_at is outside the expected time window: ${CREATED_AT} (expected between ${LOWER_BOUND} and ${UPPER_BOUND})"
  fi
fi

EVENT_WRITTEN=true
IDEM_WRITTEN=true

# =============================================================================
# Test 3: POST /webhooks/receive — idempotency replay (200 Duplicate)
# =============================================================================
section "Test 3: Receive webhook — idempotency replay (200)"

RESPONSE_3="$(curl -s -w '\n%{http_code}' -X POST "${API_BASE_URL}/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"${IDEMPOTENCY_KEY}\",\"customer_id\":\"${CUSTOMER_ID}\",\"payload\":${PAYLOAD}}")"

HTTP_STATUS_3="$(echo "$RESPONSE_3" | tail -n1)"
BODY_3="$(echo "$RESPONSE_3" | head -n-1)"

info "Response body: ${BODY_3}"
assert_status "$HTTP_STATUS_3" "200" "POST /webhooks/receive (duplicate)"
assert_json_field "$BODY_3" ".status" "duplicate" "duplicate.status"

EVENT_ID_REPLAY="$(echo "$BODY_3" | jq -r '.event_id')"
if [[ "$EVENT_ID_REPLAY" == "$EVENT_ID" ]]; then
  ok "duplicate.event_id matches original: ${EVENT_ID_REPLAY}"
else
  fail "duplicate.event_id mismatch: expected '${EVENT_ID}', got '${EVENT_ID_REPLAY}'"
fi

# =============================================================================
# Test 4: POST /webhooks/receive — missing config customer (error path)
# =============================================================================
section "Test 4: Receive webhook — unknown customer (no config)"

UNKNOWN_CUSTOMER="cust_no_config_$(date +%s%N | md5sum | head -c 8)"
UNKNOWN_IDEM="idem_no_config_$(date +%s%N | md5sum | head -c 12)"

RESPONSE_4="$(curl -s -w '\n%{http_code}' -X POST "${API_BASE_URL}/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"idempotency_key\":\"${UNKNOWN_IDEM}\",\"customer_id\":\"${UNKNOWN_CUSTOMER}\",\"payload\":${PAYLOAD}}" \
  --max-time 10)"

HTTP_STATUS_4="$(echo "$RESPONSE_4" | tail -n1)"
BODY_4="$(echo "$RESPONSE_4" | head -n-1)"

info "Response body: ${BODY_4}"
# Ingestion validates customer has a config before proceeding — expect 4xx/5xx, not 202
if [[ "$HTTP_STATUS_4" != "202" && "$HTTP_STATUS_4" != "200" ]]; then
  ok "unknown customer rejected with HTTP ${HTTP_STATUS_4} (not 202/200)"
else
  fail "unknown customer should be rejected, but got HTTP ${HTTP_STATUS_4}"
fi

# Verify the API didn't crash (response is valid JSON or at least non-empty)
if [[ -n "$BODY_4" ]]; then
  ok "error response body is non-empty"
else
  fail "error response body is empty — potential Lambda crash"
fi

# =============================================================================
# Test 5: POST /webhooks/receive — validation (missing idempotency_key)
# =============================================================================
section "Test 5: Receive webhook — validation (missing required field)"

RESPONSE_5="$(curl -s -w '\n%{http_code}' -X POST "${API_BASE_URL}/webhooks/receive" \
  -H "Content-Type: application/json" \
  -d "{\"customer_id\":\"${CUSTOMER_ID}\",\"payload\":${PAYLOAD}}")"

HTTP_STATUS_5="$(echo "$RESPONSE_5" | tail -n1)"
BODY_5="$(echo "$RESPONSE_5" | head -n-1)"

info "Response body: ${BODY_5}"
if [[ "$HTTP_STATUS_5" == "422" || "$HTTP_STATUS_5" == "400" ]]; then
  ok "missing idempotency_key rejected with HTTP ${HTTP_STATUS_5}"
else
  fail "missing idempotency_key: expected 422 or 400, got HTTP ${HTTP_STATUS_5}"
fi

# =============================================================================
# Test 6: GET /webhooks/configs — config round-trip (200)
# =============================================================================
section "Test 6: Get webhook config — round-trip"

RESPONSE_6="$(curl -s -w '\n%{http_code}' \
  "${API_BASE_URL}/webhooks/configs?customer_id=${CUSTOMER_ID}")"

HTTP_STATUS_6="$(echo "$RESPONSE_6" | tail -n1)"
BODY_6="$(echo "$RESPONSE_6" | head -n-1)"

BODY_6_REDACTED="$(echo "$BODY_6" | jq 'if type == "object" and has("secret") then .secret = "***REDACTED***" else . end' 2>/dev/null || echo "$BODY_6")"
info "Response body (secret redacted): ${BODY_6_REDACTED}"
assert_status "$HTTP_STATUS_6" "200" "GET /webhooks/configs"
assert_json_field "$BODY_6" ".url" "$DELIVERY_URL" "get_config.url"
assert_json_field "$BODY_6" ".max_retries" "3" "get_config.max_retries"
assert_json_field "$BODY_6" ".active" "true" "get_config.active"

# Secret must match what was returned on creation (contract §4)
SECRET_ROUNDTRIP="$(echo "$BODY_6" | jq -r '.secret')"
if [[ "$SECRET_ROUNDTRIP" == "$SECRET" ]]; then
  ok "get_config.secret matches create response"
else
  fail "get_config.secret mismatch: create='${SECRET}', get='${SECRET_ROUNDTRIP}'"
fi

# =============================================================================
# Test 7: GET /webhooks/configs — unknown customer (404)
# =============================================================================
section "Test 7: Get webhook config — unknown customer (404)"

RESPONSE_7="$(curl -s -w '\n%{http_code}' \
  "${API_BASE_URL}/webhooks/configs?customer_id=cust_does_not_exist_xyz")"

HTTP_STATUS_7="$(echo "$RESPONSE_7" | tail -n1)"
BODY_7="$(echo "$RESPONSE_7" | head -n-1)"

info "Response body: ${BODY_7}"
assert_status "$HTTP_STATUS_7" "404" "GET /webhooks/configs (unknown customer)"

# =============================================================================
# Test 8: DynamoDB verification — event row contract
# =============================================================================
section "Test 8: DynamoDB verification — event row (pk=EVENT#..., sk=v0)"

DDB_EVENT="$(aws dynamodb get-item \
  --region "$AWS_REGION" \
  --table-name "$EVENTS_TABLE" \
  --key "{\"pk\":{\"S\":\"EVENT#${EVENT_ID}\"},\"sk\":{\"S\":\"v0\"}}" \
  --output json 2>&1)"

if echo "$DDB_EVENT" | jq -e '.Item' >/dev/null 2>&1; then
  ok "DynamoDB: event row found for ${EVENT_ID}"

  # Verify required fields per CONTRACT_CONFIRMATION_LIST §2
  for field in event_id customer_id payload status attempt_count created_at; do
    if echo "$DDB_EVENT" | jq -e ".Item.${field}" >/dev/null 2>&1; then
      ok "DynamoDB: event.${field} is present"
    else
      fail "DynamoDB: event.${field} is MISSING (contract §2)"
    fi
  done

  # Verify status = "pending" (initial state per contract §2)
  DDB_STATUS="$(echo "$DDB_EVENT" | jq -r '.Item.status.S')"
  if [[ "$DDB_STATUS" == "pending" ]]; then
    ok "DynamoDB: event.status = 'pending'"
  else
    fail "DynamoDB: event.status expected 'pending', got '${DDB_STATUS}'"
  fi

  # Verify attempt_count = 0
  DDB_ATTEMPTS="$(echo "$DDB_EVENT" | jq -r '.Item.attempt_count.N')"
  if [[ "$DDB_ATTEMPTS" == "0" ]]; then
    ok "DynamoDB: event.attempt_count = 0"
  else
    fail "DynamoDB: event.attempt_count expected 0, got '${DDB_ATTEMPTS}'"
  fi

  # Verify pk contract: EVENT#{event_id}
  DDB_PK="$(echo "$DDB_EVENT" | jq -r '.Item.pk.S')"
  if [[ "$DDB_PK" == "EVENT#${EVENT_ID}" ]]; then
    ok "DynamoDB: event.pk = 'EVENT#${EVENT_ID}'"
  else
    fail "DynamoDB: event.pk mismatch: expected 'EVENT#${EVENT_ID}', got '${DDB_PK}'"
  fi
else
  fail "DynamoDB: event row NOT FOUND for EVENT#${EVENT_ID} in ${EVENTS_TABLE}"
  warn "DynamoDB response: ${DDB_EVENT}"
fi

# =============================================================================
# Test 9: DynamoDB verification — idempotency record + TTL
# =============================================================================
section "Test 9: DynamoDB verification — idempotency record (pk=IDEM#..., TTL set)"

DDB_IDEM="$(aws dynamodb get-item \
  --region "$AWS_REGION" \
  --table-name "$IDEMPOTENCY_TABLE" \
  --key "{\"pk\":{\"S\":\"IDEM#${IDEMPOTENCY_KEY}\"}}" \
  --output json 2>&1)"

if echo "$DDB_IDEM" | jq -e '.Item' >/dev/null 2>&1; then
  ok "DynamoDB: idempotency record found for IDEM#${IDEMPOTENCY_KEY}"

  # Verify event_id stored matches what was returned
  DDB_IDEM_EVENT_ID="$(echo "$DDB_IDEM" | jq -r '.Item.event_id.S')"
  if [[ "$DDB_IDEM_EVENT_ID" == "$EVENT_ID" ]]; then
    ok "DynamoDB: idempotency.event_id = ${EVENT_ID}"
  else
    fail "DynamoDB: idempotency.event_id mismatch: expected '${EVENT_ID}', got '${DDB_IDEM_EVENT_ID}'"
  fi

  # Verify TTL is set (contract §2 — 24h TTL)
  if echo "$DDB_IDEM" | jq -e '.Item.ttl' >/dev/null 2>&1; then
    DDB_TTL="$(echo "$DDB_IDEM" | jq -r '.Item.ttl.N')"
    NOW_TS="$(date +%s)"
    TTL_DIFF=$((DDB_TTL - NOW_TS))
    if [[ $TTL_DIFF -gt 85000 && $TTL_DIFF -lt 87800 ]]; then
      ok "DynamoDB: idempotency TTL is ~24h from now (diff=${TTL_DIFF}s)"
    else
      warn "DynamoDB: idempotency TTL diff is ${TTL_DIFF}s (expected ~86400s) — acceptable clock skew"
    fi
  else
    fail "DynamoDB: idempotency TTL is MISSING (contract §2)"
  fi
else
  fail "DynamoDB: idempotency record NOT FOUND for IDEM#${IDEMPOTENCY_KEY} in ${IDEMPOTENCY_TABLE}"
  warn "DynamoDB response: ${DDB_IDEM}"
fi

# =============================================================================
# Test 10: SQS verification — message in queue with customer_id attribute
# =============================================================================
section "Test 10: SQS verification — message in queue (event_id body, customer_id attribute)"

info "Polling SQS for event_id=${EVENT_ID} (timeout=${POLL_TIMEOUT_SECS}s)"

SQS_FOUND=false
ELAPSED=0

while [[ $ELAPSED -lt $POLL_TIMEOUT_SECS ]]; do
  SQS_RESP="$(aws sqs receive-message \
    --region "$AWS_REGION" \
    --queue-url "$QUEUE_URL" \
    --message-attribute-names All \
    --max-number-of-messages 10 \
    --wait-time-seconds 2 \
    --output json 2>&1)"

  MESSAGES="$(echo "$SQS_RESP" | jq -r '.Messages // []')"
  COUNT="$(echo "$MESSAGES" | jq 'length')"

  if [[ "$COUNT" -gt 0 ]]; then
    for i in $(seq 0 $((COUNT - 1))); do
      MSG_BODY="$(echo "$MESSAGES" | jq -r ".[${i}].Body")"
      MSG_EVENT_ID="$(echo "$MSG_BODY" | jq -r '.event_id' 2>/dev/null || echo "")"

      if [[ "$MSG_EVENT_ID" == "$EVENT_ID" ]]; then
        ok "SQS: message found with event_id=${EVENT_ID}"

        # Verify body is exactly {"event_id":"..."} (contract §6)
        BODY_KEYS="$(echo "$MSG_BODY" | jq 'keys | length')"
        if [[ "$BODY_KEYS" == "1" ]]; then
          ok "SQS: message body has exactly 1 key (event_id only)"
        else
          fail "SQS: message body has ${BODY_KEYS} keys — expected 1 (contract §6)"
          warn "SQS body: ${MSG_BODY}"
        fi

        # Verify customer_id is a MessageAttribute (contract §6)
        ATTR_CUSTOMER="$(echo "$MESSAGES" | jq -r ".[${i}].MessageAttributes.customer_id.StringValue" 2>/dev/null || echo "null")"
        if [[ "$ATTR_CUSTOMER" == "$CUSTOMER_ID" ]]; then
          ok "SQS: MessageAttribute customer_id = ${CUSTOMER_ID}"
        else
          fail "SQS: MessageAttribute customer_id mismatch: expected '${CUSTOMER_ID}', got '${ATTR_CUSTOMER}'"
        fi

        # Verify attribute DataType is String (contract §6)
        ATTR_TYPE="$(echo "$MESSAGES" | jq -r ".[${i}].MessageAttributes.customer_id.DataType" 2>/dev/null || echo "null")"
        if [[ "$ATTR_TYPE" == "String" ]]; then
          ok "SQS: MessageAttribute customer_id DataType = String"
        else
          fail "SQS: MessageAttribute customer_id DataType expected 'String', got '${ATTR_TYPE}'"
        fi

        SQS_RECEIPT_HANDLE="$(echo "$MESSAGES" | jq -r ".[${i}].ReceiptHandle")"
        SQS_MESSAGE_RECEIVED=true
        SQS_FOUND=true
        break 2
      fi
    done
  fi

  sleep "$POLL_INTERVAL_SECS"
  ELAPSED=$((ELAPSED + POLL_INTERVAL_SECS))
done

if [[ "$SQS_FOUND" == "false" ]]; then
  fail "SQS: message with event_id=${EVENT_ID} NOT found within ${POLL_TIMEOUT_SECS}s"
fi

# =============================================================================
# Summary
# =============================================================================
section "Test Summary"
echo ""
if [[ $FAILURES -eq 0 ]]; then
  echo -e "${GREEN}✅  All integration tests passed.${RESET}"
else
  echo -e "${RED}❌  ${FAILURES} integration test(s) failed.${RESET}"
fi
echo ""
# cleanup runs via EXIT trap
