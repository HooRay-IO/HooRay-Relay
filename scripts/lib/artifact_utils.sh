#!/usr/bin/env bash
set -euo pipefail

ARTIFACT_SAFE_ENV_KEYS=(
  AWS_REGION
  CUSTOMER_ID
  DELIVERY_URL
  DURATION
  ENDPOINT_PROFILE
  ITERATION_VUS
  KEEP_TEST_DATA
  LOG_GROUP_NAME
  METRIC_NAMESPACE
  MODE
  OUT_DIR
  POLL_INTERVAL_SECS
  POLL_TIMEOUT_SECS
  RATE
  RUN_LONG_DLQ_SCENARIO
  RUN_REPLAY_VALIDATION
  SKIP_DELIVERY
  SUMMARY_JSON_PATH
  TARGET_EVENTS
  TEST_RUN_ID
)

write_sanitized_env_snapshot() {
  local out_file="$1"

  mkdir -p "$(dirname "$out_file")"
  : > "$out_file"

  printf 'GENERATED_AT=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$out_file"
  for key in "${ARTIFACT_SAFE_ENV_KEYS[@]}"; do
    if [[ -n "${!key-}" ]]; then
      printf '%s=%q\n' "$key" "${!key}" >> "$out_file"
    fi
  done
}
