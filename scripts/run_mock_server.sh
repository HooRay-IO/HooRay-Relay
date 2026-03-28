#!/usr/bin/env bash
set -euo pipefail

MOCK_PORT=${MOCK_PORT:-8089}
MOCK_LATENCY_MS=${MOCK_LATENCY_MS:-50}
MOCK_FAIL_RATE=${MOCK_FAIL_RATE:-0.0}
MOCK_TIMEOUT_RATE=${MOCK_TIMEOUT_RATE:-0.0}
MOCK_TIMEOUT_MS=${MOCK_TIMEOUT_MS:-2000}

export MOCK_PORT MOCK_LATENCY_MS MOCK_FAIL_RATE MOCK_TIMEOUT_RATE MOCK_TIMEOUT_MS

PYTHON_BIN="${PYTHON:-python3}"
if ! command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  echo "Error: Python interpreter '$PYTHON_BIN' not found. Set the PYTHON environment variable to a valid python3 executable." >&2
  exit 1
fi

"$PYTHON_BIN" "$(dirname "${BASH_SOURCE[0]}")/../tests/mock_server.py"
