# Release 1.0.0 Go / No-Go Checklist

Use this checklist as the final release decision gate for `v1.0.0`.

## Go Criteria

- [ ] Environment segregation is in place and verified for `dev`, `staging`, and `prod`
- [ ] Release candidate is deployed to `staging`
- [ ] Staging deploy uses a pinned artifact or image, not a floating tag
- [ ] One full happy-path end-to-end test passes in `staging`
- [ ] One controlled failure, retry, and DLQ path passes in `staging`
- [ ] Dashboards and alarms are live for queue backlog, DLQ depth, failure rate, worker health, and latency
- [ ] Rollback procedure is documented and a previous known-good artifact or tag is available
- [ ] Final code review is complete
- [ ] `cargo fmt --all -- --check` passes on the release branch
- [ ] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes on the release branch
- [ ] `cargo test --workspace --locked` passes on the release branch
- [ ] Release metadata is ready: versions bumped, release notes prepared, tag planned
- [ ] Prod deploy permissions and owners are confirmed
- [ ] No unresolved Sev-1 or Sev-2 known issues remain

## No-Go Criteria

- [ ] Staging has not been validated end-to-end
- [ ] Retry and DLQ flow has not been proven in deployed infrastructure
- [ ] Prod would deploy from `latest`, `dev-latest`, or any non-pinned artifact
- [ ] Rollback path is unclear or untested
- [ ] Critical alarms are missing or not actionable
- [ ] Worker and ingestion contract changed without coordinated verification
- [ ] There are open bugs affecting idempotency, delivery correctness, status transitions, or replay safety
- [ ] Prod secrets, IAM roles, or environment wiring are still ambiguous
- [ ] Release branch still contains unfinished release-hardening changes
- [ ] Versions or tags do not match the intended `1.0.0` release
- [ ] Ownership for deploy, rollback, or incident response is unclear

## Minimal Decision Rule

- [ ] GO only if every Go item is true
- [ ] NO-GO if any No-Go item is true

## Recommended Sign-Off

- [ ] Engineering owner signs off on code and runtime behavior
- [ ] Infra or platform owner signs off on deploy and rollback path
- [ ] Ops owner signs off on alarms and runbook
- [ ] Release owner signs off on tag and promotion plan
