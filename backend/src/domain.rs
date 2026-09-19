use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenderCandidate {
    pub source: String,
    pub source_tender_id: String,
    pub tender_no: Option<String>,
    pub title: String,
    pub agency_code: Option<String>,
    pub agency_name: String,
    pub notice_type: Option<String>,
    #[serde(default)]
    pub notice_date: Option<NaiveDate>,
    #[serde(default)]
    pub version_key: Option<String>,
    pub source_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MatchMode {
    Any,
    All,
}

impl MatchMode {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Any => "ANY",
            Self::All => "ALL",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "ANY" => Some(Self::Any),
            "ALL" => Some(Self::All),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KeywordScope {
    Agency,
    Tender,
}

impl KeywordScope {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Agency => "AGENCY",
            Self::Tender => "TENDER",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "AGENCY" => Some(Self::Agency),
            "TENDER" => Some(Self::Tender),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum KeywordMode {
    Include,
    Exclude,
}

impl KeywordMode {
    pub fn as_db_str(&self) -> &'static str {
        match self {
            Self::Include => "INCLUDE",
            Self::Exclude => "EXCLUDE",
        }
    }

    pub fn from_db_str(value: &str) -> Option<Self> {
        match value {
            "INCLUDE" => Some(Self::Include),
            "EXCLUDE" => Some(Self::Exclude),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeywordCondition {
    pub scope: KeywordScope,
    pub mode: KeywordMode,
    pub pattern: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgencyCondition {
    pub agency_code: Option<String>,
    pub agency_name: String,
    #[serde(default)]
    pub include_children: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSpec {
    pub name: String,
    pub match_mode: MatchMode,
    #[serde(default)]
    pub keywords: Vec<KeywordCondition>,
    #[serde(default)]
    pub agencies: Vec<AgencyCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateWatchRule {
    pub name: String,
    pub match_mode: MatchMode,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub keywords: Vec<KeywordCondition>,
    #[serde(default)]
    pub agencies: Vec<AgencyCondition>,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchResult {
    pub matched: bool,
    pub matched_agency: bool,
    pub matched_keywords: Vec<String>,
    pub excluded_by: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct StoredWatchRule {
    pub id: Uuid,
    pub spec: RuleSpec,
}

#[derive(Debug, Clone)]
pub struct RuleMatch {
    pub watch_rule_id: Uuid,
    pub result: MatchResult,
}

#[derive(Debug, Serialize, FromRow)]
pub struct WatchRuleSummary {
    pub id: Uuid,
    pub name: String,
    pub match_mode: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}
