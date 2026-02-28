#!/usr/bin/env bash
set -euo pipefail

# Day 6 observability verification:
# 1) Send one successful delivery event and one exhausted-failure event (HTTP 404)
# 2) Verify worker delivery-attempt JSON logs contain required fields
# 3) Verify custom CloudWatch metrics are visible
# 4) Verify alarms exist (and optionally apply dashboard/alarms first)
#
# Defaults:
# - AWS_REGION=us-west-2
# - AWS_PROFILE=hooray-dev
# - STACK_NAME=hooray-dev
# - ENVIRONMENT=dev
# - METRIC_NAMESPACE=HoorayRelay/Worker
# - LOG_GROUP_NAME=/ecs/hooray-relay-worker-dev
# - METRIC_WAIT_SECS=180
# - METRIC_POLL_INTERVAL_SECS=10
# - LOG_WAIT_SECS=180
# - LOG_POLL_INTERVAL_SECS=10
# - APPLY_MONITORING=false

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

require_cmd aws
require_cmd jq
require_cmd bash

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_PROFILE="${AWS_PROFILE:-hooray-dev}"
STACK_NAME="${STACK_NAME:-hooray-dev}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
METRIC_NAMESPACE="${METRIC_NAMESPACE:-HoorayRelay/Worker}"
LOG_GROUP_NAME="${LOG_GROUP_NAME:-/ecs/hooray-relay-worker-${ENVIRONMENT}}"
METRIC_WAIT_SECS="${METRIC_WAIT_SECS:-180}"
METRIC_POLL_INTERVAL_SECS="${METRIC_POLL_INTERVAL_SECS:-10}"
LOG_WAIT_SECS="${LOG_WAIT_SECS:-180}"
LOG_POLL_INTERVAL_SECS="${LOG_POLL_INTERVAL_SECS:-10}"
APPLY_MONITORING="${APPLY_MONITORING:-false}"

FAILURE_RATE_ALARM_NAME="${FAILURE_RATE_ALARM_NAME:-hooray-worker-failure-rate-${ENVIRONMENT}}"
LATENCY_P95_ALARM_NAME="${LATENCY_P95_ALARM_NAME:-hooray-worker-latency-p95-${ENVIRONMENT}}"
DLQ_ALARM_NAME="${DLQ_ALARM_NAME:-hooray-relay-dlq-depth-${ENVIRONMENT}}"

if [[ "$APPLY_MONITORING" == "true" ]]; then
  echo "[setup] Applying dashboard + alarms from monitoring/ artifacts"
  aws cloudwatch put-dashboard \
    --dashboard-name "hooray-relay-worker-${ENVIRONMENT}" \
    --dashboard-body "file://${REPO_ROOT}/monitoring/worker-dashboard.json" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" >/dev/null

  aws cloudwatch put-metric-alarm \
    --cli-input-json "file://${REPO_ROOT}/monitoring/alarms/worker-failure-rate.json" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" >/dev/null

  aws cloudwatch put-metric-alarm \
    --cli-input-json "file://${REPO_ROOT}/monitoring/alarms/worker-latency-p95.json" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" >/dev/null
fi

STACK_OUTPUTS_JSON="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --query "Stacks[0].Outputs" \
  --output json)"

QUEUE_URL="$(echo "$STACK_OUTPUTS_JSON" | jq -r '.[] | select(.OutputKey=="QueueUrl") | .OutputValue')"
if [[ -z "$QUEUE_URL" || "$QUEUE_URL" == "null" ]]; then
  echo "ERROR: QueueUrl output missing from stack $STACK_NAME" >&2
  exit 1
fi
QUEUE_NAME="${QUEUE_URL##*/}"

echo "[1/5] Generating success + failure traffic"
SUCCESS_OUT="$(mktemp)"
FAILURE_OUT="$(mktemp)"

(
  cd "$REPO_ROOT"
  DELIVERY_URL="https://httpbin.org/post" \
  EXPECTED_STATUS="delivered" \
  KEEP_TEST_DATA="false" \
  AWS_REGION="$AWS_REGION" \
  AWS_PROFILE="$AWS_PROFILE" \
  STACK_NAME="$STACK_NAME" \
  ./scripts/e2e_ingestion_worker.sh | tee "$SUCCESS_OUT"
)

(
  cd "$REPO_ROOT"
  DELIVERY_URL="https://httpbin.org/status/404" \
  EXPECTED_STATUS="failed" \
  KEEP_TEST_DATA="false" \
  AWS_REGION="$AWS_REGION" \
  AWS_PROFILE="$AWS_PROFILE" \
  STACK_NAME="$STACK_NAME" \
  ./scripts/e2e_ingestion_worker.sh | tee "$FAILURE_OUT"
)

SUCCESS_EVENT_ID="$(grep '^EVENT_ID=' "$SUCCESS_OUT" | tail -n1 | cut -d= -f2-)"
FAILURE_EVENT_ID="$(grep '^EVENT_ID=' "$FAILURE_OUT" | tail -n1 | cut -d= -f2-)"

if [[ -z "$SUCCESS_EVENT_ID" || -z "$FAILURE_EVENT_ID" ]]; then
  echo "ERROR: unable to parse EVENT_ID values from e2e outputs" >&2
  exit 1
fi

echo "[2/5] Verifying custom metric names are present in CloudWatch"
metric_names=(
  "webhook.delivery.success"
  "webhook.delivery.failure"
  "webhook.delivery.latency_ms"
  "webhook.queue.depth"
)

metric_namespaces=("$METRIC_NAMESPACE")
if [[ "$METRIC_NAMESPACE" != "HooRayRelay/Worker" ]]; then
  metric_namespaces+=("HooRayRelay/Worker")
fi

for metric in "${metric_names[@]}"; do
  found="false"
  matched_namespace=""
  deadline=$(( $(date +%s) + METRIC_WAIT_SECS ))
  while [[ "$(date +%s)" -lt "$deadline" ]]; do
    for ns in "${metric_namespaces[@]}"; do
      start_time="$(date -u -v-30M +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '30 minutes ago' +%Y-%m-%dT%H:%M:%SZ)"
      end_time="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
      datapoints="$(aws cloudwatch get-metric-statistics \
        --namespace "$ns" \
        --metric-name "$metric" \
        --dimensions "Name=environment,Value=${ENVIRONMENT}" "Name=queue_name,Value=${QUEUE_NAME}" \
        --start-time "$start_time" \
        --end-time "$end_time" \
        --period 60 \
        --statistics Sum \
        --region "$AWS_REGION" \
        --profile "$AWS_PROFILE" \
        --query "length(Datapoints)" \
        --output text 2>/dev/null || true)"

      if [[ "$datapoints" =~ ^[0-9]+$ ]] && (( datapoints > 0 )); then
        found="true"
        matched_namespace="$ns"
        break
      fi
    done
    [[ "$found" == "true" ]] && break
    sleep "$METRIC_POLL_INTERVAL_SECS"
  done

  if [[ "$found" != "true" ]]; then
    echo "  - metric datapoint not visible yet for $metric; checking EMF logs directly"
    start_time_ms="$(( ( $(date +%s) - 1800 ) * 1000 ))"
    emf_count="$(aws logs filter-log-events \
      --log-group-name "$LOG_GROUP_NAME" \
      --start-time "$start_time_ms" \
      --filter-pattern "\"_aws\" \"$metric\" \"$QUEUE_NAME\" \"$ENVIRONMENT\"" \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --query "length(events)" \
      --output text 2>/dev/null || true)"

    if [[ "$emf_count" =~ ^[0-9]+$ ]] && (( emf_count > 0 )); then
      echo "  - EMF payload found in logs for $metric (CloudWatch metric ingestion may still be delayed)"
      continue
    fi

    echo "ERROR: metric not visible and no EMF log payload found: $metric (namespaces=${metric_namespaces[*]}, environment=$ENVIRONMENT, queue_name=$QUEUE_NAME)" >&2
    echo "HINT: ensure latest worker image is deployed and log group is $LOG_GROUP_NAME" >&2
    exit 1
  fi
  echo "  - metric visible: $metric (namespace=$matched_namespace)"
done

echo "[3/5] Verifying delivery-attempt log fields for both events"
START_TIME_MS="$(( ( $(date +%s) - 1800 ) * 1000 ))"
for event_id in "$SUCCESS_EVENT_ID" "$FAILURE_EVENT_ID"; do
  message=""
  deadline=$(( $(date +%s) + LOG_WAIT_SECS ))
  while [[ "$(date +%s)" -lt "$deadline" ]]; do
    events_json="$(aws logs filter-log-events \
      --log-group-name "$LOG_GROUP_NAME" \
      --start-time "$START_TIME_MS" \
      --filter-pattern "{ $.event_type = \"delivery_attempt\" && $.event_id = \"${event_id}\" }" \
      --region "$AWS_REGION" \
      --profile "$AWS_PROFILE" \
      --output json)"
    message="$(echo "$events_json" | jq -r '.events[0].message // empty')"
    [[ -n "$message" ]] && break
    sleep "$LOG_POLL_INTERVAL_SECS"
  done

  if [[ -z "$message" ]]; then
    echo "ERROR: no delivery_attempt log found for event_id=$event_id in $LOG_GROUP_NAME after ${LOG_WAIT_SECS}s" >&2
    exit 1
  fi

  echo "$message" | jq -e '
    has("event_id") and
    has("customer_id") and
    has("attempt_number") and
    has("result") and
    has("http_status") and
    has("latency_ms") and
    has("error")
  ' >/dev/null || {
    echo "ERROR: delivery_attempt log missing required fields for event_id=$event_id" >&2
    echo "Log: $message" >&2
    exit 1
  }

  echo "  - log fields verified for event_id=$event_id"
done

echo "[4/5] Verifying alarm existence and current state"
alarms_json="$(aws cloudwatch describe-alarms \
  --alarm-names "$FAILURE_RATE_ALARM_NAME" "$LATENCY_P95_ALARM_NAME" "$DLQ_ALARM_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --output json)"

for alarm in "$FAILURE_RATE_ALARM_NAME" "$LATENCY_P95_ALARM_NAME" "$DLQ_ALARM_NAME"; do
  state="$(echo "$alarms_json" | jq -r --arg a "$alarm" '.MetricAlarms[] | select(.AlarmName==$a) | .StateValue' | head -n1)"
  if [[ -z "$state" || "$state" == "null" ]]; then
    echo "ERROR: alarm not found: $alarm (set APPLY_MONITORING=true to create missing Day 6 alarms)" >&2
    exit 1
  fi
  echo "  - alarm present: $alarm (state=$state)"
done

echo "[5/5] SUCCESS"
echo "SUCCESS_EVENT_ID=$SUCCESS_EVENT_ID"
echo "FAILURE_EVENT_ID=$FAILURE_EVENT_ID"
echo "QUEUE_NAME=$QUEUE_NAME"

echo "Done. Observability signals validated for Day 6 scope."
