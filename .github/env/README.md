# GitHub Environment Bulk Import

Use these files with GitHub CLI to populate environment-scoped Actions variables
and secrets.

## Variables

```bash
cp .github/env/dev.env.example .github/env/dev.env
cp .github/env/staging.env.example .github/env/staging.env
cp .github/env/prod.env.example .github/env/prod.env

gh variable set -e dev -f .github/env/dev.env
gh variable set -e staging -f .github/env/staging.env
gh variable set -e prod -f .github/env/prod.env
```

## Secrets

Copy each `*.secrets.env.example` file to a local `*.secrets.env` file if you
do not want the secret value stored in the repo, then import with:

```bash
gh secret set -e dev -f .github/env/dev.secrets.env.example
gh secret set -e staging -f .github/env/staging.secrets.env.example
gh secret set -e prod -f .github/env/prod.secrets.env.example
```

## Notes

- The repo tracks `*.env.example` templates only. Real `.env` files under
  `.github/env/` are ignored.
- Replace the placeholder subnet and security group values before import.
- `prod-approval` should contain required reviewers only; it does not need deploy
  secrets or variables.
- After moving deploy-critical values into environments, remove overlapping
  repo-level Actions variables and secrets.
