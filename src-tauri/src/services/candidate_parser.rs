use super::*;

pub(super) fn parse_candidate_from_openai(
    _paths: &AppPaths,
    sampled: &SamplingResult,
    body: &OpenAiResponse,
) -> Result<SourceCandidate> {
    let content = body
        .choices
        .first()
        .ok_or_else(|| anyhow!("model response contained no choices"))?
        .message
        .content
        .trim();

    let content = content
        .strip_prefix("```json")
        .or_else(|| content.strip_prefix("```"))
        .unwrap_or(content)
        .trim();
    let content = content.strip_suffix("```").unwrap_or(content).trim();

    let candidate: FixtureResponse = serde_json::from_str(content)?;
    let mut book_source = parse_model_book_source(candidate.book_source)?;
    hydrate_book_source_defaults(&mut book_source, sampled);

    Ok(SourceCandidate {
        strategy: candidate.strategy,
        confidence: parse_confidence(candidate.confidence),
        evidence_refs: parse_evidence_refs(candidate.evidence_refs),
        repair_history: parse_repair_history(candidate.repair_history),
        book_source,
    })
}

pub(super) fn sanitize_candidate(candidate: &mut SourceCandidate) -> Result<()> {
    let mut value = serde_json::to_value(&candidate.book_source)?;
    normalize_model_book_source_value(&mut value);
    let mut book_source: BookSource = serde_json::from_value(value)?;
    normalize_book_source_rules(&mut book_source);
    candidate.book_source = book_source;
    Ok(())
}

fn parse_model_book_source(mut value: Value) -> Result<BookSource> {
    normalize_model_book_source_value(&mut value);
    let mut source: BookSource = serde_json::from_value(value)?;
    normalize_book_source_rules(&mut source);
    Ok(source)
}

fn normalize_model_book_source_value(value: &mut Value) {
    let Some(map) = value.as_object_mut() else {
        return;
    };

    normalize_model_book_source_aliases(map);
    normalize_model_rule_object_value(map, "ruleSearch");
    normalize_model_rule_object_value(map, "ruleExplore");
    normalize_model_rule_object_value(map, "ruleBookInfo");
    normalize_model_rule_object_value(map, "ruleToc");
    normalize_model_rule_object_value(map, "ruleContent");

    if let Some(rule_book_info) = map.get_mut("ruleBookInfo").and_then(Value::as_object_mut) {
        normalize_book_info_rule_aliases(rule_book_info);
    }

    for key in [
        "bookSourceType",
        "book_source_type",
        "customOrder",
        "custom_order",
        "weight",
        "respondTime",
        "respond_time",
        "lastUpdateTime",
        "last_update_time",
    ] {
        if let Some(entry) = map.get_mut(key) {
            normalize_json_i64_like(entry, key);
        }
    }

    for key in [
        "enabled",
        "enabledExplore",
        "enabled_explore",
        "enabledCookieJar",
        "enabled_cookie_jar",
        "loadWithBaseUrl",
        "load_with_base_url",
        "singleUrl",
        "single_url",
    ] {
        if let Some(entry) = map.get_mut(key) {
            normalize_json_bool_like(entry);
        }
    }

    for nested in map.values_mut() {
        normalize_model_rule_strings(nested);
    }
}

fn normalize_model_book_source_aliases(map: &mut serde_json::Map<String, Value>) {
    for (from, to) in [
        ("rule_search", "ruleSearch"),
        ("searchRule", "ruleSearch"),
        ("rule_explore", "ruleExplore"),
        ("exploreRule", "ruleExplore"),
        ("rule_book_info", "ruleBookInfo"),
        ("bookInfoRule", "ruleBookInfo"),
        ("rule_toc", "ruleToc"),
        ("tocRule", "ruleToc"),
        ("rule_content", "ruleContent"),
        ("contentRule", "ruleContent"),
    ] {
        if !map.contains_key(to) {
            if let Some(value) = map.remove(from) {
                map.insert(to.to_string(), value);
            }
        }
    }
}

fn normalize_model_rule_object_value(map: &mut serde_json::Map<String, Value>, key: &str) {
    let Some(value) = map.get_mut(key) else {
        return;
    };

    if let Value::String(text) = value {
        let trimmed = text.trim();
        if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
            return;
        }
        if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
            *value = parsed;
        }
    }
}

fn normalize_book_info_rule_aliases(map: &mut serde_json::Map<String, Value>) {
    for (from, to) in [
        ("titleElement", "name"),
        ("authorElement", "author"),
        ("introElement", "intro"),
        ("coverUrlElement", "coverUrl"),
        ("tocUrlElement", "tocUrl"),
        ("lastChapterElement", "lastChapter"),
        ("kindElement", "kind"),
        ("wordCountElement", "wordCount"),
        ("initJs", "init"),
    ] {
        if !map.contains_key(to) {
            if let Some(value) = map.remove(from) {
                map.insert(to.to_string(), value);
            }
        }
    }
}

fn normalize_model_rule_strings(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_model_rule_strings(nested);
            }
        }
        Value::Array(items) => {
            for nested in items {
                normalize_model_rule_strings(nested);
            }
        }
        Value::String(text) => {
            *text = normalize_reader_rule_syntax(text);
        }
        _ => {}
    }
}

fn normalize_book_source_rules(source: &mut BookSource) {
    source.search_url = source
        .search_url
        .take()
        .map(|value| normalize_reader_rule_syntax(&value));

    if let Some(rule) = source.rule_search.as_mut() {
        normalize_option_rule(&mut rule.check_key_word);
        normalize_option_rule(&mut rule.book_list);
        normalize_option_rule(&mut rule.name);
        normalize_option_rule(&mut rule.author);
        normalize_option_rule(&mut rule.intro);
        normalize_option_rule(&mut rule.kind);
        normalize_option_rule(&mut rule.last_chapter);
        normalize_option_rule(&mut rule.update_time);
        normalize_option_rule(&mut rule.book_url);
        normalize_option_rule(&mut rule.cover_url);
        normalize_option_rule(&mut rule.word_count);
    }
    if let Some(rule) = source.rule_explore.as_mut() {
        normalize_option_rule(&mut rule.check_key_word);
        normalize_option_rule(&mut rule.book_list);
        normalize_option_rule(&mut rule.name);
        normalize_option_rule(&mut rule.author);
        normalize_option_rule(&mut rule.intro);
        normalize_option_rule(&mut rule.kind);
        normalize_option_rule(&mut rule.last_chapter);
        normalize_option_rule(&mut rule.update_time);
        normalize_option_rule(&mut rule.book_url);
        normalize_option_rule(&mut rule.cover_url);
        normalize_option_rule(&mut rule.word_count);
    }
    if let Some(rule) = source.rule_book_info.as_mut() {
        normalize_option_rule(&mut rule.init);
        normalize_option_rule(&mut rule.name);
        normalize_option_rule(&mut rule.author);
        normalize_option_rule(&mut rule.intro);
        normalize_option_rule(&mut rule.kind);
        normalize_option_rule(&mut rule.last_chapter);
        normalize_option_rule(&mut rule.update_time);
        normalize_option_rule(&mut rule.cover_url);
        normalize_option_rule(&mut rule.word_count);
        normalize_option_rule(&mut rule.toc_url);
        normalize_option_rule(&mut rule.can_re_name);
        normalize_option_rule(&mut rule.download_urls);
    }
    if let Some(rule) = source.rule_toc.as_mut() {
        normalize_option_rule(&mut rule.pre_update_js);
        normalize_option_rule(&mut rule.init);
        normalize_option_rule(&mut rule.chapter_list);
        normalize_option_rule(&mut rule.chapter_name);
        normalize_option_rule(&mut rule.chapter_url);
        normalize_option_rule(&mut rule.format_js);
        normalize_option_rule(&mut rule.is_volume);
        normalize_option_rule(&mut rule.is_vip);
        normalize_option_rule(&mut rule.is_pay);
        normalize_option_rule(&mut rule.update_time);
        normalize_option_rule(&mut rule.next_toc_url);
        rule.update_time = None;
    }
    if let Some(rule) = source.rule_content.as_mut() {
        normalize_option_rule(&mut rule.content);
        normalize_option_rule(&mut rule.title);
        normalize_option_rule(&mut rule.next_content_url);
        normalize_option_rule(&mut rule.web_js);
        normalize_option_rule(&mut rule.source_regex);
        normalize_option_rule(&mut rule.replace_regex);
        normalize_option_rule(&mut rule.image_style);
        normalize_option_rule(&mut rule.image_decode);
        normalize_option_rule(&mut rule.pay_action);
        rule.title = None;
    }
}

fn normalize_option_rule(value: &mut Option<String>) {
    if let Some(text) = value.as_mut() {
        *text = normalize_rule_expression(text);
    }
}

fn normalize_rule_expression(value: &str) -> String {
    let normalized = normalize_reader_rule_syntax(value);
    if !has_multiple_extractor_segments(&normalized) {
        return normalized;
    }

    normalized
        .split(',')
        .map(str::trim)
        .find(|segment| !segment.is_empty() && segment.contains('@'))
        .map(str::to_string)
        .unwrap_or(normalized)
}

fn has_multiple_extractor_segments(value: &str) -> bool {
    let mut in_brackets = 0i32;
    let mut extractors = 0usize;
    for ch in value.chars() {
        match ch {
            '[' => in_brackets += 1,
            ']' => in_brackets = (in_brackets - 1).max(0),
            '@' if in_brackets == 0 => extractors += 1,
            _ => {}
        }
    }
    extractors > 1
}

fn normalize_json_i64_like(value: &mut Value, key: &str) {
    if let Value::String(text) = value {
        let trimmed = text.trim();
        if let Ok(parsed) = trimmed.parse::<i64>() {
            *value = Value::from(parsed);
            return;
        }

        if key.eq_ignore_ascii_case("bookSourceType")
            || key.eq_ignore_ascii_case("book_source_type")
        {
            let mapped = match trimmed.to_ascii_lowercase().as_str() {
                "novel" | "text" | "book" | "default" => Some(0),
                "audio" => Some(1),
                "image" | "comic" => Some(2),
                "file" => Some(3),
                _ => None,
            };
            if let Some(parsed) = mapped {
                *value = Value::from(parsed);
            } else {
                *value = Value::Null;
            }
        } else {
            *value = Value::Null;
        }
    }
}

fn normalize_json_bool_like(value: &mut Value) {
    if let Value::String(text) = value {
        let mapped = match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some(true),
            "false" | "0" | "no" | "off" => Some(false),
            _ => None,
        };
        if let Some(flag) = mapped {
            *value = Value::Bool(flag);
        }
    }
}

fn parse_confidence(value: Value) -> f64 {
    match value {
        Value::Number(number) => number.as_f64().unwrap_or(0.0).clamp(0.0, 1.0),
        Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(parsed) = trimmed.parse::<f64>() {
                return parsed.clamp(0.0, 1.0);
            }

            match trimmed.to_ascii_lowercase().as_str() {
                "very_high" | "very-high" | "very high" => 0.95,
                "high" => 0.85,
                "medium" | "moderate" => 0.65,
                "low" => 0.4,
                "very_low" | "very-low" | "very low" => 0.2,
                _ => 0.0,
            }
        }
        Value::Bool(flag) => {
            if flag {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

fn hydrate_book_source_defaults(book_source: &mut BookSource, sampled: &SamplingResult) {
    if book_source.book_source_name.trim().is_empty() {
        book_source.book_source_name = sampled.site_label.clone();
    }

    if book_source.book_source_url.trim().is_empty() {
        book_source.book_source_url = sampled.base_url.clone();
    }
}

fn parse_evidence_refs(value: Value) -> Vec<EvidenceRef> {
    match value {
        Value::Array(items) => items.into_iter().filter_map(parse_evidence_ref).collect(),
        Value::String(text) => vec![EvidenceRef {
            label: text,
            artifact_path: String::new(),
            detail: None,
        }],
        _ => Vec::new(),
    }
}

fn parse_evidence_ref(value: Value) -> Option<EvidenceRef> {
    match value {
        Value::Object(map) => serde_json::from_value(Value::Object(map)).ok(),
        Value::String(text) => Some(EvidenceRef {
            label: text,
            artifact_path: String::new(),
            detail: None,
        }),
        _ => None,
    }
}

fn parse_repair_history(value: Value) -> Vec<RepairStep> {
    match value {
        Value::Array(items) => items.into_iter().filter_map(parse_repair_step).collect(),
        Value::String(text) => vec![RepairStep {
            label: "model-note".into(),
            reason: text,
            applied_at: Utc::now(),
        }],
        _ => Vec::new(),
    }
}

fn parse_repair_step(value: Value) -> Option<RepairStep> {
    match value {
        Value::Object(map) => serde_json::from_value(Value::Object(map)).ok(),
        Value::String(text) => Some(RepairStep {
            label: "model-note".into(),
            reason: text,
            applied_at: Utc::now(),
        }),
        _ => None,
    }
}
