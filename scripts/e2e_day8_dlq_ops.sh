#!/usr/bin/env bash
set -euo pipefail

# Day 8 scenario test suite: error handling + DLQ ops.
#
# Scenarios:
# 1) Endpoint outage (5xx) -> message should transition to DLQ (long-running)
# 2) Permanent 4xx -> event should transition to failed
# 3) Missing config -> event should transition to failed
# 4) Disabled config -> event should transition to failed
# 5) DLQ replay workflow -> dry-run + real replay via scripts/dlq_ops.sh
#
# Defaults:
# - AWS_REGION=us-west-2
# - AWS_PROFILE=hooray-dev
# - STACK_NAME=hooray-dev
# - POLL_TIMEOUT_SECS=240
# - POLL_INTERVAL_SECS=5
# - DLQ_WAIT_TIMEOUT_SECS=480
# - DLQ_WAIT_INTERVAL_SECS=10
# - RUN_LONG_DLQ_SCENARIO=true
# - RUN_REPLAY_VALIDATION=true
# - DELETE_AFTER_REPLAY=false
# - KEEP_TEST_DATA=false

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

require_cmd aws
require_cmd jq
require_cmd curl
require_cmd bash

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_PROFILE="${AWS_PROFILE:-hooray-dev}"
STACK_NAME="${STACK_NAME:-hooray-dev}"
POLL_TIMEOUT_SECS="${POLL_TIMEOUT_SECS:-240}"
POLL_INTERVAL_SECS="${POLL_INTERVAL_SECS:-5}"
DLQ_WAIT_TIMEOUT_SECS="${DLQ_WAIT_TIMEOUT_SECS:-480}"
DLQ_WAIT_INTERVAL_SECS="${DLQ_WAIT_INTERVAL_SECS:-10}"
RUN_LONG_DLQ_SCENARIO="${RUN_LONG_DLQ_SCENARIO:-true}"
RUN_REPLAY_VALIDATION="${RUN_REPLAY_VALIDATION:-true}"
DELETE_AFTER_REPLAY="${DELETE_AFTER_REPLAY:-false}"
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
QUEUE_URL="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="QueueUrl") | .OutputValue')"
DLQ_URL="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="DLQUrl") | .OutputValue')"

if [[ -z "$API_URL" || "$API_URL" == "null" || -z "$EVENTS_TABLE" || "$EVENTS_TABLE" == "null" || -z "$CONFIGS_TABLE" || "$CONFIGS_TABLE" == "null" || -z "$QUEUE_URL" || "$QUEUE_URL" == "null" || -z "$DLQ_URL" || "$DLQ_URL" == "null" ]]; then
  echo "ERROR: missing required stack outputs (API_URL/EVENTS_TABLE/CONFIGS_TABLE/QUEUE_URL/DLQ_URL)" >&2
  exit 1
fi

if [[ -z "$IDEMPOTENCY_TABLE" || "$IDEMPOTENCY_TABLE" == "null" ]]; then
  echo "WARN: IdempotencyTableName output missing; cleanup for idempotency keys will be skipped"
fi

declare -a CLEANUP_EVENT_IDS=()
declare -a CLEANUP_CUSTOMER_IDS=()
declare -a CLEANUP_IDEMPOTENCY_KEYS=()

cleanup() {
  local exit_code=$?
  set +e

  if [[ "$KEEP_TEST_DATA" == "true" ]]; then
    echo "[CLEANUP] KEEP_TEST_DATA=true, skipping cleanup"
    return "$exit_code"
  fi

  for event_id in "${CLEANUP_EVENT_IDS[@]}"; do
    local event_pk="EVENT#${event_id}"
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
  done

  for customer_id in "${CLEANUP_CUSTOMER_IDS[@]}"; do
    aws dynamodb delete-item \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --table-name "$CONFIGS_TABLE" \
      --key "{\"pk\":{\"S\":\"CUSTOMER#${customer_id}\"},\"sk\":{\"S\":\"CONFIG\"}}" >/dev/null 2>&1 || true
  done

  if [[ -n "$IDEMPOTENCY_TABLE" && "$IDEMPOTENCY_TABLE" != "null" ]]; then
    for idem_key in "${CLEANUP_IDEMPOTENCY_KEYS[@]}"; do
      aws dynamodb delete-item \
        --region "$AWS_REGION" \
        --profile "$AWS_PROFILE" \
        --table-name "$IDEMPOTENCY_TABLE" \
        --key "{\"pk\":{\"S\":\"IDEM#${idem_key}\"}}" >/dev/null 2>&1 || true
    done
  fi

  return "$exit_code"
}

trap cleanup EXIT INT TERM

ts_now() {
  date +%s
}

unique_suffix() {
  printf '%s_%s' "$(date +%s)" "$(date +%s%N | tail -c 7)"
}

add_cleanup_event() {
  CLEANUP_EVENT_IDS+=("$1")
}

add_cleanup_customer() {
  CLEANUP_CUSTOMER_IDS+=("$1")
}

add_cleanup_idem() {
  CLEANUP_IDEMPOTENCY_KEYS+=("$1")
}

create_config_via_api() {
  local customer_id="$1"
  local delivery_url="$2"
  local delivery_secret="$3"

  local code
  code="$(curl -sS -o /tmp/day8_config_resp.json -w "%{http_code}" \
    -X POST "${API_URL}webhooks/configs" \
    -H 'content-type: application/json' \
    -d "{\"customer_id\":\"${customer_id}\",\"url\":\"${delivery_url}\",\"secret\":\"${delivery_secret}\"}")"

  if [[ "$code" != "200" && "$code" != "201" ]]; then
    echo "ERROR: failed to create config (status=${code}) for ${customer_id}" >&2
    cat /tmp/day8_config_resp.json >&2
    exit 1
  fi
}

send_event_via_api() {
  local customer_id="$1"
  local idempotency_key="$2"
  local payload_source="$3"

  local code
  code="$(curl -sS -o /tmp/day8_receive_resp.json -w "%{http_code}" \
    -X POST "${API_URL}webhooks/receive" \
    -H 'content-type: application/json' \
    -d "{\"idempotency_key\":\"${idempotency_key}\",\"customer_id\":\"${customer_id}\",\"data\":{\"source\":\"${payload_source}\"}}")"

  if [[ "$code" != "200" && "$code" != "202" ]]; then
    echo "ERROR: failed to submit event (status=${code}) for customer=${customer_id}" >&2
    cat /tmp/day8_receive_resp.json >&2
    exit 1
  fi

  local event_id
  event_id="$(jq -r '.event_id // empty' /tmp/day8_receive_resp.json)"
  if [[ -z "$event_id" || "$event_id" == "null" ]]; then
    echo "ERROR: missing event_id in ingestion response" >&2
    cat /tmp/day8_receive_resp.json >&2
    exit 1
  fi

  echo "$event_id"
}

seed_event_and_enqueue() {
  local event_id="$1"
  local customer_id="$2"
  local payload_raw="$3"

  local now
  now="$(ts_now)"
  local item_json
  item_json="$(jq -cn \
    --arg event_id "$event_id" \
    --arg customer_id "$customer_id" \
    --arg payload "$payload_raw" \
    --arg now "$now" \
    '{
      pk:{S:("EVENT#" + $event_id)},
      sk:{S:"v0"},
      event_id:{S:$event_id},
      customer_id:{S:$customer_id},
      payload:{S:$payload},
      status:{S:"pending"},
      attempt_count:{N:"0"},
      created_at:{N:$now}
    }'
  )"

  aws dynamodb put-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$EVENTS_TABLE" \
    --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
    --item "$item_json" >/dev/null

  aws sqs send-message \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --queue-url "$QUEUE_URL" \
    --message-body "{\"event_id\":\"${event_id}\"}" \
    --message-attributes "{\"customer_id\":{\"DataType\":\"String\",\"StringValue\":\"${customer_id}\"}}" >/dev/null
}

wait_for_event_status() {
  local event_id="$1"
  local expected_status="$2"
  local timeout_secs="$3"

  local event_pk="EVENT#${event_id}"
  local deadline=$(( $(ts_now) + timeout_secs ))
  local status=""

  while [[ "$(ts_now)" -lt "$deadline" ]]; do
    status="$(aws dynamodb get-item \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --table-name "$EVENTS_TABLE" \
      --key "{\"pk\":{\"S\":\"${event_pk}\"},\"sk\":{\"S\":\"v0\"}}" \
      --query 'Item.status.S' \
      --output text 2>/dev/null || true)"

    if [[ "$status" == "$expected_status" ]]; then
      echo "$status"
      return 0
    fi
    sleep "$POLL_INTERVAL_SECS"
  done

  echo "$status"
  return 1
}

wait_for_dlq_event_id() {
  local event_id="$1"
  local timeout_secs="$2"

  local deadline=$(( $(ts_now) + timeout_secs ))
  while [[ "$(ts_now)" -lt "$deadline" ]]; do
    local msgs_json
    msgs_json="$(aws sqs receive-message \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --queue-url "$DLQ_URL" \
      --max-number-of-messages 10 \
      --wait-time-seconds 2 \
      --visibility-timeout 1 \
      --attribute-names All \
      --message-attribute-names All \
      --output json 2>/dev/null || true)"

    local found
    found="$(echo "$msgs_json" | jq -r --arg event_id "$event_id" '
      (.Messages // [])
      | map(.Body // "")
      | map((fromjson? // {}) | .event_id // empty)
      | any(. == $event_id)
    ' 2>/dev/null || echo false)"

    if [[ "$found" == "true" ]]; then
      return 0
    fi

    sleep "$DLQ_WAIT_INTERVAL_SECS"
  done

  return 1
}

echo "[1/5] Scenario: permanent 4xx -> terminal failed"
{
  scenario_suffix="$(unique_suffix)"
  customer_id="cust_day8_4xx_${scenario_suffix}"
  idem_key="req_day8_4xx_${scenario_suffix}"
  create_config_via_api "$customer_id" "https://httpbin.org/status/404" "whsec_day8_4xx"
  add_cleanup_customer "$customer_id"
  add_cleanup_idem "$idem_key"

  event_id="$(send_event_via_api "$customer_id" "$idem_key" "day8-permanent-4xx")"
  add_cleanup_event "$event_id"

  if wait_for_event_status "$event_id" "failed" "$POLL_TIMEOUT_SECS" >/dev/null; then
    echo "  - PASS: event ${event_id} transitioned to failed"
  else
    echo "ERROR: event ${event_id} did not transition to failed in time" >&2
    exit 1
  fi
}

echo "[2/5] Scenario: missing config -> terminal failed"
{
  scenario_suffix="$(unique_suffix)"
  event_id="evt_day8_missing_cfg_${scenario_suffix}"
  customer_id="cust_day8_missing_cfg_${scenario_suffix}"
  payload_raw='{"source":"day8-missing-config"}'

  seed_event_and_enqueue "$event_id" "$customer_id" "$payload_raw"
  add_cleanup_event "$event_id"

  if wait_for_event_status "$event_id" "failed" "$POLL_TIMEOUT_SECS" >/dev/null; then
    echo "  - PASS: missing config event ${event_id} transitioned to failed"
  else
    echo "ERROR: missing config event ${event_id} did not transition to failed in time" >&2
    exit 1
  fi
}

echo "[3/5] Scenario: disabled config -> terminal failed"
{
  scenario_suffix="$(unique_suffix)"
  event_id="evt_day8_disabled_cfg_${scenario_suffix}"
  customer_id="cust_day8_disabled_cfg_${scenario_suffix}"
  now="$(ts_now)"
  payload_raw='{"source":"day8-disabled-config"}'

  aws dynamodb put-item \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    --table-name "$CONFIGS_TABLE" \
    --condition-expression "attribute_not_exists(pk) AND attribute_not_exists(sk)" \
    --item "{
      \"pk\":{\"S\":\"CUSTOMER#${customer_id}\"},
      \"sk\":{\"S\":\"CONFIG\"},
      \"customer_id\":{\"S\":\"${customer_id}\"},
      \"url\":{\"S\":\"https://httpbin.org/post\"},
      \"secret\":{\"S\":\"whsec_day8_disabled\"},
      \"max_retries\":{\"N\":\"3\"},
      \"active\":{\"BOOL\":false},
      \"created_at\":{\"N\":\"${now}\"},
      \"updated_at\":{\"N\":\"${now}\"}
    }" >/dev/null

  add_cleanup_customer "$customer_id"
  seed_event_and_enqueue "$event_id" "$customer_id" "$payload_raw"
  add_cleanup_event "$event_id"

  if wait_for_event_status "$event_id" "failed" "$POLL_TIMEOUT_SECS" >/dev/null; then
    echo "  - PASS: disabled config event ${event_id} transitioned to failed"
  else
    echo "ERROR: disabled config event ${event_id} did not transition to failed in time" >&2
    exit 1
  fi
}

if [[ "$RUN_LONG_DLQ_SCENARIO" == "true" ]]; then
  echo "[4/5] Scenario: outage 5xx -> transition to DLQ (long-running)"
  scenario_suffix="$(unique_suffix)"
  customer_id="cust_day8_dlq_${scenario_suffix}"
  idem_key="req_day8_dlq_${scenario_suffix}"
  create_config_via_api "$customer_id" "https://httpbin.org/status/503" "whsec_day8_dlq"
  add_cleanup_customer "$customer_id"
  add_cleanup_idem "$idem_key"

  outage_event_id="$(send_event_via_api "$customer_id" "$idem_key" "day8-outage-503")"
  add_cleanup_event "$outage_event_id"

  if wait_for_dlq_event_id "$outage_event_id" "$DLQ_WAIT_TIMEOUT_SECS"; then
    echo "  - PASS: outage event ${outage_event_id} observed in DLQ"
  else
    echo "ERROR: outage event ${outage_event_id} not observed in DLQ within timeout=${DLQ_WAIT_TIMEOUT_SECS}s" >&2
    exit 1
  fi
else
  echo "[4/5] Skipped long DLQ scenario (RUN_LONG_DLQ_SCENARIO=false)"
fi

if [[ "$RUN_REPLAY_VALIDATION" == "true" ]]; then
  echo "[5/5] Scenario: validate replay flow via dlq_ops.sh"
  replay_ids=""
  replay_match=false
  for attempt in 1 2 3 4 5; do
    inspect_out="$(MODE=inspect MAX_MESSAGES=10 VISIBILITY_TIMEOUT_SECS=0 AWS_REGION="$AWS_REGION" AWS_PROFILE="$AWS_PROFILE" STACK_NAME="$STACK_NAME" "$REPO_ROOT/scripts/dlq_ops.sh")"
    echo "$inspect_out"

    ids_from_inspect="$(echo "$inspect_out" | grep -Eo '[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}' | sort -u | paste -sd, -)"
    if [[ -z "$ids_from_inspect" ]]; then
      continue
    fi

    if [[ -z "$replay_ids" ]]; then
      replay_ids="$ids_from_inspect"
    else
      replay_ids="$(
        {
          echo "$replay_ids"
          echo "$ids_from_inspect"
        } | tr ',' '\n' | awk 'NF' | sort -u | paste -sd, -
      )"
    fi

    if MODE=replay MAX_MESSAGES=10 VISIBILITY_TIMEOUT_SECS=0 REPLAY_MESSAGE_IDS="$replay_ids" DRY_RUN=true \
      AWS_REGION="$AWS_REGION" AWS_PROFILE="$AWS_PROFILE" STACK_NAME="$STACK_NAME" \
      "$REPO_ROOT/scripts/dlq_ops.sh"; then
      replay_match=true
      break
    fi

    sleep 1
  done

  if [[ "$replay_match" != "true" ]]; then
    echo "ERROR: dry-run replay validation could not match a current receive batch after retries" >&2
    exit 1
  fi

  replay_sent=false
  for attempt in 1 2 3 4 5; do
    if MODE=replay MAX_MESSAGES=10 VISIBILITY_TIMEOUT_SECS=0 REPLAY_MESSAGE_IDS="$replay_ids" DRY_RUN=false DELETE_AFTER_REPLAY="$DELETE_AFTER_REPLAY" \
      AWS_REGION="$AWS_REGION" AWS_PROFILE="$AWS_PROFILE" STACK_NAME="$STACK_NAME" \
      "$REPO_ROOT/scripts/dlq_ops.sh"; then
      replay_sent=true
      break
    fi
    sleep 1
  done

  if [[ "$replay_sent" != "true" ]]; then
    echo "ERROR: real replay validation failed to match a current receive batch after retries" >&2
    exit 1
  fi

  echo "  - PASS: replay workflow validated (ids=${replay_ids})"
else
  echo "[5/5] Skipped replay validation (RUN_REPLAY_VALIDATION=false)"
fi

echo "SUCCESS: Day 8 scenario suite completed"
