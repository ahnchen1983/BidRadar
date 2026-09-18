# Data Strategy

## 1. Daily Scan

BidRadar scans new notices broadly but persists selectively.

The first-stage scanner should fetch only fields needed to identify and classify a notice:

- source tender ID / notice ID
- tender number
- tender title
- agency name
- agency code when available
- notice type
- notice date
- deadline when available
- budget when available
- source URL
- source version identifier when available

A notice that does not match any enabled watch rule is discarded after evaluation.

## 2. Watch Rule Types

Two keyword scopes are supported:

### AGENCY

Matches the publishing/procuring agency name.

Examples:

- 嘉義縣政府
- 雲林縣政府
- 國家發展委員會

Canonical agency subscriptions may additionally use agency code and an optional include-children flag.

### TENDER

Matches the tender title.

Examples:

- AI
- 人工智慧
- 數位轉型
- 地方創生
- 品牌設計

## 3. Include / Exclude

Every rule may contain:

- INCLUDE conditions
- EXCLUDE conditions

An EXCLUDE match vetoes the rule.

The positive conditions are evaluated according to:

- ANY — at least one positive condition matches
- ALL — every positive condition matches

## 4. Lazy Historical Data

Historical data is not proactively mirrored.

When a user searches a range for the first time:

1. Check query coverage.
2. Query the external source only for uncovered/stale coverage.
3. Normalize returned records.
4. Save the records actually returned.
5. Save coverage metadata including completion state and result count.

## 5. Detail Fetching

A lightweight tender row may exist without full detail.

Detail states:

- INDEX_ONLY
- FULL
- STALE
- FETCH_ERROR

Full detail is fetched only when needed.

## 6. Versioning

Source changes must not silently overwrite historical truth.

Each source version stores:

- tender ID
- source version key
- notice type
- content hash
- raw payload when legally/technically appropriate
- published timestamp
- fetched timestamp

## 7. Source of Truth

The government source remains the authoritative source.

BidRadar records:

- first_seen_at
- last_seen_at
- last_fetched_at
- last_verified_at
- source_hash

This makes freshness visible and supports revalidation without blocking normal reads.
