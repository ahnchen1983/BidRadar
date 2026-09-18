use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use serde_json::json;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::domain::{
    AgencyCondition, KeywordCondition, KeywordMode, KeywordScope, MatchMode, RuleMatch, RuleSpec,
    StoredWatchRule, TenderCandidate,
};

#[derive(Debug, Default, Clone, Copy)]
pub struct PersistOutcome {
    pub version_inserted: bool,
    pub match_records_inserted: u64,
}

#[derive(Debug, Default, Clone)]
pub struct RunCompletion {
    pub status: &'static str,
    pub scanned_count: u64,
    pub matched_count: u64,
    pub persisted_count: u64,
    pub error_count: u64,
    pub new_match_count: u64,
    pub duplicate_version_count: u64,
    pub skipped_no_rules: bool,
    pub error: Option<String>,
}

#[async_trait]
pub trait CollectorRepository: Send + Sync {
    async fn enabled_rules(&self) -> Result<Vec<StoredWatchRule>>;

    async fn start_collector_run(&self, source: &str, target_date: NaiveDate) -> Result<Uuid>;

    async fn persist_matches(
        &self,
        candidate: &TenderCandidate,
        matches: &[RuleMatch],
    ) -> Result<PersistOutcome>;

    async fn finish_collector_run(&self, run_id: Uuid, completion: &RunCompletion) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresCollectorRepository {
    pool: PgPool,
}

impl PostgresCollectorRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(FromRow)]
struct RuleRow {
    id: Uuid,
    name: String,
    match_mode: String,
}

#[derive(FromRow)]
struct KeywordRow {
    watch_rule_id: Uuid,
    scope: String,
    mode: String,
    pattern: String,
}

#[derive(FromRow)]
struct AgencyRow {
    watch_rule_id: Uuid,
    agency_code: Option<String>,
    agency_name: String,
    include_children: bool,
}

#[async_trait]
impl CollectorRepository for PostgresCollectorRepository {
    async fn enabled_rules(&self) -> Result<Vec<StoredWatchRule>> {
        let rule_rows = sqlx::query_as::<_, RuleRow>(
            r#"
            SELECT id, name, match_mode
            FROM watch_rules
            WHERE enabled = true
            ORDER BY created_at, id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to load enabled watch rules")?;

        if rule_rows.is_empty() {
            return Ok(Vec::new());
        }

        let keyword_rows = sqlx::query_as::<_, KeywordRow>(
            r#"
            SELECT keyword.watch_rule_id, keyword.scope, keyword.mode, keyword.pattern
            FROM watch_keywords AS keyword
            JOIN watch_rules AS rule ON rule.id = keyword.watch_rule_id
            WHERE rule.enabled = true
            ORDER BY keyword.created_at, keyword.id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to load watch keywords")?;

        let agency_rows = sqlx::query_as::<_, AgencyRow>(
            r#"
            SELECT agency.watch_rule_id, agency.agency_code, agency.agency_name,
                   agency.include_children
            FROM watch_agencies AS agency
            JOIN watch_rules AS rule ON rule.id = agency.watch_rule_id
            WHERE rule.enabled = true
            ORDER BY agency.created_at, agency.id
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .context("failed to load watched agencies")?;

        let mut rules = Vec::with_capacity(rule_rows.len());
        let mut indexes = HashMap::with_capacity(rule_rows.len());
        for row in rule_rows {
            let match_mode = MatchMode::from_db_str(&row.match_mode)
                .ok_or_else(|| anyhow!("invalid match mode in database: {}", row.match_mode))?;
            indexes.insert(row.id, rules.len());
            rules.push(StoredWatchRule {
                id: row.id,
                spec: RuleSpec {
                    name: row.name,
                    match_mode,
                    keywords: Vec::new(),
                    agencies: Vec::new(),
                },
            });
        }

        for row in keyword_rows {
            let Some(index) = indexes.get(&row.watch_rule_id).copied() else {
                continue;
            };
            let scope = KeywordScope::from_db_str(&row.scope)
                .ok_or_else(|| anyhow!("invalid keyword scope in database: {}", row.scope))?;
            let mode = KeywordMode::from_db_str(&row.mode)
                .ok_or_else(|| anyhow!("invalid keyword mode in database: {}", row.mode))?;
            rules[index].spec.keywords.push(KeywordCondition {
                scope,
                mode,
                pattern: row.pattern,
            });
        }

        for row in agency_rows {
            let Some(index) = indexes.get(&row.watch_rule_id).copied() else {
                continue;
            };
            rules[index].spec.agencies.push(AgencyCondition {
                agency_code: row.agency_code,
                agency_name: row.agency_name,
                include_children: row.include_children,
            });
        }

        Ok(rules)
    }

    async fn start_collector_run(&self, source: &str, target_date: NaiveDate) -> Result<Uuid> {
        sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO collector_runs (source, run_type, target_date)
            VALUES ($1, 'DAILY', $2)
            RETURNING id
            "#,
        )
        .bind(source)
        .bind(target_date)
        .fetch_one(&self.pool)
        .await
        .context("failed to create collector run")
    }

    async fn persist_matches(
        &self,
        candidate: &TenderCandidate,
        matches: &[RuleMatch],
    ) -> Result<PersistOutcome> {
        let notice_date = candidate
            .notice_date
            .ok_or_else(|| anyhow!("matched source candidate is missing notice_date"))?;
        let version_key = candidate
            .version_key
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("matched source candidate is missing version_key"))?;
        let raw_payload =
            serde_json::to_value(candidate).context("failed to serialize candidate")?;
        let canonical = serde_json::to_vec(candidate).context("failed to hash candidate")?;
        let content_hash = format!("{:x}", Sha256::digest(canonical));
        let published_at = notice_date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| anyhow!("invalid notice date"))?
            .and_utc();

        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to begin ingest transaction")?;
        let tender_id = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO tenders (
                source, source_tender_id, tender_no, title, agency_code, agency_name,
                notice_type, notice_date, source_url, current_version, source_hash,
                last_fetched_at, last_verified_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, now(), now())
            ON CONFLICT (source, source_tender_id)
            DO UPDATE SET
                last_seen_at = now(),
                last_fetched_at = now(),
                last_verified_at = now()
            RETURNING id
            "#,
        )
        .bind(&candidate.source)
        .bind(&candidate.source_tender_id)
        .bind(candidate.tender_no.as_deref())
        .bind(&candidate.title)
        .bind(candidate.agency_code.as_deref())
        .bind(&candidate.agency_name)
        .bind(candidate.notice_type.as_deref())
        .bind(notice_date)
        .bind(candidate.source_url.as_deref())
        .bind(version_key)
        .bind(&content_hash)
        .fetch_one(&mut *tx)
        .await
        .context("failed to upsert matched tender")?;

        let version_inserted = sqlx::query_scalar::<_, Uuid>(
            r#"
            INSERT INTO tender_versions (
                tender_id, version_key, notice_type, content_hash, raw_payload,
                published_at
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (tender_id, version_key) DO NOTHING
            RETURNING id
            "#,
        )
        .bind(tender_id)
        .bind(version_key)
        .bind(candidate.notice_type.as_deref())
        .bind(&content_hash)
        .bind(&raw_payload)
        .bind(published_at)
        .fetch_optional(&mut *tx)
        .await
        .context("failed to insert tender version")?
        .is_some();

        // Only a newly observed version may advance the current snapshot. This
        // prevents an overlapping yesterday/today scan from regressing a tender
        // when an older version is seen again.
        if version_inserted {
            sqlx::query(
                r#"
                UPDATE tenders
                SET tender_no = $2,
                    title = $3,
                    agency_code = COALESCE($4, agency_code),
                    agency_name = $5,
                    notice_type = $6,
                    notice_date = $7,
                    source_url = $8,
                    current_version = $9,
                    source_hash = $10,
                    updated_at = now()
                WHERE id = $1
                  AND (notice_date IS NULL OR $7 >= notice_date)
                "#,
            )
            .bind(tender_id)
            .bind(candidate.tender_no.as_deref())
            .bind(&candidate.title)
            .bind(candidate.agency_code.as_deref())
            .bind(&candidate.agency_name)
            .bind(candidate.notice_type.as_deref())
            .bind(notice_date)
            .bind(candidate.source_url.as_deref())
            .bind(version_key)
            .bind(&content_hash)
            .execute(&mut *tx)
            .await
            .context("failed to advance tender snapshot")?;
        }

        let mut match_records_inserted = 0;
        for rule_match in matches.iter().filter(|item| item.result.matched) {
            let inserted = sqlx::query_scalar::<_, Uuid>(
                r#"
                INSERT INTO tender_matches (
                    tender_id, watch_rule_id, notice_version, matched_agency,
                    matched_keywords
                )
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (tender_id, watch_rule_id, notice_version) DO NOTHING
                RETURNING id
                "#,
            )
            .bind(tender_id)
            .bind(rule_match.watch_rule_id)
            .bind(version_key)
            .bind(rule_match.result.matched_agency)
            .bind(&rule_match.result.matched_keywords)
            .fetch_optional(&mut *tx)
            .await
            .context("failed to insert tender match")?
            .is_some();
            match_records_inserted += u64::from(inserted);
        }

        tx.commit()
            .await
            .context("failed to commit ingest transaction")?;
        Ok(PersistOutcome {
            version_inserted,
            match_records_inserted,
        })
    }

    async fn finish_collector_run(&self, run_id: Uuid, completion: &RunCompletion) -> Result<()> {
        let metadata = json!({
            "new_match_count": completion.new_match_count,
            "duplicate_version_count": completion.duplicate_version_count,
            "skipped_no_rules": completion.skipped_no_rules,
            "error": completion.error,
        });

        sqlx::query(
            r#"
            UPDATE collector_runs
            SET status = $2,
                scanned_count = $3,
                matched_count = $4,
                persisted_count = $5,
                error_count = $6,
                finished_at = now(),
                metadata = $7
            WHERE id = $1
            "#,
        )
        .bind(run_id)
        .bind(completion.status)
        .bind(to_i32(completion.scanned_count))
        .bind(to_i32(completion.matched_count))
        .bind(to_i32(completion.persisted_count))
        .bind(to_i32(completion.error_count))
        .bind(metadata)
        .execute(&self.pool)
        .await
        .context("failed to finish collector run")?;

        Ok(())
    }
}

fn to_i32(value: u64) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}
