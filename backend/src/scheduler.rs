use std::{env, time::Duration as StdDuration};

use anyhow::{anyhow, bail, Context, Result};
use chrono::{DateTime, Days, LocalResult, NaiveDate, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use serde::Serialize;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    collector::{self, CollectorReport},
    repository::PostgresCollectorRepository,
    source::TenderSource,
};

const SCHEDULER_NAME: &str = "PCC_DAILY";
const DEFAULT_TIMEZONE: &str = "Asia/Taipei";
const DEFAULT_TIMES: &str = "06:00,12:00,18:00,23:00";

#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub enabled: bool,
    pub timezone: Tz,
    pub times: Vec<NaiveTime>,
    pub lookback_days: u64,
    pub catch_up_on_startup: bool,
}

impl SchedulerConfig {
    pub fn from_env() -> Result<Self> {
        let enabled = parse_bool_env("DAILY_SCHEDULER_ENABLED", false)?;
        let catch_up_on_startup = parse_bool_env("DAILY_SCHEDULER_CATCH_UP_ON_STARTUP", true)?;
        let timezone_name =
            env::var("DAILY_SCHEDULER_TIMEZONE").unwrap_or_else(|_| DEFAULT_TIMEZONE.to_owned());
        let timezone = timezone_name
            .parse::<Tz>()
            .with_context(|| format!("invalid DAILY_SCHEDULER_TIMEZONE: {timezone_name}"))?;
        let times_value =
            env::var("DAILY_SCHEDULER_TIMES").unwrap_or_else(|_| DEFAULT_TIMES.to_owned());
        let mut times = parse_times(&times_value)?;
        times.sort_unstable();
        times.dedup();

        let lookback_days = env::var("DAILY_SCHEDULER_LOOKBACK_DAYS")
            .unwrap_or_else(|_| "1".to_owned())
            .parse::<u64>()
            .context("DAILY_SCHEDULER_LOOKBACK_DAYS must be a non-negative integer")?;
        if lookback_days > 30 {
            bail!("DAILY_SCHEDULER_LOOKBACK_DAYS must not exceed 30");
        }

        Ok(Self {
            enabled,
            timezone,
            times,
            lookback_days,
            catch_up_on_startup,
        })
    }
}

#[derive(Debug)]
pub enum SlotRunOutcome {
    AlreadyClaimed,
    Completed {
        status: &'static str,
        reports: Vec<CollectorReport>,
        errors: Vec<ScheduledDateError>,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledDateError {
    pub date: NaiveDate,
    pub error: String,
}

pub async fn run<S>(pool: PgPool, source: S, config: SchedulerConfig) -> Result<()>
where
    S: TenderSource + 'static,
{
    if !config.enabled {
        tracing::info!("daily scheduler disabled");
        return Ok(());
    }

    tracing::info!(
        timezone = %config.timezone,
        times = ?config.times,
        lookback_days = config.lookback_days,
        catch_up_on_startup = config.catch_up_on_startup,
        "daily scheduler started"
    );

    if config.catch_up_on_startup {
        let catch_up_slot = previous_slot_at_or_before(Utc::now(), &config)?;
        execute_and_log(&pool, &source, &config, catch_up_slot).await;
    }

    loop {
        let next_slot = next_slot_after(Utc::now(), &config)?;
        let wait = (next_slot - Utc::now())
            .to_std()
            .unwrap_or(StdDuration::ZERO);
        tracing::info!(scheduled_for = %next_slot, wait_seconds = wait.as_secs(), "next daily scan scheduled");
        tokio::time::sleep(wait).await;
        execute_and_log(&pool, &source, &config, next_slot).await;
    }
}

async fn execute_and_log<S>(
    pool: &PgPool,
    source: &S,
    config: &SchedulerConfig,
    scheduled_for: DateTime<Utc>,
) where
    S: TenderSource + ?Sized,
{
    match run_scheduled_slot(pool, source, config, scheduled_for).await {
        Ok(SlotRunOutcome::AlreadyClaimed) => {
            tracing::info!(%scheduled_for, "scheduled slot already claimed by another instance");
        }
        Ok(SlotRunOutcome::Completed {
            status,
            reports,
            errors,
        }) => {
            tracing::info!(
                %scheduled_for,
                status,
                completed_dates = reports.len(),
                failed_dates = errors.len(),
                "scheduled daily scan finished"
            );
        }
        Err(error) => {
            tracing::error!(%error, %scheduled_for, "scheduled daily scan could not run");
        }
    }
}

pub async fn run_scheduled_slot<S>(
    pool: &PgPool,
    source: &S,
    config: &SchedulerConfig,
    scheduled_for: DateTime<Utc>,
) -> Result<SlotRunOutcome>
where
    S: TenderSource + ?Sized,
{
    let target_dates = target_dates(scheduled_for, config)?;
    let run_id = claim_slot(pool, scheduled_for, config.timezone.name(), &target_dates).await?;
    let Some(run_id) = run_id else {
        return Ok(SlotRunOutcome::AlreadyClaimed);
    };

    let repository = PostgresCollectorRepository::new(pool.clone());
    let mut reports = Vec::with_capacity(target_dates.len());
    let mut errors = Vec::new();

    for target_date in target_dates {
        match collector::collect_daily(source, &repository, target_date).await {
            Ok(report) => reports.push(report),
            Err(error) => errors.push(ScheduledDateError {
                date: target_date,
                error: truncate_error(&error.to_string()),
            }),
        }
    }

    let status = if errors.is_empty() {
        "SUCCESS"
    } else if reports.is_empty() {
        "FAILED"
    } else {
        "PARTIAL"
    };
    finish_slot(pool, run_id, status, &reports, &errors).await?;

    Ok(SlotRunOutcome::Completed {
        status,
        reports,
        errors,
    })
}

async fn claim_slot(
    pool: &PgPool,
    scheduled_for: DateTime<Utc>,
    timezone: &str,
    target_dates: &[NaiveDate],
) -> Result<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        r#"
        INSERT INTO scheduler_runs (
            scheduler_name, scheduled_for, timezone, target_dates
        )
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (scheduler_name, scheduled_for) DO NOTHING
        RETURNING id
        "#,
    )
    .bind(SCHEDULER_NAME)
    .bind(scheduled_for)
    .bind(timezone)
    .bind(target_dates)
    .fetch_optional(pool)
    .await
    .context("failed to claim scheduler slot")
}

async fn finish_slot(
    pool: &PgPool,
    run_id: Uuid,
    status: &str,
    reports: &[CollectorReport],
    errors: &[ScheduledDateError],
) -> Result<()> {
    let reports = serde_json::to_value(reports).context("failed to serialize scheduler reports")?;
    let errors = serde_json::to_value(errors).context("failed to serialize scheduler errors")?;
    let metadata = json!({
        "completed_dates": reports.as_array().map_or(0, Vec::len),
        "failed_dates": errors.as_array().map_or(0, Vec::len),
    });

    sqlx::query(
        r#"
        UPDATE scheduler_runs
        SET status = $2,
            reports = $3,
            errors = $4,
            finished_at = now()
        WHERE id = $1
        "#,
    )
    .bind(run_id)
    .bind(status)
    .bind(reports)
    .bind(errors)
    .execute(pool)
    .await
    .with_context(|| format!("failed to finish scheduler run: {metadata}"))?;

    Ok(())
}

fn parse_bool_env(key: &str, default: bool) -> Result<bool> {
    let Ok(value) = env::var(key) else {
        return Ok(default);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(anyhow!("{key} must be true or false")),
    }
}

fn parse_times(value: &str) -> Result<Vec<NaiveTime>> {
    let times: Vec<NaiveTime> = value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            NaiveTime::parse_from_str(item, "%H:%M")
                .with_context(|| format!("invalid DAILY_SCHEDULER_TIMES value: {item}"))
        })
        .collect::<Result<_>>()?;
    if times.is_empty() {
        bail!("DAILY_SCHEDULER_TIMES must contain at least one HH:MM value");
    }
    Ok(times)
}

fn target_dates(scheduled_for: DateTime<Utc>, config: &SchedulerConfig) -> Result<Vec<NaiveDate>> {
    let local_date = scheduled_for.with_timezone(&config.timezone).date_naive();
    (0..=config.lookback_days)
        .rev()
        .map(|days_ago| {
            local_date
                .checked_sub_days(Days::new(days_ago))
                .ok_or_else(|| anyhow!("target date underflow"))
        })
        .collect()
}

fn next_slot_after(now: DateTime<Utc>, config: &SchedulerConfig) -> Result<DateTime<Utc>> {
    let local_date = now.with_timezone(&config.timezone).date_naive();
    for day_offset in 0..=2 {
        let date = local_date
            .checked_add_days(Days::new(day_offset))
            .ok_or_else(|| anyhow!("scheduler date overflow"))?;
        for time in &config.times {
            if let Some(candidate) = local_to_utc(config.timezone, date, *time) {
                if candidate > now {
                    return Ok(candidate);
                }
            }
        }
    }
    bail!("could not calculate the next scheduler slot")
}

fn previous_slot_at_or_before(
    now: DateTime<Utc>,
    config: &SchedulerConfig,
) -> Result<DateTime<Utc>> {
    let local_date = now.with_timezone(&config.timezone).date_naive();
    for day_offset in 0..=2 {
        let date = local_date
            .checked_sub_days(Days::new(day_offset))
            .ok_or_else(|| anyhow!("scheduler date underflow"))?;
        for time in config.times.iter().rev() {
            if let Some(candidate) = local_to_utc(config.timezone, date, *time) {
                if candidate <= now {
                    return Ok(candidate);
                }
            }
        }
    }
    bail!("could not calculate the previous scheduler slot")
}

fn local_to_utc(timezone: Tz, date: NaiveDate, time: NaiveTime) -> Option<DateTime<Utc>> {
    let local = date.and_time(time);
    match timezone.from_local_datetime(&local) {
        LocalResult::Single(value) => Some(value.with_timezone(&Utc)),
        LocalResult::Ambiguous(first, second) => Some(first.min(second).with_timezone(&Utc)),
        LocalResult::None => None,
    }
}

fn truncate_error(value: &str) -> String {
    value.chars().take(2_000).collect()
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use async_trait::async_trait;
    use chrono::{NaiveDate, TimeZone};

    use super::*;
    use crate::{domain::TenderCandidate, source::TenderSource};

    fn config(lookback_days: u64) -> SchedulerConfig {
        SchedulerConfig {
            enabled: true,
            timezone: chrono_tz::Asia::Taipei,
            times: parse_times(DEFAULT_TIMES).unwrap(),
            lookback_days,
            catch_up_on_startup: true,
        }
    }

    #[test]
    fn calculates_taipei_schedule_in_utc() {
        let config = config(1);
        let now = Utc.with_ymd_and_hms(2026, 9, 19, 3, 0, 0).unwrap();

        assert_eq!(
            next_slot_after(now, &config).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 19, 4, 0, 0).unwrap()
        );
        assert_eq!(
            previous_slot_at_or_before(now, &config).unwrap(),
            Utc.with_ymd_and_hms(2026, 9, 18, 22, 0, 0).unwrap()
        );
    }

    #[test]
    fn builds_oldest_to_newest_lookback_dates() {
        let config = config(1);
        let scheduled_for = Utc.with_ymd_and_hms(2026, 9, 18, 22, 0, 0).unwrap();

        assert_eq!(
            target_dates(scheduled_for, &config).unwrap(),
            vec![
                NaiveDate::from_ymd_opt(2026, 9, 18).unwrap(),
                NaiveDate::from_ymd_opt(2026, 9, 19).unwrap(),
            ]
        );
    }

    struct MockSource;

    #[async_trait]
    impl TenderSource for MockSource {
        fn source_name(&self) -> &'static str {
            "SCHEDULER_MOCK"
        }

        async fn list_notices(&self, date: NaiveDate) -> Result<Vec<TenderCandidate>> {
            Ok(vec![TenderCandidate {
                source: self.source_name().into(),
                source_tender_id: "ai-001".into(),
                tender_no: Some("AI-001".into()),
                title: "AI智慧客服建置".into(),
                agency_code: None,
                agency_name: "測試機關".into(),
                notice_type: Some("公開招標公告".into()),
                notice_date: Some(date),
                version_key: Some(format!("ai-001-{date}")),
                source_url: None,
            }])
        }
    }

    #[sqlx::test(migrations = "../migrations")]
    #[ignore = "requires a PostgreSQL server"]
    async fn postgres_scheduler_claims_each_slot_once(pool: PgPool) {
        let rule_id = Uuid::new_v4();
        sqlx::query(
            r#"
            INSERT INTO watch_rules (id, name, match_mode, enabled)
            VALUES ($1, 'AI', 'ANY', true)
            "#,
        )
        .bind(rule_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO watch_keywords (watch_rule_id, scope, mode, pattern)
            VALUES ($1, 'TENDER', 'INCLUDE', 'AI')
            "#,
        )
        .bind(rule_id)
        .execute(&pool)
        .await
        .unwrap();

        let source = MockSource;
        let config = config(0);
        let scheduled_for = Utc.with_ymd_and_hms(2026, 9, 18, 22, 0, 0).unwrap();

        let first = run_scheduled_slot(&pool, &source, &config, scheduled_for)
            .await
            .unwrap();
        assert!(matches!(
            first,
            SlotRunOutcome::Completed {
                status: "SUCCESS",
                ..
            }
        ));
        assert!(matches!(
            run_scheduled_slot(&pool, &source, &config, scheduled_for)
                .await
                .unwrap(),
            SlotRunOutcome::AlreadyClaimed
        ));

        let scheduler_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM scheduler_runs")
            .fetch_one(&pool)
            .await
            .unwrap();
        let collector_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collector_runs")
            .fetch_one(&pool)
            .await
            .unwrap();
        let tender_count = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM tenders")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(scheduler_count, 1);
        assert_eq!(collector_count, 1);
        assert_eq!(tender_count, 1);
    }
}
