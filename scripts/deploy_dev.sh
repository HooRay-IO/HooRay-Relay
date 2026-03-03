#!/usr/bin/env bash
set -euo pipefail

if [[ ! -f samconfig.local.toml ]]; then
  echo "samconfig.local.toml not found."
  echo "Create it from samconfig.local.toml.example and fill in your account/image/subnet/security group values."
  exit 1
fi

require_cmd() {
  local name="$1"
  if ! command -v "$name" >/dev/null 2>&1; then
    echo "ERROR: missing required command: ${name}" >&2
    exit 1
  fi
}

toml_value() {
  local key="$1"
  awk -F'=' -v k="$key" '
    $1 ~ "^[[:space:]]*" k "[[:space:]]*$" {
      v=$2
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", v)
      gsub(/^"|"$/, "", v)
      print v
      exit
    }
  ' samconfig.local.toml
}

PUSH_WORKER_IMAGE="${PUSH_WORKER_IMAGE:-true}"
WORKER_DOCKERFILE="${WORKER_DOCKERFILE:-worker/Dockerfile}"
ECR_REPO="${ECR_REPO:-hooray-relay-worker-dev}"
IMAGE_TAG="${IMAGE_TAG:-$(git rev-parse --short HEAD 2>/dev/null || date +%Y%m%d%H%M%S)}"
LATEST_TAG="${LATEST_TAG:-dev-latest}"
DOCKER_PLATFORM="${DOCKER_PLATFORM:-linux/arm64}"

AWS_PROFILE="${AWS_PROFILE:-$(toml_value profile)}"
AWS_REGION="${AWS_REGION:-$(toml_value region)}"
AWS_DEFAULT_REGION="${AWS_DEFAULT_REGION:-$AWS_REGION}"
export AWS_PROFILE AWS_REGION AWS_DEFAULT_REGION

if [[ "$PUSH_WORKER_IMAGE" == "true" ]]; then
  require_cmd aws
  require_cmd docker

  AWS_ACCOUNT_ID="$(aws sts get-caller-identity --query "Account" --output text)"
  ECR_REGISTRY="${AWS_ACCOUNT_ID}.dkr.ecr.${AWS_REGION}.amazonaws.com"
  SHA_IMAGE_URI="${ECR_REGISTRY}/${ECR_REPO}:${IMAGE_TAG}"
  LATEST_IMAGE_URI="${ECR_REGISTRY}/${ECR_REPO}:${LATEST_TAG}"

  echo "Preparing worker image push:"
  echo "  repo=${ECR_REPO}"
  echo "  sha_tag=${IMAGE_TAG}"
  echo "  latest_tag=${LATEST_TAG}"
  echo "  region=${AWS_REGION}"
  echo "  profile=${AWS_PROFILE}"

  aws ecr describe-repositories --repository-names "$ECR_REPO" >/dev/null 2>&1 || \
    aws ecr create-repository --repository-name "$ECR_REPO" >/dev/null

  aws ecr get-login-password | docker login --username AWS --password-stdin "$ECR_REGISTRY"

  if [[ -n "$DOCKER_PLATFORM" ]]; then
    docker build --platform "$DOCKER_PLATFORM" -f "$WORKER_DOCKERFILE" -t "${ECR_REPO}:${IMAGE_TAG}" .
  else
    docker build -f "$WORKER_DOCKERFILE" -t "${ECR_REPO}:${IMAGE_TAG}" .
  fi

  docker tag "${ECR_REPO}:${IMAGE_TAG}" "$SHA_IMAGE_URI"
  docker tag "${ECR_REPO}:${IMAGE_TAG}" "$LATEST_IMAGE_URI"
  docker push "$SHA_IMAGE_URI"
  docker push "$LATEST_IMAGE_URI"

  echo "Pushed worker images:"
  echo "  ${SHA_IMAGE_URI}"
  echo "  ${LATEST_IMAGE_URI}"
  echo "Ensure WorkerImageUri in samconfig.local.toml points to :${LATEST_TAG}"
fi

# Keep build artifacts aligned with the current template before deploy.
sam build --cached --parallel
sam deploy --resolve-s3 --config-file samconfig.local.toml --config-env default
