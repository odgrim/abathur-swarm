# Code Review: GitHub Issues Adapter

**Task**: 8f6e4b3d-9530-401a-bf24-388e586ef6c1
**Reviewer**: code-reviewer agent
**Date**: 2026-02-19
**Status**: PASS

## Build & Tests

| Check | Result |
|-------|--------|
| `cargo check` | ✅ PASS – no errors or warnings |
| `cargo test --lib` | ✅ PASS – 932 tests, 47 GitHub-specific, 0 failures |

## Checklist Results

### Correctness

1. ✅ `cargo check` passes cleanly
2. ✅ All unit tests pass (932 total, 0 failed)
3. ✅ `create_pr` custom action correctly appends `"Closes #N"` to the PR body via `format_pr_body(body, issue_number)`. When body is empty, returns `"Closes #N"`; when non-empty, appends `"\n\nCloses #N"`.
4. ✅ `to_github_state` correctly returns `&'static str` — function signature is `fn to_github_state(new_status: &str) -> &'static str`, no borrow of self or new_status.
5. ✅ `issue.pull_request.is_none()` filter is present in `ingestion.rs` line 188, correctly excluding PRs from ingestion results.
6. ✅ Rate limiter uses `RateLimiter::new(5_000, Duration::from_secs(3_600))` — 5000 tokens / 3600s as required.
7. ✅ Auth header uses `Authorization: Bearer {token}` (not bare token style).
8. ✅ `list_issues` receives `since` from `last_poll.map(|dt| dt.to_rfc3339())`, enabling incremental polling.

### Registration

9. ✅ `pub mod github_issues;` added to `src/adapters/plugins/mod.rs` (line 9)
10. ✅ `KnownAdapter` entry uses `name: "github-issues"` (hyphen, not underscore)
11. ✅ `"github-issues"` match arm in `create_native_adapter()` creates both ingestion and egress adapters sharing the same `Arc<GitHubClient>`
12. ✅ Uses `include_str!("github_issues/default_adapter.toml")` with underscore path (underscore in path, hyphen in name)

### Adapter Config & User Experience

13. ⚠️ `default_adapter.toml` has `owner`, `repo`, `filter_labels`, `state` but **does not have `status_*` keys**.
    **Assessment**: This is **intentional and correct**. GitHub Issues only has binary "open"/"closed" states (not configurable like ClickUp workspace statuses). The `to_github_state()` function handles this mapping internally. Adding `status_*` keys would be dead/unused config that could mislead users.
14. ✅ `default_adapter.md` explains the `create_pr` custom action with all parameters (`title`, `body`, `head`, `base`, `issue_number`)
15. ✅ Documentation explicitly explains: "When `issue_number` is provided, `\"Closes #N\"` is automatically appended to the PR body so that merging the PR closes the linked issue."

### Edge Cases

16. ✅ `parse_issue_number` returns `Option<u64>` (None for non-numeric), and callers correctly map None → `DomainError::ValidationFailed`
17. ✅ `create_pr` handles `issue_number` as either JSON number (`v.as_u64()`) or JSON string (`v.as_str().and_then(|s| s.parse::<u64>().ok())`)
18. ✅ `CreateItem` destructures `description` (not `body`) from `EgressAction::CreateItem` — matches the domain model definition at `adapter.rs:291`
19. ✅ `to_ingestion_item` uses `issue.body.clone().unwrap_or_default()` to produce empty string for None body

### Tests

20. ✅ Priority extraction tests cover all 4 levels: `test_extract_priority_critical`, `_high`, `_normal_via_medium`, `_normal_via_normal`, `_low`, plus `_case_insensitive` and `_no_match`
21. ✅ `parse_issue_number` tests: `_plain`, `_hash_prefix`, `_qualified`, `_invalid`
22. ✅ `to_github_state` tests: `_open_variants`, `_closed_variants`, `_case_insensitive`
23. ✅ All 932 tests pass including ClickUp adapter tests

## Summary

The GitHub Issues adapter implementation is **correct and complete**. All critical checklist items pass. The only deviation from the checklist (missing `status_*` TOML keys) is a deliberate and correct design decision appropriate for GitHub's fixed-state API model, not a defect.

The implementation demonstrates:
- Proper pagination with Link header parsing
- Incremental polling via `since` parameter
- PR filtering at ingestion time
- Correct auth header format
- Appropriate rate limits (5000/hr)
- Comprehensive test coverage (47 tests)
- Full documentation including "Closes #N" auto-close behavior
