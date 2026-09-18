mod collector;
mod domain;
mod repository;
mod rules;
mod source;

use std::{env, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::NaiveDate;
use domain::{CreateWatchRule, RuleSpec, TenderCandidate, WatchRuleSummary};
use repository::PostgresCollectorRepository;
use serde::{Deserialize, Serialize};
use source::pcc::PccDailySource;
use sqlx::{postgres::PgPoolOptions, PgPool};
use uuid::Uuid;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    pcc_source: PccDailySource,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bidradar_api=info,tower_http=info".into()),
        )
        .init();

    let database_url = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://bidradar:bidradar@localhost:5432/bidradar".into());
    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let pcc_base_url = env::var("PCC_BASE_URL").ok();
    let pcc_timeout_seconds = env::var("PCC_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(60);

    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    sqlx::migrate!("../migrations").run(&db).await?;

    let pcc_source = PccDailySource::new(
        pcc_base_url.as_deref(),
        Duration::from_secs(pcc_timeout_seconds),
    )?;
    let state = Arc::new(AppState { db, pcc_source });

    let app = Router::new()
        .route("/health", get(health))
        .route(
            "/api/v1/watch-rules",
            get(list_watch_rules).post(create_watch_rule),
        )
        .route("/api/v1/match/preview", post(preview_match))
        .route("/api/v1/collect/daily", post(run_daily_collector))
        .with_state(state);

    let addr: SocketAddr = bind_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(%addr, "BidRadar API listening");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "bidradar-api",
    })
}

async fn list_watch_rules(
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WatchRuleSummary>>, ApiError> {
    let rows = sqlx::query_as::<_, WatchRuleSummary>(
        r#"
        SELECT id, name, match_mode, enabled, created_at
        FROM watch_rules
        ORDER BY created_at DESC
        "#,
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(rows))
}

async fn create_watch_rule(
    State(state): State<Arc<AppState>>,
    Json(input): Json<CreateWatchRule>,
) -> Result<(StatusCode, Json<WatchRuleSummary>), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError::BadRequest("name must not be empty".into()));
    }

    if input.keywords.is_empty() && input.agencies.is_empty() {
        return Err(ApiError::BadRequest(
            "at least one agency or keyword condition is required".into(),
        ));
    }

    let mut tx = state.db.begin().await?;
    let rule_id = Uuid::new_v4();

    let rule = sqlx::query_as::<_, WatchRuleSummary>(
        r#"
        INSERT INTO watch_rules (id, name, match_mode, enabled)
        VALUES ($1, $2, $3, $4)
        RETURNING id, name, match_mode, enabled, created_at
        "#,
    )
    .bind(rule_id)
    .bind(input.name.trim())
    .bind(input.match_mode.as_db_str())
    .bind(input.enabled)
    .fetch_one(&mut *tx)
    .await?;

    for keyword in input.keywords {
        if keyword.pattern.trim().is_empty() {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO watch_keywords (watch_rule_id, scope, mode, pattern)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(rule_id)
        .bind(keyword.scope.as_db_str())
        .bind(keyword.mode.as_db_str())
        .bind(keyword.pattern.trim())
        .execute(&mut *tx)
        .await?;
    }

    for agency in input.agencies {
        if agency.agency_name.trim().is_empty() {
            continue;
        }

        sqlx::query(
            r#"
            INSERT INTO watch_agencies
                (watch_rule_id, agency_code, agency_name, include_children)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(rule_id)
        .bind(agency.agency_code)
        .bind(agency.agency_name.trim())
        .bind(agency.include_children)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok((StatusCode::CREATED, Json(rule)))
}

#[derive(serde::Deserialize)]
struct PreviewRequest {
    rule: RuleSpec,
    tender: TenderCandidate,
}

async fn preview_match(Json(input): Json<PreviewRequest>) -> Json<domain::MatchResult> {
    Json(rules::evaluate(&input.rule, &input.tender))
}

#[derive(Deserialize)]
struct DailyCollectorRequest {
    date: NaiveDate,
}

async fn run_daily_collector(
    State(state): State<Arc<AppState>>,
    Json(input): Json<DailyCollectorRequest>,
) -> Result<Json<collector::CollectorReport>, ApiError> {
    let repository = PostgresCollectorRepository::new(state.db.clone());
    let report = collector::collect_daily(&state.pcc_source, &repository, input.date)
        .await
        .map_err(ApiError::Collector)?;
    Ok(Json(report))
}

enum ApiError {
    BadRequest(String),
    Database(sqlx::Error),
    Collector(anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        Self::Database(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::BadRequest(message) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": message })),
            )
                .into_response(),
            Self::Database(error) => {
                tracing::error!(%error, "database error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": "database error" })),
                )
                    .into_response()
            }
            Self::Collector(error) => {
                tracing::error!(%error, "daily collector failed");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(serde_json::json!({ "error": "daily collector failed" })),
                )
                    .into_response()
            }
        }
    }
}
