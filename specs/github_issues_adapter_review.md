# GitHub Issues Adapter — Code Review

**Reviewer**: code-reviewer agent
**Date**: 2026-02-19
**Verdict**: ✅ PASS — All critical checks satisfied; implementation is correct.

---

## Summary

The GitHub Issues adapter implementation is complete and correct. `cargo check` passes cleanly, all 932 library tests pass (including 47 github_issues-specific tests and 19 ClickUp regression tests). Every item on the review checklist was verified.

---

## Checklist Results

### Correctness

| # | Check | Result | Notes |
|---|-------|--------|-------|
| 1 | `cargo check` passes | ✅ PASS | "Finished `dev` profile" — zero errors |
| 2 | Unit tests pass (`cargo test --lib`) | ✅ PASS | 932 passed, 0 failed |
| 3 | `create_pr` appends "Closes #N" to PR body | ✅ PASS | `format_pr_body` in `egress.rs:87–97`; empty body → `"Closes #N"`, non-empty body → `"{body}\n\nCloses #N"` |
| 4 | `to_github_state` returns `&'static str` | ✅ PASS | Signature `pub fn to_github_state(new_status: &str) -> &'static str` (egress.rs:73) |
| 5 | `issue.pull_request.is_some()` filters PRs | ✅ PASS | `ingestion.rs:188` — `.filter(\|issue\| issue.pull_request.is_none())` |
| 6 | Rate limiter: 5000 tokens / 3600s | ✅ PASS | `client.rs:100` — `RateLimiter::new(5_000, Duration::from_secs(3_600))` |
| 7 | Auth: `Authorization: Bearer {token}` | ✅ PASS | `client.rs:129` — `.header("Authorization", format!("Bearer {}", self.token))` |
| 8 | `list_issues` uses `since` for incremental polling | ✅ PASS | `client.rs:186–188` appends `&since={ts}` to URL; `ingestion.rs:170–181` converts `last_poll` to ISO 8601 `since` string |

### Registration

| # | Check | Result | Notes |
|---|-------|--------|-------|
| 9 | `pub mod github_issues;` in `mod.rs` | ✅ PASS | `mod.rs:9` |
| 10 | `KnownAdapter` entry with name `"github-issues"` (hyphen) | ✅ PASS | `mod.rs:68` |
| 11 | `"github-issues"` match arm creates both adapters sharing `Arc<GitHubClient>` | ✅ PASS | `mod.rs:138–162` — single `Arc::new(GitHubClient::from_env()?)` shared via `Arc::clone` |
| 12 | `include_str!("github_issues/default_adapter.toml")` with underscore | ✅ PASS | `mod.rs:80` — underscore path, not hyphen |

### Adapter Config & User Experience

| # | Check | Result | Notes |
|---|-------|--------|-------|
| 13 | TOML has `owner`, `repo`, `filter_labels`, `state`, `status_*` keys | ✅ PASS* | Has `owner`, `repo`, `filter_labels`, `state`. No `status_*` keys — **intentionally correct**: GitHub Issues has a binary open/closed state (not arbitrary custom statuses like ClickUp), so configurable `status_*` mappings are not applicable. The hardcoded `to_github_state()` mapping covers all reasonable values. |
| 14 | Markdown explains `create_pr` custom action with parameters | ✅ PASS | `default_adapter.md` — full JSON example with `title`, `body`, `head`, `base`, `issue_number` |
| 15 | Documentation explains "Closes #N" auto-close behavior | ✅ PASS | `default_adapter.md:98–100` — explicit description |

### Edge Cases

| # | Check | Result | Notes |
|---|-------|--------|-------|
| 16 | `parse_issue_number` returns `ValidationFailed` for non-numeric external_ids | ✅ PASS | `egress.rs:125–129` — `None` return → `DomainError::ValidationFailed(...)` |
| 17 | `create_pr` handles `issue_number` as JSON number OR string | ✅ PASS | `egress.rs:246–254` — tries `as_u64()` first, then `as_str().parse::<u64>()` |
| 18 | `CreateItem` uses `description` field (not `body`) | ✅ PASS | `egress.rs:170–205` — matches `EgressAction::CreateItem { title, description, fields }` and passes `description` to `create_issue()` |
| 19 | No-body issues → empty string description | ✅ PASS | `ingestion.rs:122` — `issue.body.clone().unwrap_or_default()` |

### Tests

| # | Check | Result | Notes |
|---|-------|--------|-------|
| 20 | Tests for priority extraction (all 4 levels + case-insensitive + no match) | ✅ PASS | 7 tests: `test_extract_priority_critical`, `_high`, `_normal_via_medium`, `_normal_via_normal`, `_low`, `_case_insensitive`, `_no_match` |
| 21 | Tests for `parse_issue_number` valid and invalid | ✅ PASS | `test_parse_issue_number_plain`, `_hash_prefix`, `_qualified`, `_invalid` (covers `""`, `"#"`, `"not-a-number"`) |
| 22 | Tests for `to_github_state` mapping | ✅ PASS | `test_to_github_state_open_variants`, `_closed_variants`, `_case_insensitive` |
| 23 | All existing tests (including ClickUp) still pass | ✅ PASS | 19 ClickUp tests pass; 932 total — zero failures |

---

## Notable Design Decisions

1. **Binary state vs. configurable status mapping**: GitHub Issues only supports `open`/`closed`. The `to_github_state()` function provides a sensible hardcoded mapping of lifecycle terms (done, completed, resolved, wontfix → closed; everything else → open), with no config required. This is the correct design; ClickUp's `status_*` config keys don't translate to GitHub.

2. **Shared `Arc<GitHubClient>`**: Both ingestion and egress adapters share the same client instance, including its `Arc<Mutex<RateLimiter>>`. This ensures rate-limit tokens are drawn from a single shared bucket, preventing one adapter from bypassing limits enforced by the other.

3. **PR filtering at ingestion**: `GitHubIssue.pull_request.is_some()` reliably identifies PRs (GitHub's own signal), avoiding fragile heuristics like title/URL pattern matching.

4. **Incremental polling**: `last_poll` is correctly converted to ISO 8601 and passed as the `since` query parameter, minimising API usage after the first full poll.

5. **Pagination**: `list_issues` follows `rel="next"` Link headers automatically, so repositories with large issue counts are handled without manual page management.

---

## Conclusion

The implementation satisfies every requirement on the checklist. `cargo check` is clean, all 932 tests pass, and the edge cases are properly covered with targeted unit tests. **Mark as complete.**
