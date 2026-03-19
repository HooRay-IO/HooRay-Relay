# Rollback Runbook

Use this runbook to roll back HooRay-Relay after a bad deploy in `staging` or `prod`.

This system currently deploys:
- ingestion as a SAM-managed Lambda/API stack
- worker as an ECS/Fargate service managed by the same CloudFormation stack

The preferred rollback unit is the previously known-good worker image and stack configuration.

## Scope

Use this runbook when:
- worker delivery failures spike after deploy
- queue backlog grows unexpectedly after deploy
- ECS worker health degrades after deploy
- ingestion or worker alarms regress after release promotion

This runbook assumes:
- stack names are `hooray-staging` and `hooray-prod`
- worker ECS service names are `hooray-relay-worker-staging` and `hooray-relay-worker-prod`
- worker image tags are pinned in ECR

## Required Inputs

Before rollback, identify:
- target environment: `staging` or `prod`
- current stack name
- current deployed worker image URI
- previous known-good worker image URI
- current ECS task definition ARN
- release commit/tag being rolled back from
- release commit/tag being rolled back to

## Fast Triage

Confirm the issue is real and identify scope:

```bash
ENV=staging
AWS_PROFILE=hooray-${ENV}
AWS_REGION=us-west-2
STACK_NAME=hooray-${ENV}
ECS_CLUSTER=hooray-relay-worker-${ENV}
ECS_SERVICE=hooray-relay-worker-${ENV}
```

Check ECS service health:

```bash
aws ecs describe-services \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE" \
  --query "services[0].{desired:desiredCount,running:runningCount,pending:pendingCount,status:status,taskDefinition:taskDefinition}" \
  --output table
```

Check stack status:

```bash
aws cloudformation describe-stacks \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --stack-name "$STACK_NAME" \
  --query "Stacks[0].{name:StackName,status:StackStatus,lastUpdated:LastUpdatedTime}" \
  --output table
```

Check worker alarms:

```bash
aws cloudwatch describe-alarms \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --alarm-names \
    "hooray-worker-running-count-${ENV}" \
    "hooray-worker-failure-rate-${ENV}" \
    "hooray-worker-latency-p95-${ENV}" \
    "hooray-relay-dlq-depth-${ENV}" \
  --query "MetricAlarms[].{name:AlarmName,state:StateValue}" \
  --output table
```

## Determine Current Worker Image

Get the currently deployed worker image from CloudFormation:

```bash
aws cloudformation describe-stacks \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --stack-name "$STACK_NAME" \
  --query "Stacks[0].Parameters[?ParameterKey=='WorkerImageUri'].ParameterValue" \
  --output text
```

Get the currently running ECS task definition:

```bash
aws ecs describe-services \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE" \
  --query "services[0].taskDefinition" \
  --output text
```

Get the exact image for that task definition:

```bash
TASK_DEF_ARN="<task-definition-arn>"

aws ecs describe-task-definition \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --task-definition "$TASK_DEF_ARN" \
  --query "taskDefinition.containerDefinitions[0].image" \
  --output text
```

## Preferred Rollback: Redeploy Stack With Previous Good Worker Image

Use this when the regression is worker-related and the previous image is known.

1. Identify the previous known-good image URI.

2. Redeploy the stack with that image URI.

For local rollback using environment-specific SAM config:

```bash
ENV=staging
AWS_PROFILE=hooray-${ENV}
AWS_REGION=us-west-2
PREV_IMAGE_URI="<previous-good-worker-image-uri>"

sam deploy \
  --config-file "samconfig.${ENV}.toml" \
  --config-env default \
  --resolve-s3 \
  --parameter-overrides \
    Environment="${ENV}" \
    SqsVisibilityTimeoutSeconds=60 \
    SqsMaxReceiveCount=4 \
    EnableEcsWorker=true \
    WorkerImageUri="${PREV_IMAGE_URI}"
```

Notes:
- Include any additional environment-specific overrides normally required by your environment.
- If your local `samconfig` does not fully cover ECS values, use the same parameter set as the normal deploy path.

## Fallback Rollback: ECS Service Only

Use this when:
- the issue is isolated to the worker,
- you need a faster rollback,
- and you already know the previous good ECS task definition or image.

Option A: roll back to the previous task definition ARN

```bash
PREV_TASK_DEF_ARN="<previous-good-task-definition-arn>"

aws ecs update-service \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --service "$ECS_SERVICE" \
  --task-definition "$PREV_TASK_DEF_ARN"

aws ecs wait services-stable \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE"
```

Option B: register a new task definition with the previous image URI, then update the service

Use [`infra/ecs/deploy_worker_service.sh`](/Users/yizehu/Workspaces/HooRay-Relay/infra/ecs/deploy_worker_service.sh) if you need to roll back outside SAM with a known prior image.

## Ingestion Rollback

If the problem is in ingestion rather than worker delivery:
- redeploy the previous known-good release commit/tag through the standard SAM deploy path
- verify the `IngestionFunction` and API behavior after deploy

If both ingestion and worker changed in the bad release, roll back to the previous known-good release commit/tag and redeploy the whole stack.

## Verification After Rollback

After rollback, verify all of the following:

- ECS service has `desired=running` and `pending=0`
- worker running-count alarm is not breaching after metrics settle
- worker failure-rate and latency alarms return to normal
- DLQ growth stops
- queue visible depth is stable or draining
- one happy-path end-to-end event is delivered successfully

Suggested checks:

```bash
aws ecs describe-services \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --cluster "$ECS_CLUSTER" \
  --services "$ECS_SERVICE" \
  --query "services[0].{desired:desiredCount,running:runningCount,pending:pendingCount,rollout:deployments[0].rolloutState}" \
  --output table
```

```bash
aws cloudwatch describe-alarms \
  --region "$AWS_REGION" \
  --profile "$AWS_PROFILE" \
  --alarm-names \
    "hooray-worker-running-count-${ENV}" \
    "hooray-worker-failure-rate-${ENV}" \
    "hooray-worker-latency-p95-${ENV}" \
    "hooray-relay-dlq-depth-${ENV}" \
  --query "MetricAlarms[].{name:AlarmName,state:StateValue}" \
  --output table
```

## Release Discipline

Before every release, record:
- release tag or commit SHA
- deployed `WorkerImageUri`
- previous known-good `WorkerImageUri`
- ECS task definition ARN

Without that record, rollback becomes slower and more error-prone.

## Ownership

Recommended sign-off roles during rollback:
- engineering owner: confirm root cause and safe rollback target
- infra/platform owner: execute or approve rollback in `prod`
- release owner: freeze promotions and record incident timeline

## Follow-Up After Rollback

After service is stable:
- capture the bad image/tag and rollback target in the incident record
- keep the failing release out of further promotion
- create follow-up fixes before reattempting release
