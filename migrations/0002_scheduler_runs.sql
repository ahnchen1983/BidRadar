CREATE TABLE scheduler_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    scheduler_name text NOT NULL,
    scheduled_for timestamptz NOT NULL,
    timezone text NOT NULL,
    status text NOT NULL DEFAULT 'RUNNING'
        CHECK (status IN ('RUNNING', 'SUCCESS', 'PARTIAL', 'FAILED')),
    target_dates date[] NOT NULL DEFAULT '{}',
    reports jsonb NOT NULL DEFAULT '[]'::jsonb,
    errors jsonb NOT NULL DEFAULT '[]'::jsonb,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    UNIQUE (scheduler_name, scheduled_for)
);

CREATE INDEX scheduler_runs_status_idx
    ON scheduler_runs (status, scheduled_for DESC);
