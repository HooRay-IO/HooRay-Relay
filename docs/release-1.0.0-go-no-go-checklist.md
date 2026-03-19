# Release 1.0.0 Go / No-Go Checklist

Use this checklist as the final release decision gate for `v1.0.0`.

## Go Criteria

- [x] Environment segregation is in place and verified for `dev`, `staging`, and `prod`
- [x] Release candidate is deployed to `staging`
- [x] Staging deploy uses a pinned artifact or image, not a floating tag
- [x] One full happy-path end-to-end test passes in `staging`
- [x] One controlled failure, retry, and DLQ path passes in `staging`
- [x] Dashboards and alarms are live for queue backlog, DLQ depth, failure rate, worker health, and latency
- [x] Rollback procedure is documented and a previous known-good artifact or tag is available
- [x] Final code review is complete
- [x] `cargo fmt --all -- --check` passes on the release branch
- [x] `cargo clippy --workspace --all-targets --all-features --locked -- -D warnings` passes on the release branch
- [x] `cargo test --workspace --locked` passes on the release branch
- [x] Release metadata is ready: versions bumped, release notes prepared, tag planned
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
- [x] Versions or tags do not match the intended `1.0.0` release
- [ ] Ownership for deploy, rollback, or incident response is unclear

## Minimal Decision Rule

- [ ] GO only if every Go item is true
- [x] NO-GO if any No-Go item is true

## Recommended Sign-Off

- [ ] Engineering owner signs off on code and runtime behavior
- [ ] Infra or platform owner signs off on deploy and rollback path
- [ ] Ops owner signs off on alarms and runbook
- [ ] Release owner signs off on tag and promotion plan
