# Daily Scheduler

## Default Schedule

BidRadar's Docker Compose configuration enables the scheduler at these local
times:

```text
Timezone: Asia/Taipei
Times:    06:00, 12:00, 18:00, 23:00
Lookback: 1 day
Catch-up: enabled
```

Each slot scans the previous local date first and the current local date second.
Processing oldest-to-newest prevents a recovery scan from leaving an older
notice as the final snapshot.

## Configuration

| Variable | Default in application | Purpose |
|---|---|---|
| `DAILY_SCHEDULER_ENABLED` | `false` | Starts the in-process scheduler. Docker Compose sets it to `true`. |
| `DAILY_SCHEDULER_TIMEZONE` | `Asia/Taipei` | IANA timezone used to interpret configured times. |
| `DAILY_SCHEDULER_TIMES` | `06:00,12:00,18:00,23:00` | Comma-separated local `HH:MM` slots. |
| `DAILY_SCHEDULER_LOOKBACK_DAYS` | `1` | Additional prior local dates to scan; maximum `30`. |
| `DAILY_SCHEDULER_CATCH_UP_ON_STARTUP` | `true` | Attempts the most recent configured slot on startup. |

Invalid timezone, time, boolean or lookback settings stop application startup
with an explicit configuration error.

## Cross-instance Deduplication

Every scheduled execution first inserts a row identified by:

```text
scheduler_name + scheduled_for
```

The database unique constraint is the claim. If another API instance already
inserted the row, the later instance skips that slot. This remains effective
after the first instance finishes, unlike a short-lived process lock.

Manual `POST /api/v1/collect/daily` runs do not use a scheduler claim, but their
tender-version and rule-match writes remain idempotent.

## Recovery and Failure States

The scheduler stores one of:

- `SUCCESS` — all target dates completed;
- `PARTIAL` — at least one target date completed and at least one failed;
- `FAILED` — every target date failed;
- `RUNNING` — claimed but not yet finalized, including a process that stopped
  unexpectedly during the slot.

Each date also creates its own `collector_runs` entry. Source or persistence
errors are truncated before storage, retained per date, and do not terminate the
background scheduling loop.

The next configured slot again scans today and yesterday, providing automatic
recovery without retry storms against the government source.
