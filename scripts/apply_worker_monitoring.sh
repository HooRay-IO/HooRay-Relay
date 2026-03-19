#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF' >&2
Usage:
  ./scripts/apply_worker_monitoring.sh <dev|staging|prod>

Applies the worker dashboard and worker alarms for the selected environment.
EOF
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "ERROR: required command not found: $cmd" >&2
    exit 1
  fi
}

stack_name_for_env() {
  case "$1" in
    dev) echo "hooray-dev" ;;
    staging) echo "hooray-staging" ;;
    prod) echo "hooray-prod" ;;
    *)
      echo "ERROR: unsupported environment '$1'" >&2
      usage
      exit 1
      ;;
  esac
}

profile_name_for_env() {
  case "$1" in
    dev) echo "hooray-dev" ;;
    staging) echo "hooray-staging" ;;
    prod) echo "hooray-prod" ;;
    *)
      echo "ERROR: unsupported environment '$1'" >&2
      usage
      exit 1
      ;;
  esac
}

render_template() {
  local src="$1"
  local dst="$2"
  sed \
    -e "s|__ENVIRONMENT__|${ENVIRONMENT}|g" \
    -e "s|__QUEUE_NAME__|${QUEUE_NAME}|g" \
    -e "s|__AWS_REGION__|${AWS_REGION}|g" \
    "$src" > "$dst"
}

if [[ $# -ne 1 ]]; then
  usage
  exit 1
fi

require_cmd aws
require_cmd mktemp

ENVIRONMENT="$1"
AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_PROFILE="$(profile_name_for_env "$ENVIRONMENT")"
STACK_NAME="$(stack_name_for_env "$ENVIRONMENT")"

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/.." && pwd)"

QUEUE_URL="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --query "Stacks[0].Outputs[?OutputKey=='QueueUrl'].OutputValue" \
  --output text)"

if [[ -z "$QUEUE_URL" || "$QUEUE_URL" == "None" ]]; then
  echo "ERROR: failed to resolve QueueUrl for stack $STACK_NAME" >&2
  exit 1
fi

QUEUE_NAME="${QUEUE_URL##*/}"

DASHBOARD_FILE="$(mktemp)"
FAILURE_ALARM_FILE="$(mktemp)"
LATENCY_ALARM_FILE="$(mktemp)"

cleanup() {
  rm -f "$DASHBOARD_FILE" "$FAILURE_ALARM_FILE" "$LATENCY_ALARM_FILE"
}

trap cleanup EXIT

render_template "${REPO_ROOT}/monitoring/worker-dashboard.template.json" "$DASHBOARD_FILE"
render_template "${REPO_ROOT}/monitoring/alarms/worker-failure-rate.template.json" "$FAILURE_ALARM_FILE"
render_template "${REPO_ROOT}/monitoring/alarms/worker-latency-p95.template.json" "$LATENCY_ALARM_FILE"

aws cloudwatch put-dashboard \
  --dashboard-name "hooray-relay-worker-${ENVIRONMENT}" \
  --dashboard-body "file://${DASHBOARD_FILE}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE"

aws cloudwatch put-metric-alarm \
  --cli-input-json "file://${FAILURE_ALARM_FILE}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE"

aws cloudwatch put-metric-alarm \
  --cli-input-json "file://${LATENCY_ALARM_FILE}" \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE"

echo "Applied worker monitoring for environment=${ENVIRONMENT} queue_name=${QUEUE_NAME}"
