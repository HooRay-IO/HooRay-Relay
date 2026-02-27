#!/usr/bin/env bash
set -euo pipefail

if [[ ! -f samconfig.local.toml ]]; then
  echo "samconfig.local.toml not found."
  echo "Create it from samconfig.local.toml.example and fill in your account/image/subnet/security group values."
  exit 1
fi

# Keep build artifacts aligned with the current template before deploy.
sam build --cached --parallel
sam deploy --resolve-s3 --config-file samconfig.local.toml --config-env default
