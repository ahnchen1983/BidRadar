# BidRadar Architecture

## Core Idea

BidRadar is a **demand-driven procurement intelligence system**.

It does not build a full mirror of all procurement data.

### Daily path

```text
Government daily notices
        |
        v
Daily Source Adapter
        |
        v
Minimal Notice Normalization
        |
        v
Rule Engine
   |            |
 no match      match
   |            |
 DROP           v
           Persist tender
                |
                v
           Match record
                |
                v
        Notification queue
```

### Historical search path

```text
User search
    |
    v
Query Coverage lookup
    |
    +-- covered + fresh --> local DB
    |
    +-- not covered ------> external source
                              |
                              v
                         normalize
                              |
                              v
                    save returned entities
                              |
                              v
                    save coverage metadata
```

## Components

### Rust API

- Axum HTTP API
- Tokio runtime
- SQLx for PostgreSQL
- Serde for normalized source data

### PostgreSQL

Stores only:

- daily notices that matched a watch rule;
- historical records actually discovered by a user search;
- full detail actually requested/fetched;
- source versions and verification metadata;
- query coverage;
- match and notification state.

### Source Adapters

Government source access is isolated behind an adapter boundary.

This prevents the rest of BidRadar from depending on a single endpoint, HTML shape, CSV format, or unofficial API.

The adapter exposes two conceptual operations:

1. `list_notices(date)`
2. `fetch_detail(source_tender_id)`

The V0.1 `PCC_DAILY_HTML` adapter implements the first operation against the
official announcement-date page. HTML selectors, ROC date formatting and PCC
redirect URLs remain inside the adapter; the collector only receives normalized
`TenderCandidate` values.

### Collector Pipeline

The collector loads enabled rules before contacting PCC. A run with no enabled
rules is recorded and skipped. For a normal run it:

1. fetches one requested publication date;
2. keeps opportunity-like notice sections only;
3. evaluates every candidate against enabled Watch Rules;
4. discards candidates with zero matches;
5. upserts the matched tender identity;
6. inserts a notice version idempotently;
7. inserts one match record per rule and version idempotently;
8. records run counts and failure state in `collector_runs`.

## Performance Principles

1. Never fetch external source data on every page view.
2. Never recompute expensive analytics on every request.
3. Keep list/search entities shallow.
4. Fetch detail lazily.
5. Preserve source hash and verification timestamps.
6. Use unique constraints for idempotent ingestion.
7. Measure API P95 from the beginning.

## Initial Performance Budget

| Endpoint | Target P95 |
|---|---:|
| GET /health | < 50 ms |
| GET /api/v1/watch-rules | < 100 ms |
| POST /api/v1/watch-rules | < 150 ms |
| POST /api/v1/match/preview | < 50 ms |
| Local tender list/search | < 200 ms |

External government-source latency is measured separately and must not be hidden inside local API targets.
