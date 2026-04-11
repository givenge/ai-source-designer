use super::*;

pub(super) async fn request_model_message(
    paths: &AppPaths,
    task_id: &str,
    purpose: &str,
    ai_settings: &StartAiSettings,
    api_key: &str,
    system: &str,
    user: &str,
) -> Result<String> {
    let payload = json!({
        "model": ai_settings.model,
        "temperature": 0.1,
        "messages": [
            {
                "role": "system",
                "content": system
            },
            {
                "role": "user",
                "content": user
            }
        ]
    });

    let endpoint = normalize_openai_endpoint(&ai_settings.base_url);
    let _ = log_to_task_file(
        paths,
        task_id,
        &format!(
            "{} [\"synthesizing\"] model-request purpose={} endpoint={} model={} timeout={}s\n",
            Utc::now().to_rfc3339(),
            purpose,
            endpoint,
            ai_settings.model,
            MODEL_REQUEST_TIMEOUT.as_secs()
        ),
    );
    let client = reqwest::Client::builder()
        .timeout(MODEL_REQUEST_TIMEOUT)
        .build()?;
    let response = client
        .post(&endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {api_key}"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| {
            let summary = describe_model_request_error(&endpoint, &ai_settings.model, &error);
            let _ = log_to_task_file(
                paths,
                task_id,
                &format!(
                    "{} [\"synthesizing\"] model-request-error purpose={} {}\n",
                    Utc::now().to_rfc3339(),
                    purpose,
                    summary
                ),
            );
            anyhow!("{summary}")
        })?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let snippet = body.chars().take(240).collect::<String>();
        let summary = format!(
            "模型请求失败：HTTP {status} endpoint={} model={} body={}",
            endpoint, ai_settings.model, snippet
        );
        let _ = log_to_task_file(
            paths,
            task_id,
            &format!(
                "{} [\"synthesizing\"] model-request-error purpose={} {}\n",
                Utc::now().to_rfc3339(),
                purpose,
                summary
            ),
        );
        bail!(summary);
    }

    let body: OpenAiResponse = response.json().await?;
    Ok(body
        .choices
        .first()
        .ok_or_else(|| anyhow!("model response contained no choices"))?
        .message
        .content
        .clone())
}

fn describe_model_request_error(endpoint: &str, model: &str, error: &reqwest::Error) -> String {
    let kind = if error.is_timeout() {
        "timeout"
    } else if error.is_connect() {
        "connect"
    } else if error.is_body() {
        "body"
    } else if error.is_decode() {
        "decode"
    } else if error.is_request() {
        "request"
    } else {
        "unknown"
    };

    let mut details = vec![
        format!("模型请求失败：{kind}"),
        format!("endpoint={endpoint}"),
        format!("model={model}"),
        format!("detail={error}"),
    ];
    if error.is_timeout() {
        details.push(format!("timeout={}s", MODEL_REQUEST_TIMEOUT.as_secs()));
    }
    details.join(" ")
}

pub(super) fn log_to_task_file(paths: &AppPaths, task_id: &str, line: &str) -> Result<()> {
    let log_path = paths.task_path(task_id, "logs/task.log");
    if let Some(parent) = log_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut content = fs::read_to_string(&log_path).unwrap_or_default();
    content.push_str(line);
    fs::write(log_path, content)?;
    Ok(())
}

pub(super) async fn synthesize_candidate(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    ai_settings: &StartAiSettings,
    api_key: Option<String>,
) -> Result<SourceCandidate> {
    let inferred = infer_candidate_from_sampled(paths, task_id, sampled)?;

    if ai_settings.base_url.starts_with("mock://") {
        return load_mock_candidate(paths, sampled, false);
    }

    if api_key.as_deref().unwrap_or_default().trim().is_empty() {
        return Ok(inferred);
    }

    let key = api_key.ok_or_else(|| anyhow!("real model call requires api key"))?;
    let content = request_model_message(
        paths,
        task_id,
        "source-synthesis",
        ai_settings,
        &key,
        "Return exactly one JSON object with keys strategy, confidence, evidenceRefs, repairHistory and bookSource. Do not output markdown fences, comments, explanations, or trailing characters. evidenceRefs must be an array of objects with label, artifactPath and optional detail. repairHistory must be an array.",
        &build_prompt(sampled),
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
    let model_candidate = parse_candidate_from_openai(paths, sampled, &body)?;
    Ok(merge_candidates(inferred, model_candidate))
}

pub(super) fn normalize_openai_endpoint(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else {
        format!("{trimmed}/chat/completions")
    }
}

fn build_prompt(sampled: &SamplingResult) -> String {
    let pages = sampled
        .pages
        .iter()
        .map(|page| format!("- role={} url={}", page.role, page.url))
        .collect::<Vec<_>>()
        .join("\n");
    let requests = sampled
        .network_samples
        .iter()
        .map(|record| format!("- {} {}", record.method, record.request_url))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are an AI book-source agent.\nFollow this skill contract exactly:\n{}\n\nCompatibility contract:\n- Output Reader-Rust compatible rule syntax only.\n- Allowed extractors: @text, @html, @textNodes, @ownText, @href, @attr[name].\n- Allowed selector special form: text.KEYWORD.\n- Rule expression must contain at most one extractor segment. Avoid forms like `a@href, b@href`.\n- CSS selector part must be parseable by standard selector syntax.\n- Do not use Playwright/CSS pseudo syntax such as ::text, ::attr(...), :contains(...).\n- Use URL placeholders {{key}}, {{page}}, and {{{{...}}}} only.\n\nSite base URL: {}\nSampled pages:\n{}\nObserved network requests:\n{}\n\nReturn one JSON object with keys strategy, confidence, evidenceRefs, repairHistory and bookSource.\nbookSource must try to cover exploreUrl, ruleExplore, searchUrl, ruleSearch, ruleBookInfo, ruleToc and ruleContent when evidence exists.\nDo not return markdown fences.",
        PLAYWRIGHT_BOOK_SOURCE_SKILL,
        sampled.base_url,
        pages,
        requests
    )
}

pub(super) fn build_repair_prompt(
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
    failure_summary: &str,
) -> Result<String> {
    let pages = sampled
        .pages
        .iter()
        .map(|page| format!("- role={} url={}", page.role, page.url))
        .collect::<Vec<_>>()
        .join("\n");
    let current = serde_json::to_string_pretty(&candidate.book_source)?;

    Ok(format!(
        "You are repairing a Reader-Rust book source after validation failure.\nFollow this skill contract exactly:\n{}\n\nCompatibility contract:\n- Output Reader-Rust compatible rule syntax only.\n- Allowed extractors: @text, @html, @textNodes, @ownText, @href, @attr[name].\n- Allowed selector special form: text.KEYWORD.\n- Rule expression must contain at most one extractor segment. Avoid forms like `a@href, b@href`.\n- CSS selector part must be parseable by standard selector syntax.\n- Do not use unsupported syntax such as ::text, ::attr(...), :contains(...).\n- Keep URL placeholders as {{{{key}}}}, {{{{page}}}}, or {{{{...}}}} only.\n- If search cannot be made reliable, keep searchUrl/ruleSearch minimal or null, but preserve explore/book/toc/content when valid.\n\nFailure summary:\n{failure_summary}\n\nSite base URL: {base_url}\nSampled pages:\n{pages}\n\nCurrent bookSource JSON:\n{current}\n\nReturn one JSON object with keys strategy, confidence, evidenceRefs, repairHistory and bookSource. Fix only the incompatible or failing fields and keep evidence-backed fields stable. Do not return markdown fences.",
        PLAYWRIGHT_BOOK_SOURCE_SKILL,
        failure_summary = failure_summary,
        base_url = sampled.base_url,
        pages = pages,
        current = current
    ))
}

fn load_mock_candidate(
    paths: &AppPaths,
    sampled: &SamplingResult,
    broken: bool,
) -> Result<SourceCandidate> {
    let fixture_name = sampled
        .fixture_name
        .as_ref()
        .ok_or_else(|| anyhow!("mock synthesis requires fixture"))?;
    let response_file = if broken {
        "mock-openai-response-broken.json"
    } else {
        "mock-openai-response.json"
    };
    let raw = fs::read_to_string(paths.fixtures_dir.join(fixture_name).join(response_file))?;
    let body: OpenAiResponse = serde_json::from_str(&raw)?;
    parse_candidate_from_openai(paths, sampled, &body)
}
