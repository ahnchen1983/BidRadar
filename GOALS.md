# BidRadar Goals

## Current Goal

### GOAL-001 — Data Pipeline Prototype

**Status:** COMPLETED

### Definition of Done

- [x] Architecture decision: Rust + PostgreSQL
- [x] Demand-driven data strategy
- [x] Watch Rule model: Agency + Tender Keyword
- [x] Rust workspace
- [x] PostgreSQL schema / migrations
- [x] Axum API skeleton
- [x] Rule Engine preview endpoint
- [x] Rule Engine unit tests
- [x] PCC daily source adapter
- [x] Fetch notices for a specified date
- [x] Persist matched tenders only
- [x] Deduplicate same notice/version
- [x] Integration test: source -> matcher -> persistence
- [x] Daily scheduled execution

### Acceptance Scenario

Given a daily batch of government procurement notices:

1. BidRadar scans the minimum list fields.
2. Agency and tender-title rules are evaluated.
3. Non-matches are discarded.
4. Matches are persisted.
5. A match record stores why the tender matched.
6. Duplicate notice versions are not inserted twice.

### GOAL-001B Checkpoint

`POST /api/v1/collect/daily` now executes the complete manual path:

```text
date -> PCC daily gazette -> opportunity filter -> Watch Rules
     -> matched tenders only -> tender_versions -> tender_matches
```

The collector is idempotent by stable tender identity plus PCC notice-version key.
If no Watch Rule is enabled, it records a successful skipped run without calling
the upstream source.

### GOAL-001C Checkpoint

The production scheduler now:

- runs at `06:00`, `12:00`, `18:00` and `23:00` in `Asia/Taipei` by default;
- scans the scheduled local date plus the previous day;
- catches up the latest missed slot when the service starts;
- uses a PostgreSQL unique slot claim so multiple API instances cannot execute
  the same scheduled slot twice;
- records `SUCCESS`, `PARTIAL` or `FAILED` with per-date reports and errors;
- continues to later dates and future slots after a source failure.

The schedule, timezone, lookback window and startup catch-up behavior are all
environment-configurable.

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
