# BidRadar Goals

## Current Goal

### GOAL-001 — Data Pipeline Prototype

**Status:** IN_PROGRESS

### Definition of Done

- [x] Architecture decision: Rust + PostgreSQL
- [x] Demand-driven data strategy
- [x] Watch Rule model: Agency + Tender Keyword
- [x] Rust workspace
- [x] PostgreSQL schema / migrations
- [x] Axum API skeleton
- [x] Rule Engine preview endpoint
- [x] Rule Engine unit tests
- [ ] PCC daily source adapter
- [ ] Fetch notices for a specified date
- [ ] Persist matched tenders only
- [ ] Deduplicate same notice/version
- [ ] Integration test: source -> matcher -> persistence
- [ ] Daily scheduled execution

### Acceptance Scenario

Given a daily batch of government procurement notices:

1. BidRadar scans the minimum list fields.
2. Agency and tender-title rules are evaluated.
3. Non-matches are discarded.
4. Matches are persisted.
5. A match record stores why the tender matched.
6. Duplicate notice versions are not inserted twice.

---

## Next Goals

### GOAL-002 — Lazy Search + Query Coverage

- Search historical government data on demand.
- Save only data returned by an actual user search.
- Record query coverage so cached results are never presented as falsely complete.

### GOAL-003 — Tender Detail + Version Tracking

- Fetch full detail only when required.
- Detect changed source content.
- Preserve prior versions rather than overwriting history.

### GOAL-004 — Notifications

- Web push
- Email
- Optional LINE integration
- Deduplication by user + tender + notice version
- Notification preferences for new notices / corrections / deadline changes / awards

### GOAL-005 — Web UI

- Today / matched tenders
- Search
- Watch rules
- Tender detail
- Saved tenders
