#!/usr/bin/env bash
set -euo pipefail

require_set() {
  local name="$1"
  if [[ -z "${!name:-}" ]]; then
    echo "ERROR: ${name} is required" >&2
    exit 1
  fi
}

require_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "ERROR: missing required command: ${name}" >&2
    exit 1
  fi
}

require_cmd aws
require_cmd envsubst

AWS_REGION="${AWS_REGION:-}"
ECS_CLUSTER="${ECS_CLUSTER:-}"
ECS_SERVICE="${ECS_SERVICE:-}"
WORKER_TASK_ROLE_ARN="${WORKER_TASK_ROLE_ARN:-}"
WORKER_EXECUTION_ROLE_ARN="${WORKER_EXECUTION_ROLE_ARN:-}"
WORKER_IMAGE_URI="${WORKER_IMAGE_URI:-}"
STACK_NAME="${STACK_NAME:-hooray-dev}"
CPU="${CPU:-256}"
MEMORY="${MEMORY:-512}"
CONTAINER_NAME="${CONTAINER_NAME:-worker}"
TASK_FAMILY="${TASK_FAMILY:-${ECS_SERVICE:-hooray-relay-worker}}"
LOG_GROUP_NAME="${LOG_GROUP_NAME:-/ecs/${ECS_SERVICE:-hooray-relay-worker}}"
DESIRED_COUNT="${DESIRED_COUNT:-}"

require_set AWS_REGION
require_set ECS_CLUSTER
require_set ECS_SERVICE
require_set WORKER_TASK_ROLE_ARN
require_set WORKER_EXECUTION_ROLE_ARN
require_set WORKER_IMAGE_URI

EVENTS_TABLE="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --query "Stacks[0].Outputs[?OutputKey=='EventsTableName'].OutputValue" \
  --output text)"
CONFIGS_TABLE="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --query "Stacks[0].Outputs[?OutputKey=='ConfigsTableName'].OutputValue" \
  --output text)"
QUEUE_URL="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --query "Stacks[0].Outputs[?OutputKey=='QueueUrl'].OutputValue" \
  --output text)"
BREAKER_STATES_TABLE="$(aws cloudformation describe-stacks \
  --stack-name "$STACK_NAME" \
  --region "$AWS_REGION" \
  --query "Stacks[0].Outputs[?OutputKey=='BreakerStatesTableName'].OutputValue" \
  --output text)"

for resolved in EVENTS_TABLE CONFIGS_TABLE QUEUE_URL BREAKER_STATES_TABLE; do
  if [[ -z "${!resolved}" || "${!resolved}" == "None" ]]; then
    echo "ERROR: could not resolve ${resolved} from stack ${STACK_NAME}" >&2
    exit 1
  fi
done

tmp_task_json="$(mktemp)"
trap 'rm -f "$tmp_task_json"' EXIT

export AWS_REGION CPU MEMORY CONTAINER_NAME LOG_GROUP_NAME TASK_FAMILY \
  WORKER_TASK_ROLE_ARN WORKER_EXECUTION_ROLE_ARN WORKER_IMAGE_URI \
  EVENTS_TABLE CONFIGS_TABLE QUEUE_URL BREAKER_STATES_TABLE

envsubst < infra/ecs/task-definition.template.json > "$tmp_task_json"

echo "[1/3] Registering ECS task definition"
task_def_arn="$(aws ecs register-task-definition \
  --region "$AWS_REGION" \
  --cli-input-json "file://${tmp_task_json}" \
  --query "taskDefinition.taskDefinitionArn" \
  --output text)"

echo "[2/3] Updating ECS service to ${task_def_arn}"
if [[ -n "$DESIRED_COUNT" ]]; then
  aws ecs update-service \
    --region "$AWS_REGION" \
    --cluster "$ECS_CLUSTER" \
    --service "$ECS_SERVICE" \
    --task-definition "$task_def_arn" \
    --desired-count "$DESIRED_COUNT" \
    >/dev/null
else
  aws ecs update-service \
    --region "$AWS_REGION" \
    --cluster "$ECS_CLUSTER" \
    --service "$ECS_SERVICE" \
    --task-definition "$task_def_arn" \
    >/dev/null
fi

echo "[3/3] Waiting for service to stabilize"
aws ecs wait services-stable \
  --region "$AWS_REGION" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE"

echo "SUCCESS: service updated"
echo "TASK_DEFINITION_ARN=${task_def_arn}"
