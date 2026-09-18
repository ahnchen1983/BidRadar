use crate::domain::{
    AgencyCondition, KeywordCondition, KeywordMode, KeywordScope, MatchMode, MatchResult, RuleSpec,
    TenderCandidate,
};

fn contains_ci(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    if needle.is_empty() {
        return false;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

fn agency_matches(condition: &AgencyCondition, tender: &TenderCandidate) -> bool {
    if let (Some(expected), Some(actual)) = (&condition.agency_code, &tender.agency_code) {
        if actual == expected {
            return true;
        }
    }

    if condition.include_children {
        tender.agency_name.starts_with(condition.agency_name.trim())
    } else {
        tender.agency_name.trim() == condition.agency_name.trim()
    }
}

fn keyword_matches(condition: &KeywordCondition, tender: &TenderCandidate) -> bool {
    match condition.scope {
        KeywordScope::Agency => contains_ci(&tender.agency_name, &condition.pattern),
        KeywordScope::Tender => contains_ci(&tender.title, &condition.pattern),
    }
}

pub fn evaluate(rule: &RuleSpec, tender: &TenderCandidate) -> MatchResult {
    let mut excluded_by = Vec::new();

    for keyword in rule
        .keywords
        .iter()
        .filter(|k| matches!(k.mode, KeywordMode::Exclude))
    {
        if keyword_matches(keyword, tender) {
            excluded_by.push(keyword.pattern.clone());
        }
    }

    if !excluded_by.is_empty() {
        return MatchResult {
            matched: false,
            matched_agency: false,
            matched_keywords: Vec::new(),
            excluded_by,
        };
    }

    let agency_results: Vec<bool> = rule
        .agencies
        .iter()
        .map(|agency| agency_matches(agency, tender))
        .collect();

    let include_keywords: Vec<&KeywordCondition> = rule
        .keywords
        .iter()
        .filter(|k| matches!(k.mode, KeywordMode::Include))
        .collect();

    let keyword_results: Vec<bool> = include_keywords
        .iter()
        .map(|keyword| keyword_matches(keyword, tender))
        .collect();

    let positive_count = agency_results.len() + keyword_results.len();
    let positive_match_count = agency_results.iter().filter(|v| **v).count()
        + keyword_results.iter().filter(|v| **v).count();

    let matched = if positive_count == 0 {
        false
    } else {
        match rule.match_mode {
            MatchMode::Any => positive_match_count > 0,
            MatchMode::All => positive_match_count == positive_count,
        }
    };

    let matched_keywords = include_keywords
        .iter()
        .zip(keyword_results.iter())
        .filter_map(|(keyword, did_match)| {
            if *did_match {
                Some(keyword.pattern.clone())
            } else {
                None
            }
        })
        .collect();

    MatchResult {
        matched,
        matched_agency: agency_results.iter().any(|v| *v),
        matched_keywords,
        excluded_by,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        AgencyCondition, KeywordCondition, KeywordMode, KeywordScope, MatchMode, RuleSpec,
        TenderCandidate,
    };

    fn candidate() -> TenderCandidate {
        TenderCandidate {
            source: "PCC".into(),
            source_tender_id: "demo-001".into(),
            tender_no: Some("115-AI-001".into()),
            title: "生成式AI數位轉型輔導計畫".into(),
            agency_code: Some("376500000A".into()),
            agency_name: "嘉義縣政府".into(),
            notice_type: Some("招標公告".into()),
            source_url: None,
        }
    }

    #[test]
    fn any_mode_matches_tender_keyword() {
        let rule = RuleSpec {
            name: "AI".into(),
            match_mode: MatchMode::Any,
            agencies: vec![],
            keywords: vec![KeywordCondition {
                scope: KeywordScope::Tender,
                mode: KeywordMode::Include,
                pattern: "AI".into(),
            }],
        };

        assert!(evaluate(&rule, &candidate()).matched);
    }

    #[test]
    fn all_mode_requires_agency_and_keyword() {
        let rule = RuleSpec {
            name: "Chiayi AI".into(),
            match_mode: MatchMode::All,
            agencies: vec![AgencyCondition {
                agency_code: None,
                agency_name: "嘉義縣政府".into(),
                include_children: false,
            }],
            keywords: vec![KeywordCondition {
                scope: KeywordScope::Tender,
                mode: KeywordMode::Include,
                pattern: "數位轉型".into(),
            }],
        };

        assert!(evaluate(&rule, &candidate()).matched);
    }

    #[test]
    fn exclude_vetoes_match() {
        let rule = RuleSpec {
            name: "AI but not consulting".into(),
            match_mode: MatchMode::Any,
            agencies: vec![],
            keywords: vec![
                KeywordCondition {
                    scope: KeywordScope::Tender,
                    mode: KeywordMode::Include,
                    pattern: "AI".into(),
                },
                KeywordCondition {
                    scope: KeywordScope::Tender,
                    mode: KeywordMode::Exclude,
                    pattern: "輔導".into(),
                },
            ],
        };

        let result = evaluate(&rule, &candidate());
        assert!(!result.matched);
        assert_eq!(result.excluded_by, vec!["輔導"]);
    }
}
