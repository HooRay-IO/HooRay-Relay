# ECS Worker Deployment Artifacts

This folder contains copy-pasteable templates and a deploy helper for the non-Lambda worker runtime.

## Files

- `task-definition.template.json`: ECS task definition template for worker container.
- `worker-task-policy.template.json`: IAM policy template for the worker task role.
- `deploy_worker_service.sh`: Helper script to render templates and deploy/update ECS service.

## Prerequisites

- Existing ECS cluster and service.
- Existing ECR repo with pushed worker image.
- AWS CLI configured (`AWS_PROFILE`, `AWS_REGION`).
- The stack that creates DynamoDB/SQS is already deployed (`STACK_NAME`, default `hooray-dev`).

## Required environment variables for deploy script

- `AWS_REGION`
- `AWS_ACCOUNT_ID`
- `ECS_CLUSTER`
- `ECS_SERVICE`
- `WORKER_TASK_ROLE_ARN`
- `WORKER_EXECUTION_ROLE_ARN`
- `WORKER_IMAGE_URI` (example: `123456789012.dkr.ecr.us-west-2.amazonaws.com/hooray-relay-worker:abc123`)

Optional:
- `STACK_NAME` (default: `hooray-dev`)
- `CPU` (default: `256`)
- `MEMORY` (default: `512`)
- `CONTAINER_NAME` (default: `worker`)
- `DESIRED_COUNT` (default: unchanged)

## Create policy from template

```bash
export AWS_REGION=us-west-2
export AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
export ENVIRONMENT=dev
envsubst < infra/ecs/worker-task-policy.template.json > /tmp/worker-task-policy.json
```

Attach `/tmp/worker-task-policy.json` to your worker task role.

## Deploy/update ECS service

```bash
export AWS_REGION=us-west-2
export AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query Account --output text)"
export ECS_CLUSTER=hooray-relay
export ECS_SERVICE=hooray-relay-worker
export WORKER_TASK_ROLE_ARN=arn:aws:iam::123456789012:role/hooray-relay-worker-task-role
export WORKER_EXECUTION_ROLE_ARN=arn:aws:iam::123456789012:role/ecsTaskExecutionRole
export WORKER_IMAGE_URI=123456789012.dkr.ecr.us-west-2.amazonaws.com/hooray-relay-worker:abc123
export STACK_NAME=hooray-dev

./infra/ecs/deploy_worker_service.sh
```

This script:
1. Resolves queue/table names from CloudFormation outputs.
2. Renders task definition JSON.
3. Registers new task definition revision.
4. Updates ECS service to that revision.
