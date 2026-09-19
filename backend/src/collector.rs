use anyhow::Result;
use chrono::NaiveDate;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    domain::RuleMatch,
    repository::{CollectorRepository, RunCompletion},
    rules,
    source::TenderSource,
};

#[derive(Debug, Clone, Serialize)]
pub struct CollectorReport {
    pub run_id: Uuid,
    pub source: String,
    pub target_date: NaiveDate,
    pub scanned_count: u64,
    pub matched_count: u64,
    pub new_version_count: u64,
    pub duplicate_version_count: u64,
    pub new_match_count: u64,
    pub skipped_no_rules: bool,
}

impl CollectorReport {
    fn completion(&self, status: &'static str, error: Option<String>) -> RunCompletion {
        RunCompletion {
            status,
            scanned_count: self.scanned_count,
            matched_count: self.matched_count,
            persisted_count: self.new_version_count,
            error_count: u64::from(error.is_some()),
            new_match_count: self.new_match_count,
            duplicate_version_count: self.duplicate_version_count,
            skipped_no_rules: self.skipped_no_rules,
            error,
        }
    }
}

pub async fn collect_daily<S, R>(
    source: &S,
    repository: &R,
    target_date: NaiveDate,
) -> Result<CollectorReport>
where
    S: TenderSource + ?Sized,
    R: CollectorRepository + ?Sized,
{
    let run_id = repository
        .start_collector_run(source.source_name(), target_date)
        .await?;
    let mut report = CollectorReport {
        run_id,
        source: source.source_name().to_owned(),
        target_date,
        scanned_count: 0,
        matched_count: 0,
        new_version_count: 0,
        duplicate_version_count: 0,
        new_match_count: 0,
        skipped_no_rules: false,
    };

    let run_result: Result<()> = async {
        let watch_rules = repository.enabled_rules().await?;
        if watch_rules.is_empty() {
            report.skipped_no_rules = true;
            return Ok(());
        }

        let candidates = source.list_notices(target_date).await?;
        report.scanned_count = candidates.len() as u64;

        for candidate in candidates {
            let matches: Vec<RuleMatch> = watch_rules
                .iter()
                .filter_map(|watch_rule| {
                    let result = rules::evaluate(&watch_rule.spec, &candidate);
                    result.matched.then_some(RuleMatch {
                        watch_rule_id: watch_rule.id,
                        result,
                    })
                })
                .collect();

            if matches.is_empty() {
                continue;
            }

            report.matched_count += 1;
            let outcome = repository.persist_matches(&candidate, &matches).await?;
            if outcome.version_inserted {
                report.new_version_count += 1;
            } else {
                report.duplicate_version_count += 1;
            }
            report.new_match_count += outcome.match_records_inserted;
        }

        Ok(())
    }
    .await;

    match run_result {
        Ok(()) => {
            repository
                .finish_collector_run(run_id, &report.completion("SUCCESS", None))
                .await?;
            Ok(report)
        }
        Err(error) => {
            let status = if report.new_version_count > 0 {
                "PARTIAL"
            } else {
                "FAILED"
            };
            let message = truncate_error(&error.to_string());
            if let Err(finish_error) = repository
                .finish_collector_run(run_id, &report.completion(status, Some(message)))
                .await
            {
                tracing::error!(%finish_error, %run_id, "failed to record collector failure");
            }
            Err(error)
        }
    }
}

fn truncate_error(value: &str) -> String {
    value.chars().take(2_000).collect()
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, sync::Arc};

    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::NaiveDate;
    use tokio::sync::Mutex;

    use super::*;
    use crate::{
        domain::{
            KeywordCondition, KeywordMode, KeywordScope, MatchMode, RuleSpec, StoredWatchRule,
            TenderCandidate,
        },
        repository::{PersistOutcome, RunCompletion},
    };

    struct MockSource {
        candidates: Vec<TenderCandidate>,
    }

    #[async_trait]
    impl TenderSource for MockSource {
        fn source_name(&self) -> &'static str {
            "MOCK"
        }

        async fn list_notices(&self, _date: NaiveDate) -> Result<Vec<TenderCandidate>> {
            Ok(self.candidates.clone())
        }
    }

    #[derive(Default)]
    struct MemoryState {
        versions: HashSet<(String, String)>,
        matches: HashSet<(String, String, Uuid)>,
        persisted_source_ids: Vec<String>,
        completions: Vec<RunCompletion>,
    }

    struct MemoryRepository {
        rules: Vec<StoredWatchRule>,
        state: Arc<Mutex<MemoryState>>,
    }

    #[async_trait]
    impl CollectorRepository for MemoryRepository {
        async fn enabled_rules(&self) -> Result<Vec<StoredWatchRule>> {
            Ok(self.rules.clone())
        }

        async fn start_collector_run(
            &self,
            _source: &str,
            _target_date: NaiveDate,
        ) -> Result<Uuid> {
            Ok(Uuid::new_v4())
        }

        async fn persist_matches(
            &self,
            candidate: &TenderCandidate,
            matches: &[RuleMatch],
        ) -> Result<PersistOutcome> {
            let version_key = candidate.version_key.clone().unwrap();
            let mut state = self.state.lock().await;
            state
                .persisted_source_ids
                .push(candidate.source_tender_id.clone());
            let version_inserted = state
                .versions
                .insert((candidate.source_tender_id.clone(), version_key.clone()));
            let mut match_records_inserted = 0;
            for rule_match in matches {
                if state.matches.insert((
                    candidate.source_tender_id.clone(),
                    version_key.clone(),
                    rule_match.watch_rule_id,
                )) {
                    match_records_inserted += 1;
                }
            }
            Ok(PersistOutcome {
                version_inserted,
                match_records_inserted,
            })
        }

        async fn finish_collector_run(
            &self,
            _run_id: Uuid,
            completion: &RunCompletion,
        ) -> Result<()> {
            self.state.lock().await.completions.push(completion.clone());
            Ok(())
        }
    }

    fn candidate(id: &str, title: &str) -> TenderCandidate {
        TenderCandidate {
            source: "MOCK".into(),
            source_tender_id: id.into(),
            tender_no: Some(id.into()),
            title: title.into(),
            agency_code: None,
            agency_name: "測試機關".into(),
            notice_type: Some("公開招標公告".into()),
            notice_date: NaiveDate::from_ymd_opt(2026, 9, 18),
            version_key: Some(format!("{id}-v1")),
            source_url: None,
        }
    }

    fn ai_rule() -> StoredWatchRule {
        StoredWatchRule {
            id: Uuid::new_v4(),
            spec: RuleSpec {
                name: "AI".into(),
                match_mode: MatchMode::Any,
                keywords: vec![KeywordCondition {
                    scope: KeywordScope::Tender,
                    mode: KeywordMode::Include,
                    pattern: "AI".into(),
                }],
                agencies: vec![],
            },
        }
    }

    #[tokio::test]
    async fn persists_only_matches_and_is_idempotent_by_version() {
        let source = MockSource {
            candidates: vec![
                candidate("ai-001", "AI智慧客服建置"),
                candidate("road-001", "道路改善工程"),
            ],
        };
        let state = Arc::new(Mutex::new(MemoryState::default()));
        let repository = MemoryRepository {
            rules: vec![ai_rule()],
            state: state.clone(),
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();

        let first = collect_daily(&source, &repository, date).await.unwrap();
        assert_eq!(first.scanned_count, 2);
        assert_eq!(first.matched_count, 1);
        assert_eq!(first.new_version_count, 1);
        assert_eq!(first.new_match_count, 1);
        assert_eq!(state.lock().await.persisted_source_ids, vec!["ai-001"]);

        let second = collect_daily(&source, &repository, date).await.unwrap();
        assert_eq!(second.new_version_count, 0);
        assert_eq!(second.duplicate_version_count, 1);
        assert_eq!(second.new_match_count, 0);
    }

    #[tokio::test]
    async fn skips_the_upstream_source_when_no_rules_are_enabled() {
        let source = MockSource {
            candidates: vec![candidate("ai-001", "AI智慧客服建置")],
        };
        let state = Arc::new(Mutex::new(MemoryState::default()));
        let repository = MemoryRepository {
            rules: vec![],
            state,
        };
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();

        let report = collect_daily(&source, &repository, date).await.unwrap();
        assert!(report.skipped_no_rules);
        assert_eq!(report.scanned_count, 0);
    }

    #[sqlx::test(migrations = "../migrations")]
    #[ignore = "requires a PostgreSQL server"]
    async fn postgres_pipeline_persists_only_matches_and_deduplicates_versions(pool: sqlx::PgPool) {
        let rule = ai_rule();
        sqlx::query(
            r#"
            INSERT INTO watch_rules (id, name, match_mode, enabled)
            VALUES ($1, $2, 'ANY', true)
            "#,
        )
        .bind(rule.id)
        .bind(&rule.spec.name)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO watch_keywords (watch_rule_id, scope, mode, pattern)
            VALUES ($1, 'TENDER', 'INCLUDE', 'AI')
            "#,
        )
        .bind(rule.id)
        .execute(&pool)
        .await
        .unwrap();

        let source = MockSource {
            candidates: vec![
                candidate("ai-001", "AI智慧客服建置"),
                candidate("road-001", "道路改善工程"),
            ],
        };
        let repository = crate::repository::PostgresCollectorRepository::new(pool.clone());
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();

        let first = collect_daily(&source, &repository, date).await.unwrap();
        assert_eq!(first.scanned_count, 2);
        assert_eq!(first.matched_count, 1);
        assert_eq!(first.new_version_count, 1);
        assert_eq!(first.new_match_count, 1);

        let second = collect_daily(&source, &repository, date).await.unwrap();
        assert_eq!(second.new_version_count, 0);
        assert_eq!(second.duplicate_version_count, 1);
        assert_eq!(second.new_match_count, 0);

        let tender_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenders")
            .fetch_one(&pool)
            .await
            .unwrap();
        let version_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tender_versions")
            .fetch_one(&pool)
            .await
            .unwrap();
        let match_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tender_matches")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tender_count, 1);
        assert_eq!(version_count, 1);
        assert_eq!(match_count, 1);
    }
}
