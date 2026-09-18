use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use chrono::{Datelike, NaiveDate};
use reqwest::{Client, Url};
use scraper::{ElementRef, Html};
use sha2::{Digest, Sha256};

use crate::{domain::TenderCandidate, source::TenderSource};

const SOURCE_NAME: &str = "PCC_DAILY_HTML";
const DEFAULT_BASE_URL: &str = "https://web.pcc.gov.tw";
const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// Opportunity-like sections in PCC's daily gazette.
///
/// Award, failed-award, vendor suspension, sale and lease sections are
/// deliberately excluded from the V0.1 new-opportunity scanner.
const OPPORTUNITY_NOTICE_TYPES: &[&str] = &[
    "公開招標公告",
    "公開招標更正公告",
    "經公開評選或公開徵求之限制性招標公告",
    "經公開評選或公開徵求之限制性招標更正公告",
    "選擇性招標公告(建立合格廠商名單)",
    "選擇性招標更正公告(建立合格廠商名單)",
    "選擇性招標公告(個案)",
    "選擇性招標更正公告(個案)",
    "選擇性招標後續邀標公告",
    "選擇性招標後續邀標更正公告",
    "公開徵求廠商提供參考資料",
    "公開徵求廠商提供參考資料更正公告",
    "招標文件公開閱覽公告資料公告",
    "招標文件公開閱覽公告資料更正公告",
    "公開取得報價單或企劃書公告",
    "公開取得報價單或企劃書更正公告",
];

#[derive(Clone)]
pub struct PccDailySource {
    client: Client,
    base_url: Url,
}

impl PccDailySource {
    pub fn new(base_url: Option<&str>, timeout: Duration) -> Result<Self> {
        let base_url =
            Url::parse(base_url.unwrap_or(DEFAULT_BASE_URL)).context("invalid PCC base URL")?;
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(timeout)
            .user_agent("BidRadar/0.1 (+https://github.com/ahnchen1983/BidRadar)")
            .build()
            .context("failed to build PCC HTTP client")?;

        Ok(Self { client, base_url })
    }

    fn daily_url(&self, date: NaiveDate) -> Result<Url> {
        let roc_year = date.year() - 1911;
        if roc_year <= 0 {
            bail!("PCC daily lookup does not support dates before 1912");
        }

        let mut url = self
            .base_url
            .join("/prkms/tender/common/noticeDate/readPublish")
            .context("failed to construct PCC daily URL")?;
        let date_str = format!("{roc_year}年{:02}月{:02}日", date.month(), date.day());
        url.query_pairs_mut().append_pair("dateStr", &date_str);
        Ok(url)
    }

    async fn fetch_html(&self, date: NaiveDate) -> Result<String> {
        let url = self.daily_url(date)?;
        let response = self
            .client
            .get(url.clone())
            .send()
            .await
            .with_context(|| format!("PCC daily request failed: {url}"))?
            .error_for_status()
            .with_context(|| format!("PCC daily request returned an error: {url}"))?;

        if response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
        {
            bail!("PCC daily response exceeded {MAX_RESPONSE_BYTES} bytes");
        }

        let bytes = response
            .bytes()
            .await
            .context("failed to read PCC daily response")?;
        if bytes.len() > MAX_RESPONSE_BYTES {
            bail!("PCC daily response exceeded {MAX_RESPONSE_BYTES} bytes");
        }

        String::from_utf8(bytes.to_vec()).context("PCC daily response was not UTF-8")
    }
}

impl Default for PccDailySource {
    fn default() -> Self {
        Self::new(None, Duration::from_secs(60)).expect("default PCC client must be valid")
    }
}

#[async_trait]
impl TenderSource for PccDailySource {
    fn source_name(&self) -> &'static str {
        SOURCE_NAME
    }

    async fn list_notices(&self, date: NaiveDate) -> Result<Vec<TenderCandidate>> {
        let html = self.fetch_html(date).await?;
        parse_daily_html(&html, date, &self.base_url)
    }
}

pub fn parse_daily_html(
    html: &str,
    date: NaiveDate,
    base_url: &Url,
) -> Result<Vec<TenderCandidate>> {
    if html.contains("尚無資料") {
        return Ok(Vec::new());
    }

    let roc_year = date.year() - 1911;
    let expected_title = format!(
        "民國{roc_year}年{:02}月{:02}日刊登公報之政府採購公告",
        date.month(),
        date.day()
    );
    if !html.contains(&expected_title) {
        bail!("PCC response did not contain the requested daily gazette title");
    }

    let document = Html::parse_document(html);
    let mut current_notice_type: Option<String> = None;
    let mut candidates = Vec::new();

    for node in document.root_element().descendants() {
        let Some(element) = ElementRef::wrap(node) else {
            continue;
        };
        if element.value().name() != "a" {
            continue;
        }

        if let Some(section_id) = element.value().attr("id") {
            current_notice_type = is_opportunity_notice(section_id).then(|| section_id.to_owned());
        }

        if !has_class(&element, "tenderLinkPublish") {
            continue;
        }

        let Some(notice_type) = current_notice_type.clone() else {
            continue;
        };
        let version_key = element
            .value()
            .attr("href")
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow!("PCC notice link was missing href"))?;
        let listing_text = collapse_whitespace(element.text());
        let (agency_name, tender_no, title) = parse_listing_text(&listing_text)
            .with_context(|| format!("failed to parse PCC notice listing: {listing_text}"))?;

        let source_tender_id = source_tender_id(&agency_name, &tender_no);
        let source_url = notice_url(base_url, date, version_key)?;
        candidates.push(TenderCandidate {
            source: SOURCE_NAME.to_owned(),
            source_tender_id,
            tender_no: Some(tender_no),
            title,
            agency_code: None,
            agency_name,
            notice_type: Some(notice_type),
            notice_date: Some(date),
            version_key: Some(version_key.to_owned()),
            source_url: Some(source_url.to_string()),
        });
    }

    Ok(candidates)
}

fn is_opportunity_notice(value: &str) -> bool {
    OPPORTUNITY_NOTICE_TYPES.contains(&value)
}

fn has_class(element: &ElementRef<'_>, expected: &str) -> bool {
    element
        .value()
        .attr("class")
        .is_some_and(|classes| classes.split_whitespace().any(|class| class == expected))
}

fn collapse_whitespace<'a>(text: impl Iterator<Item = &'a str>) -> String {
    text.flat_map(|fragment| fragment.split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_listing_text(value: &str) -> Result<(String, String, String)> {
    let without_ordinal = value
        .split_once('>')
        .map_or(value, |(_, remainder)| remainder)
        .trim();
    let (agency_name, case_and_title) = without_ordinal
        .split_once('：')
        .or_else(|| without_ordinal.split_once(':'))
        .ok_or_else(|| anyhow!("missing agency separator"))?;
    let (tender_no, title) = case_and_title
        .split_once(" - ")
        .ok_or_else(|| anyhow!("missing tender number/title separator"))?;

    let agency_name = agency_name.trim();
    let tender_no = tender_no.trim();
    let title = title.trim();
    if agency_name.is_empty() || tender_no.is_empty() || title.is_empty() {
        bail!("agency, tender number and title must not be empty");
    }

    Ok((
        agency_name.to_owned(),
        tender_no.to_owned(),
        title.to_owned(),
    ))
}

fn source_tender_id(agency_name: &str, tender_no: &str) -> String {
    let normalized_agency = agency_name.split_whitespace().collect::<String>();
    let normalized_tender_no = tender_no
        .split_whitespace()
        .collect::<String>()
        .to_uppercase();
    let identity = format!("{normalized_agency}\0{normalized_tender_no}");
    format!("pcc-index-v1:{:x}", Sha256::digest(identity.as_bytes()))
}

fn notice_url(base_url: &Url, date: NaiveDate, version_key: &str) -> Result<Url> {
    let mut url = base_url
        .join("/prkms/tender/common/noticeDate/redirectPublic")
        .context("failed to construct PCC notice URL")?;
    url.query_pairs_mut()
        .append_pair("ds", &date.format("%Y%m%d").to_string())
        .append_pair("fn", version_key);
    Ok(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../../tests/fixtures/pcc_daily_sample.html");

    #[test]
    fn parses_only_opportunity_notices_and_keeps_versions_separate() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();
        let base_url = Url::parse(DEFAULT_BASE_URL).unwrap();
        let candidates = parse_daily_html(FIXTURE, date, &base_url).unwrap();

        assert_eq!(candidates.len(), 3);
        assert_eq!(candidates[0].agency_name, "衛生福利部朴子醫院");
        assert_eq!(candidates[0].tender_no.as_deref(), Some("D051"));
        assert_eq!(candidates[0].title, "AI智慧戰情系統-電子白板一式");
        assert_eq!(candidates[0].notice_type.as_deref(), Some("公開招標公告"));
        assert_eq!(candidates[0].notice_date, Some(date));
        assert_eq!(
            candidates[0].version_key.as_deref(),
            Some("TIQ-1-71117222.xml")
        );

        assert_eq!(
            candidates[0].source_tender_id,
            candidates[2].source_tender_id
        );
        assert_ne!(candidates[0].version_key, candidates[2].version_key);
        assert!(candidates[0]
            .source_url
            .as_deref()
            .unwrap()
            .contains("ds=20260918&fn=TIQ-1-71117222.xml"));
    }

    #[test]
    fn accepts_a_valid_empty_day() {
        let html = "<html><body><div>尚無資料</div></body></html>";
        let date = NaiveDate::from_ymd_opt(2026, 9, 19).unwrap();
        let base_url = Url::parse(DEFAULT_BASE_URL).unwrap();

        assert!(parse_daily_html(html, date, &base_url).unwrap().is_empty());
    }

    #[test]
    fn rejects_an_unexpected_page() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 18).unwrap();
        let base_url = Url::parse(DEFAULT_BASE_URL).unwrap();
        let error = parse_daily_html("<html>maintenance</html>", date, &base_url).unwrap_err();

        assert!(error.to_string().contains("daily gazette title"));
    }
}
