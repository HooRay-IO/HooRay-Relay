#!/usr/bin/env bash
set -euo pipefail

# Day 6 ingestion observability validation:
# 1) Optionally apply ingestion dashboard
# 2) Verify dashboard exists
# 3) Verify required custom metrics are discoverable
# 4) Verify required alarms exist

AWS_PROFILE="${AWS_PROFILE:-hooray-dev}"
AWS_REGION="${AWS_REGION:-us-west-2}"
ENVIRONMENT="${ENVIRONMENT:-dev}"
METRIC_NAMESPACE="${METRIC_NAMESPACE:-HoorayRelay/Ingestion}"
APPLY_DASHBOARD="${APPLY_DASHBOARD:-false}"
STRICT_METRICS="${STRICT_METRICS:-false}"

DASHBOARD_NAME="${DASHBOARD_NAME:-hooray-relay-ingestion-${ENVIRONMENT}}"
INGESTION_ERROR_ALARM_NAME="${INGESTION_ERROR_ALARM_NAME:-hooray-relay-ingestion-errors-${ENVIRONMENT}}"
INGESTION_ENQUEUE_ALARM_NAME="${INGESTION_ENQUEUE_ALARM_NAME:-hooray-relay-ingestion-enqueue-failure-${ENVIRONMENT}}"
INGESTION_EVENT_CREATE_ALARM_NAME="${INGESTION_EVENT_CREATE_ALARM_NAME:-hooray-relay-ingestion-event-create-failure-${ENVIRONMENT}}"
INGESTION_LATENCY_ALARM_NAME="${INGESTION_LATENCY_ALARM_NAME:-hooray-relay-ingestion-latency-${ENVIRONMENT}}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

for cmd in aws jq; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
done

if [[ "$APPLY_DASHBOARD" == "true" ]]; then
  echo "[setup] Applying ingestion dashboard from monitoring/ingestion-dashboard.json"
  aws cloudwatch put-dashboard \
    --dashboard-name "$DASHBOARD_NAME" \
    --dashboard-body "file://${REPO_ROOT}/monitoring/ingestion-dashboard.json" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" >/dev/null
fi

echo "[1/3] Verifying ingestion dashboard exists"
if ! aws cloudwatch get-dashboard \
  --dashboard-name "$DASHBOARD_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" >/dev/null 2>&1; then
  echo "ERROR: dashboard not found: $DASHBOARD_NAME (set APPLY_DASHBOARD=true to create it)" >&2
  exit 1
fi
echo "  - dashboard present: $DASHBOARD_NAME"

echo "[2/3] Verifying required custom metric names are discoverable"
always_on_metric_names=(
  "webhook.receive.count"
  "webhook.receive.latency_ms"
)

scenario_metric_names=(
  "webhook.idempotency.duplicate.count"
  "webhook.event.create.failure.count"
  "webhook.enqueue.failure.count"
)

for metric in "${always_on_metric_names[@]}"; do
  count="$(aws cloudwatch list-metrics \
    --namespace "$METRIC_NAMESPACE" \
    --metric-name "$metric" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    | jq -r '.Metrics | length')"
  if [[ "$count" -lt 1 ]]; then
    echo "ERROR: metric not found in namespace $METRIC_NAMESPACE: $metric" >&2
    exit 1
  fi
  echo "  - metric present: $metric"
done

for metric in "${scenario_metric_names[@]}"; do
  count="$(aws cloudwatch list-metrics \
    --namespace "$METRIC_NAMESPACE" \
    --metric-name "$metric" \
    --region "$AWS_REGION" \
    --profile "$AWS_PROFILE" \
    | jq -r '.Metrics | length')"
  if [[ "$count" -lt 1 ]]; then
    if [[ "$STRICT_METRICS" == "true" ]]; then
      echo "ERROR: metric not found in namespace $METRIC_NAMESPACE: $metric" >&2
      exit 1
    fi
    echo "  - warning: metric not found yet: $metric (emit at least one matching scenario, or set STRICT_METRICS=true to require it)"
    continue
  fi
  echo "  - metric present: $metric"
done

echo "[3/3] Verifying required alarm names exist"
alarms_json="$(aws cloudwatch describe-alarms \
  --alarm-names \
    "$INGESTION_ERROR_ALARM_NAME" \
    "$INGESTION_ENQUEUE_ALARM_NAME" \
    "$INGESTION_EVENT_CREATE_ALARM_NAME" \
    "$INGESTION_LATENCY_ALARM_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE")"

for alarm in \
  "$INGESTION_ERROR_ALARM_NAME" \
  "$INGESTION_ENQUEUE_ALARM_NAME" \
  "$INGESTION_EVENT_CREATE_ALARM_NAME" \
  "$INGESTION_LATENCY_ALARM_NAME"; do
  state="$(echo "$alarms_json" | jq -r --arg a "$alarm" '.MetricAlarms[] | select(.AlarmName==$a) | .StateValue' | head -n1)"
  if [[ -z "$state" || "$state" == "null" ]]; then
    echo "ERROR: alarm not found: $alarm (deploy template updates with sam deploy)" >&2
    exit 1
  fi
  echo "  - alarm present: $alarm (state=$state)"
done

echo "SUCCESS: ingestion Day 6 observability checks passed."
