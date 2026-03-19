# Release 1.0.0 Notes

Planned release tag: `v1.0.0`

## Scope

`v1.0.0` ships the current production runtime split:

- ingestion API on AWS Lambda via SAM
- delivery worker on ECS as a long-running SQS poller

## Included In This Release

- environment-segregated `dev`, `staging`, and `prod` deployments
- pinned staging and prod worker images
- staging happy-path end-to-end validation
- staging failure, retry, and DLQ validation
- rollback runbook in `docs/rollback.md`
- worker dashboards and alarms for staging and prod

## Current Release Inputs

- crate version: `1.0.0`
- planned git tag: `v1.0.0`
- release branch: `release-1.0.0-readiness`

## Approval And Ownership Model

- prod deployment path is gated by the GitHub Actions `prod-approval` environment before `deploy-prod`
- `prod-approval` reviewers are GitHub users or GitHub teams configured on the GitHub environment
- those GitHub reviewers approve workflow execution; they do not grant AWS API permissions directly
- the actual prod deploy and rollback permissions come from the AWS role assumed by GitHub Actions via OIDC
- platform or infra owner is responsible for prod deploy execution and rollback approval
- engineering owner is responsible for release code and runtime behavior
- delivery worker owner is responsible for worker logic, retry behavior, and replay behavior
- customer integration owner is responsible for customer endpoint and contract issues
- release owner is responsible for promotion freeze, tag timing, and incident timeline capture

Named approvers and environment reviewers are configured outside the repo in GitHub environment settings, and AWS deploy rights are configured outside the repo in AWS IAM.

## Known Limits

- the delivery worker remains ECS-based for `1.0.0`; Lambda migration is deferred to a later release
- final code review and final owner sign-off remain release gates outside this document
