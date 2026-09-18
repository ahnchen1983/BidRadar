CREATE EXTENSION IF NOT EXISTS pgcrypto;

CREATE TABLE watch_rules (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id uuid,
    name text NOT NULL,
    match_mode text NOT NULL CHECK (match_mode IN ('ANY', 'ALL')),
    enabled boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE watch_keywords (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    watch_rule_id uuid NOT NULL REFERENCES watch_rules(id) ON DELETE CASCADE,
    scope text NOT NULL CHECK (scope IN ('AGENCY', 'TENDER')),
    mode text NOT NULL CHECK (mode IN ('INCLUDE', 'EXCLUDE')),
    pattern text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX watch_keywords_rule_idx ON watch_keywords (watch_rule_id);

CREATE TABLE watch_agencies (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    watch_rule_id uuid NOT NULL REFERENCES watch_rules(id) ON DELETE CASCADE,
    agency_code text,
    agency_name text NOT NULL,
    include_children boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX watch_agencies_rule_idx ON watch_agencies (watch_rule_id);
CREATE INDEX watch_agencies_code_idx ON watch_agencies (agency_code) WHERE agency_code IS NOT NULL;

CREATE TABLE tenders (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source text NOT NULL,
    source_tender_id text NOT NULL,
    tender_no text,
    title text NOT NULL,
    agency_code text,
    agency_name text NOT NULL,
    notice_type text,
    notice_date date,
    deadline_at timestamptz,
    procurement_type text,
    budget numeric(18, 2),
    source_url text,
    current_version text,
    detail_status text NOT NULL DEFAULT 'INDEX_ONLY'
        CHECK (detail_status IN ('INDEX_ONLY', 'FULL', 'STALE', 'FETCH_ERROR')),
    source_hash text,
    first_seen_at timestamptz NOT NULL DEFAULT now(),
    last_seen_at timestamptz NOT NULL DEFAULT now(),
    last_fetched_at timestamptz,
    last_verified_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (source, source_tender_id)
);

CREATE INDEX tenders_notice_date_idx ON tenders (notice_date DESC);
CREATE INDEX tenders_agency_code_idx ON tenders (agency_code);
CREATE INDEX tenders_tender_no_idx ON tenders (tender_no);

CREATE TABLE tender_details (
    tender_id uuid PRIMARY KEY REFERENCES tenders(id) ON DELETE CASCADE,
    procurement_method text,
    award_method text,
    qualification text,
    description text,
    performance_location text,
    performance_period text,
    contact_name text,
    contact_phone text,
    contact_email text,
    opening_at timestamptz,
    opening_location text,
    raw_payload jsonb,
    parsed_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE tender_versions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id uuid NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    version_key text NOT NULL,
    notice_type text,
    content_hash text,
    raw_payload jsonb,
    published_at timestamptz,
    fetched_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tender_id, version_key)
);

CREATE INDEX tender_versions_tender_idx
    ON tender_versions (tender_id, fetched_at DESC);

CREATE TABLE tender_matches (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    tender_id uuid NOT NULL REFERENCES tenders(id) ON DELETE CASCADE,
    watch_rule_id uuid NOT NULL REFERENCES watch_rules(id) ON DELETE CASCADE,
    notice_version text NOT NULL DEFAULT '',
    matched_agency boolean NOT NULL DEFAULT false,
    matched_keywords text[] NOT NULL DEFAULT '{}',
    matched_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (tender_id, watch_rule_id, notice_version)
);

CREATE TABLE notifications (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    match_id uuid NOT NULL REFERENCES tender_matches(id) ON DELETE CASCADE,
    channel text NOT NULL,
    status text NOT NULL DEFAULT 'PENDING'
        CHECK (status IN ('PENDING', 'SENT', 'FAILED', 'SKIPPED')),
    attempts integer NOT NULL DEFAULT 0,
    last_error text,
    created_at timestamptz NOT NULL DEFAULT now(),
    sent_at timestamptz,
    UNIQUE (match_id, channel)
);

CREATE INDEX notifications_pending_idx
    ON notifications (created_at)
    WHERE status = 'PENDING';

CREATE TABLE query_coverage (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    query_hash text NOT NULL,
    source text NOT NULL,
    normalized_query jsonb NOT NULL,
    date_from date,
    date_to date,
    searched_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz,
    page_count integer,
    result_count integer,
    completed boolean NOT NULL DEFAULT false,
    UNIQUE (query_hash, source, date_from, date_to)
);

CREATE INDEX query_coverage_lookup_idx
    ON query_coverage (query_hash, source, searched_at DESC);

CREATE TABLE collector_runs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    source text NOT NULL,
    run_type text NOT NULL,
    target_date date,
    status text NOT NULL DEFAULT 'RUNNING'
        CHECK (status IN ('RUNNING', 'SUCCESS', 'PARTIAL', 'FAILED')),
    scanned_count integer NOT NULL DEFAULT 0,
    matched_count integer NOT NULL DEFAULT 0,
    persisted_count integer NOT NULL DEFAULT 0,
    error_count integer NOT NULL DEFAULT 0,
    started_at timestamptz NOT NULL DEFAULT now(),
    finished_at timestamptz,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);
