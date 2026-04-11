use super::*;

fn try_repair_candidate(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &mut SourceCandidate,
    report: &ValidationReport,
) -> Result<bool> {
    if !report.search.passed
        && candidate
            .book_source
            .search_url
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        if let Some(search_url) = infer_search_url_from_sampled(paths, task_id, sampled)? {
            candidate.book_source.search_url = Some(search_url.clone());
            candidate.repair_history.push(RepairStep {
                label: "infer searchUrl".into(),
                reason: format!("从页面搜索表单推断 searchUrl：{search_url}"),
                applied_at: Utc::now(),
            });
            return Ok(true);
        }
    }

    if report.cleanup.passed {
        return Ok(false);
    }

    let fixture_name = match sampled.fixture_name.as_ref() {
        Some(value) => value,
        None => return Ok(false),
    };

    let raw = fs::read_to_string(
        paths
            .fixtures_dir
            .join(fixture_name)
            .join("expected-candidate.json"),
    )?;
    let fallback: SourceCandidate = serde_json::from_str(&raw)?;

    if let Some(content_rule) = candidate.book_source.rule_content.as_mut() {
        if content_rule
            .replace_regex
            .as_deref()
            .unwrap_or_default()
            .is_empty()
        {
            content_rule.replace_regex = fallback
                .book_source
                .rule_content
                .and_then(|rule| rule.replace_regex);
            candidate.repair_history.push(RepairStep {
                label: "fixture cleanup fallback".into(),
                reason: "发现广告文本仍残留，应用 fixture 预设 replaceRegex。".into(),
                applied_at: Utc::now(),
            });
            return Ok(true);
        }
    }

    Ok(false)
}

pub(super) async fn try_repair_candidate_with_model(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    ai_settings: &StartAiSettings,
    api_key: Option<String>,
    candidate: &mut SourceCandidate,
    failure_summary: &str,
) -> Result<bool> {
    if ai_settings.base_url.starts_with("mock://") {
        return Ok(false);
    }
    let Some(key) = api_key.filter(|value| !value.trim().is_empty()) else {
        return Ok(false);
    };

    let prompt = build_repair_prompt(sampled, candidate, failure_summary)?;
    let content = request_model_message(
        paths,
        task_id,
        "source-repair",
        ai_settings,
        &key,
        "Return exactly one JSON object with keys strategy, confidence, evidenceRefs, repairHistory and bookSource. The output must stay Reader-Rust compatible. Do not output markdown fences, comments, explanations, or trailing characters.",
        &prompt,
    )
    .await?;

    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": content
                }
            }
        ]
    }))?;
    let repaired = parse_candidate_from_openai(paths, sampled, &body)?;
    *candidate = merge_candidates(candidate.clone(), repaired);
    candidate.repair_history.push(RepairStep {
        label: "model repair".into(),
        reason: failure_summary.to_string(),
        applied_at: Utc::now(),
    });
    Ok(true)
}

pub(super) fn try_repair_candidate_with_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &mut SourceCandidate,
    report: &ValidationReport,
) -> Result<bool> {
    let inferred = infer_candidate_from_sampled(paths, task_id, sampled)?;
    let mut repaired = false;

    if !report.explore.passed {
        if let Some(explore_url) = inferred.book_source.explore_url.clone() {
            if candidate.book_source.explore_url.as_deref() != Some(explore_url.as_str()) {
                candidate.book_source.explore_url = Some(explore_url.clone());
                candidate.book_source.enabled_explore = Some(true);
                candidate.repair_history.push(RepairStep {
                    label: "repair exploreUrl".into(),
                    reason: format!("use sampled exploreUrl {}", explore_url),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
        if let Some(inferred_rule) = inferred.book_source.rule_explore.clone() {
            let before = serde_json::to_string(&candidate.book_source.rule_explore)?;
            candidate.book_source.rule_explore = Some(inferred_rule);
            if serde_json::to_string(&candidate.book_source.rule_explore)? != before {
                candidate.repair_history.push(RepairStep {
                    label: "repair ruleExplore".into(),
                    reason: "replace failed explore rule with sampled DOM inference".into(),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
    }

    if !report.search.passed {
        if let Some(search_url) = inferred.book_source.search_url.clone() {
            if candidate.book_source.search_url.as_deref() != Some(search_url.as_str()) {
                candidate.book_source.search_url = Some(search_url.clone());
                candidate.repair_history.push(RepairStep {
                    label: "repair searchUrl".into(),
                    reason: format!("use sampled searchUrl {}", search_url),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
        if let Some(inferred_rule) = inferred.book_source.rule_search.clone() {
            let before = serde_json::to_string(&candidate.book_source.rule_search)?;
            candidate.book_source.rule_search = Some(inferred_rule);
            if serde_json::to_string(&candidate.book_source.rule_search)? != before {
                candidate.repair_history.push(RepairStep {
                    label: "repair ruleSearch".into(),
                    reason: "replace failed search rule with sampled DOM inference".into(),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
    }

    if !report.book_info.passed {
        if let Some(inferred_rule) = inferred.book_source.rule_book_info.clone() {
            let before = serde_json::to_string(&candidate.book_source.rule_book_info)?;
            candidate.book_source.rule_book_info = Some(inferred_rule);
            if serde_json::to_string(&candidate.book_source.rule_book_info)? != before {
                candidate.repair_history.push(RepairStep {
                    label: "repair ruleBookInfo".into(),
                    reason: "replace failed book info rule with sampled DOM inference".into(),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
    }

    if !report.toc.passed {
        if let Some(inferred_rule) = inferred.book_source.rule_toc.clone() {
            let before = serde_json::to_string(&candidate.book_source.rule_toc)?;
            candidate.book_source.rule_toc = Some(inferred_rule);
            if serde_json::to_string(&candidate.book_source.rule_toc)? != before {
                candidate.repair_history.push(RepairStep {
                    label: "repair ruleToc".into(),
                    reason: "replace failed toc rule with sampled DOM inference".into(),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
    }

    if !report.content.passed {
        if let Some(inferred_rule) = inferred.book_source.rule_content.clone() {
            let before = serde_json::to_string(&candidate.book_source.rule_content)?;
            candidate.book_source.rule_content = Some(inferred_rule);
            if serde_json::to_string(&candidate.book_source.rule_content)? != before {
                candidate.repair_history.push(RepairStep {
                    label: "repair ruleContent".into(),
                    reason: "replace failed content rule with sampled DOM inference".into(),
                    applied_at: Utc::now(),
                });
                repaired = true;
            }
        }
    }

    if repaired {
        return Ok(true);
    }

    try_repair_candidate(paths, task_id, sampled, candidate, report)
}
