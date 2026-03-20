#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF' >&2
Usage:
  eval "$(./scripts/use_env.sh <dev|staging|prod>)"
  ./scripts/use_env.sh <dev|staging|prod> --check

This script prints shell exports for the selected environment by resolving
CloudFormation stack outputs from AWS. Use eval to load them into your shell.
EOF
}

require_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "ERROR: missing required command: ${name}" >&2
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

output_value() {
  local stack_name="$1"
  local key="$2"

  aws cloudformation describe-stacks \
    --stack-name "$stack_name" \
    --region "$AWS_REGION" \
    --query "Stacks[0].Outputs[?OutputKey=='${key}'].OutputValue" \
    --output text
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
  usage
  exit 1
fi

ENVIRONMENT="$1"
MODE="${2:-exports}"

if [[ "$MODE" != "exports" && "$MODE" != "--check" ]]; then
  usage
  exit 1
fi

require_cmd aws

STACK_NAME="$(stack_name_for_env "$ENVIRONMENT")"
AWS_PROFILE="$(profile_name_for_env "$ENVIRONMENT")"
AWS_REGION="${AWS_REGION:-us-west-2}"
AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-$AWS_REGION}"
WORKER_ECR_REPOSITORY="${WORKER_ECR_REPOSITORY:-hooray-relay-worker-${ENVIRONMENT}}"

export AWS_PROFILE AWS_REGION AWS_DEFAULT_REGION

if [[ "$MODE" == "--check" ]]; then
  aws sts get-caller-identity --output json >/dev/null
  aws cloudformation describe-stacks \
    --stack-name "$STACK_NAME" \
    --region "$AWS_REGION" \
    --query "Stacks[0].{Name:StackName,Status:StackStatus}" \
    --output table
  exit 0
fi

EVENTS_TABLE="$(output_value "$STACK_NAME" "EventsTableName")"
IDEMPOTENCY_TABLE="$(output_value "$STACK_NAME" "IdempotencyTableName")"
CONFIGS_TABLE="$(output_value "$STACK_NAME" "ConfigsTableName")"
BREAKER_STATES_TABLE="$(output_value "$STACK_NAME" "BreakerStatesTableName")"
QUEUE_URL="$(output_value "$STACK_NAME" "QueueUrl")"
DLQ_URL="$(output_value "$STACK_NAME" "DLQUrl")"
INGESTION_API_URL="$(output_value "$STACK_NAME" "IngestionApiUrl")"
AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query "Account" --output text)"
ECR_REGISTRY="${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
IMAGE_TAG="$(git rev-parse --short HEAD 2>/dev/null || echo "latest")"
IMAGE_URI="${ECR_REGISTRY}/${WORKER_ECR_REPOSITORY}:${IMAGE_TAG}"

cat <<EOF
export ENVIRONMENT="${ENVIRONMENT}"
export AWS_PROFILE="${AWS_PROFILE}"
export AWS_REGION="${AWS_REGION}"
export AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION}"
export STACK_NAME="${STACK_NAME}"
export EVENTS_TABLE="${EVENTS_TABLE}"
export IDEMPOTENCY_TABLE="${IDEMPOTENCY_TABLE}"
export CONFIGS_TABLE="${CONFIGS_TABLE}"
export BREAKER_STATES_TABLE="${BREAKER_STATES_TABLE}"
export WEBHOOK_EVENTS_TABLE="${EVENTS_TABLE}"
export WEBHOOK_CONFIGS_TABLE="${CONFIGS_TABLE}"
export QUEUE_URL="${QUEUE_URL}"
export DLQ_URL="${DLQ_URL}"
export INGESTION_API_URL="${INGESTION_API_URL}"
export API_URL="${INGESTION_API_URL%/}"
export AWS_ACCOUNT_ID="${AWS_ACCOUNT_ID}"
export ECR_REGISTRY="${ECR_REGISTRY}"
export ECR_REPO="${WORKER_ECR_REPOSITORY}"
export IMAGE_TAG="${IMAGE_TAG}"
export IMAGE_URI="${IMAGE_URI}"
EOF
