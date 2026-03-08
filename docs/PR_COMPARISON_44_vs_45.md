# PR Comparison: #44 vs #45

> Generated 2026-03-08. Compares the two open pull requests that both add a Day 9 k6 load-test
> and suggests actions for the team.

---

## Summary table

| Attribute | PR #44 — `chore/day9-loadtest-delivered-at` | PR #45 — `feat/engineer1-day9-load-test` |
|---|---|---|
| **Title** | feat: add k6 load test and persist delivered\_at on delivery | test: add Day 9 k6 load test |
| **Author** | OneLastStop529 (Engineer 2) | Raydiate09 (Engineer 1) |
| **Created** | 2026-03-06 | 2026-03-08 |
| **Base SHA** | `fe9bdc70` (older main) | `744aaa64` (current main) |
| **Files changed** | 8 | 3 |
| **Lines added / deleted** | +619 / −53 | +115 / −4 |
| **Commits** | 4 | 1 |
| **Mergeable state** | clean (needs rebase) | clean |
| **Reviewer requested** | Raydiate09 | — |
| **PR URL** | [#44](https://github.com/HooRay-IO/HooRay-Relay/pull/44) | [#45](https://github.com/HooRay-IO/HooRay-Relay/pull/45) |

---

## What each PR does

### PR #44 — broad scope

| File | Change |
|---|---|
| `.envrc.example` | Adds `DELIVERY_URL`, `API_URL`, `CUSTOMER_ID` env-var defaults for local/e2e/load runs |
| `.gitignore` | 3 new ignores |
| `docs/ENGINEER_2_TIMELINE.md` | Expands Day 9 into an actionable baseline + tuning plan |
| `docs/day9-performance-report.md` | **New** — performance report template (125 lines) |
| `scripts/day9_seed_and_report.sh` | **New** — seed and report script (111 lines) |
| `tests/load_test.js` | **New** — k6 ramping-VU script, 197 lines |
| `worker/src/main.rs` | Adds regression test asserting `ConfigNotFound` is terminal (non-retry) |
| `worker/src/services/dynamodb.rs` | Persists `delivered_at` timestamp in DynamoDB when an event is marked delivered |

**Key production change:** `delivered_at` is now written to DynamoDB on delivery — this is a real data-model improvement beyond the load test.

### PR #45 — narrow scope

| File | Change |
|---|---|
| `docs/ENGINEER_1_TIMELINE.md` | Marks "create k6 script" task complete |
| `ingestion/README.md` | Adds Day 9 section, TOC entry, and roadmap status updates |
| `tests/load_test.js` | **New** — k6 constant-arrival-rate script, 82 lines |

---

## k6 script comparison

Both PRs add `tests/load_test.js`, creating a direct merge conflict.

| Aspect | PR #44 script | PR #45 script |
|---|---|---|
| **Executor** | `ramping-vus` | `constant-arrival-rate` |
| **Lines** | 197 | 82 |
| **UUID source** | Remote CDN (`jslib.k6.io`) | Remote CDN (`jslib.k6.io`) |
| **BASE\_URL trailing slash** | Not normalised | Not normalised |
| **TARGET\_URL default** | `https://httpbin.org/post` | `https://httpbin.org/post` |
| **Setup failure handling** | Only `check()`'d — test continues on failure | Only `check()`'d — test continues on failure |
| **Thresholds** | `http_req_failed < 0.001`, `p(95) < 100ms` | `http_req_failed < 0.001`, `p(95) < 100ms` |
| **Scenario features** | Multi-stage ramp-up / ramp-down | Fixed rate arrival |

**Executor choice:** `constant-arrival-rate` (PR #45) more accurately simulates a sustained
target RPS and is the executor recommended by k6 docs for throughput SLO testing. The
`ramping-vus` approach (PR #44) is useful for warming up but does not guarantee a constant
arrival rate.

---

## Shared issues found in review

Both k6 scripts have the following problems that need fixing regardless of which script is kept:

1. **Remote UUID import** — importing from `https://jslib.k6.io/...` at runtime introduces a
   supply-chain risk and makes tests fail in air-gapped environments. Replace with a local
   `uuidv4()` polyfill.
2. **BASE\_URL double-slash** — the CloudFormation `IngestionApiUrl` output ends with a trailing
   slash (e.g. `.../Prod/`), so `${BASE_URL}/webhooks/receive` becomes `.../Prod//webhooks/receive`.
   Normalise `BASE_URL` by stripping any trailing slash at the top of the script.
3. **Unsafe TARGET\_URL default** — defaulting to `https://httpbin.org/post` during a 500 rps
   load test can generate large outbound traffic to a third-party service when the worker is
   running. Require `TARGET_URL` explicitly when `SETUP_CREATE_CONFIG=true`, or default to a
   clearly invalid placeholder.
4. **Setup failure silent continuation** — if the config-create POST in `setup()` fails and
   `SETUP_CREATE_CONFIG=true`, the test continues and hammers an unconfigured customer. Throw
   or call `fail()` on a non-2xx response so the test aborts.

---

## Issues specific to PR #44

- **⚠️ P1 Security — committed secret:** A commit in this PR added `artifacts/` snapshots
  containing `OPENCLAW_GATEWAY_TOKEN` and potentially other credentials. The token must be
  treated as **compromised** and rotated immediately. The snapshots must be removed from git
  history (or filtered out of the `scripts/day9_seed_and_report.sh` output before committing).
  `artifacts/` should be added to `.gitignore`.
- **API\_URL trailing slash** — `.envrc.example` sets `API_URL` by stripping the trailing slash
  from the CloudFormation output. Downstream scripts that append `/webhooks/...` directly may
  produce malformed URLs. Keep the trailing slash or document the stripping convention
  consistently.
- **Markdown table format** — `docs/day9-performance-report.md` uses leading `||` on table rows,
  rendering an extra empty column. Fix to standard Markdown table syntax.
- **`update_event_status` fragility** — the `dynamodb.rs` implementation builds the DynamoDB
  update expression by overwriting it in multiple branches (last-branch-wins). This is fragile
  when adding more optional fields. Accumulate `SET` parts into a `Vec` and join them once.
- **`delivered_at` not tested** — the new persistence path lacks a unit or integration test that
  calls `update_event_status` and asserts the stored `delivered_at` and the removal of
  `next_retry_at`.
- **Needs rebase** — base SHA `fe9bdc70` is behind the current main `744aaa64`. A rebase is
  required before merging.

---

## Issues specific to PR #45

- **Dangling sentence in README** — the line about `start` being a `std::time::Instant` captured
  at handler entry appears at the end of the Day 9 section but belongs to the handler/observability
  section. Remove it or move it to the correct place.
- **Typo in README** — "Validate 500 req/sec sustained throughput with <$100ms p95" should be
  "<100ms p95" (no `$`).
- **TARGET\_URL not documented in README** — the run instructions enable `SETUP_CREATE_CONFIG=true`
  but do not mention `TARGET_URL`, leaving readers unaware that the worker will deliver to
  `https://httpbin.org/post`.

---

## Suggested actions

### Immediate (before any merge)

1. **Rotate `OPENCLAW_GATEWAY_TOKEN`** and any other credentials visible in the PR #44
   `artifacts/` snapshots. This is a P1 security action.
2. **Add `artifacts/` to `.gitignore`** to prevent future accidental credential commits.
3. **Remove or filter the secret from git history** in PR #44 using `git filter-repo` or by
   force-pushing a cleaned branch.

### Merge strategy

4. **Merge PR #44 first** (after the security fix and rebase) because it contains the
   **production-quality `delivered_at` persistence** change (`worker/src/services/dynamodb.rs`)
   that is independent of the load test and immediately useful. It also adds the `ConfigNotFound`
   regression test. These changes should not be blocked by the load-test discussion.

   - Option: split PR #44 into two focused PRs — (a) `delivered_at` + regression test, (b) load
     test + docs — to keep the production code change reviewable on its own.

5. **Close PR #45** after PR #44 is merged, because:
   - PR #44 already covers the k6 load-test requirement.
   - Both PRs write to the same `tests/load_test.js` file, causing a conflict.
   - The documentation updates in PR #45 (`ENGINEER_1_TIMELINE.md`, `ingestion/README.md`) can be
     applied as a follow-up commit on main.

   Alternatively, keep PR #45 open only if the team prefers the `constant-arrival-rate` executor.
   In that case close PR #44's k6 script portion and cherry-pick only the `delivered_at` changes.

### Before merging whichever k6 script is chosen

6. Replace the remote `jslib.k6.io` UUID import with a local `uuidv4()` polyfill.
7. Add `BASE_URL` trailing-slash normalisation at the top of the script.
8. Make `TARGET_URL` required (or clearly invalid by default) when `SETUP_CREATE_CONFIG=true` and
   abort the test run on config-create failure.

### Code quality follow-ups (can be separate PRs)

9. Refactor `update_event_status` in `dynamodb.rs` to accumulate SET expressions in a `Vec`
   rather than overwriting a single string.
10. Add a unit/integration test covering `delivered_at` persistence and `next_retry_at` removal.
11. Fix the `docs/day9-performance-report.md` Markdown table formatting.
12. Fix the `ingestion/README.md` typo (`<$100ms` → `<100ms`) and dangling sentence.
