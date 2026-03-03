#!/usr/bin/env bash
set -euo pipefail

# Backward-compatible wrapper.
# Canonical worker script: scripts/e2e_day6_worker_observability.sh
echo "WARNING: scripts/e2e_day6_observability.sh is deprecated; use scripts/e2e_day6_worker_observability.sh" >&2
exec "$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)/e2e_day6_worker_observability.sh" "$@"
