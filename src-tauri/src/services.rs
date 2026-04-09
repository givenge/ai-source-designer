use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use chrono::Utc;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rusqlite::{params, Connection};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::domain::{
    AiSettings, ArtifactBundle, ArtifactFile, ArtifactManifest, BookSource, EvidenceRef,
    LogEntry, RepairStep, SamplingResult, SourceCandidate, StartAiSettings, TaskRecord,
    TaskStatus, ValidationPhase, ValidationReport,
};
use crate::state::AppPaths;
use crate::worker;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureResponse {
    strategy: String,
    confidence: f64,
    #[serde(default)]
    evidence_refs: Value,
    #[serde(default)]
    repair_history: Value,
    book_source: BookSource,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAiMessage {
    content: String,
}

pub fn init_database(paths: &AppPaths) -> Result<()> {
    let conn = Connection::open(&paths.db_path)?;
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
          id TEXT PRIMARY KEY,
          status TEXT NOT NULL,
          updated_at TEXT NOT NULL,
          target_url TEXT NOT NULL,
          payload TEXT NOT NULL
        );
        "#,
    )?;
    Ok(())
}

pub fn load_settings(paths: &AppPaths) -> Result<AiSettings> {
    if !paths.settings_path.exists() {
        return Ok(AiSettings::default());
    }

    let raw = fs::read_to_string(&paths.settings_path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub fn save_settings(paths: &AppPaths, settings: &AiSettings) -> Result<AiSettings> {
    let raw = serde_json::to_string_pretty(settings)?;
    fs::write(&paths.settings_path, raw)?;
    Ok(settings.clone())
}

pub fn create_task(paths: &AppPaths, draft: crate::domain::TaskDraft) -> Result<TaskRecord> {
    let task = TaskRecord::new(draft);
    paths.ensure_task_layout(&task.id)?;
    persist_task(paths, &task)?;
    write_logs(paths, &task)?;
    Ok(task)
}

pub fn list_tasks(paths: &AppPaths) -> Result<Vec<TaskRecord>> {
    let conn = Connection::open(&paths.db_path)?;
    let mut stmt = conn.prepare("SELECT payload FROM tasks ORDER BY updated_at DESC")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;

    let mut tasks = Vec::new();
    for row in rows {
        let payload = row?;
        let task: TaskRecord = serde_json::from_str(&payload)?;
        tasks.push(task);
    }

    Ok(tasks)
}

pub fn get_task(paths: &AppPaths, task_id: &str) -> Result<TaskRecord> {
    let conn = Connection::open(&paths.db_path)?;
    let payload: String = conn.query_row(
        "SELECT payload FROM tasks WHERE id = ?1",
        [task_id],
        |row| row.get(0),
    )?;

    Ok(serde_json::from_str(&payload)?)
}

pub fn persist_task(paths: &AppPaths, task: &TaskRecord) -> Result<()> {
    let conn = Connection::open(&paths.db_path)?;
    conn.execute(
        "INSERT OR REPLACE INTO tasks (id, status, updated_at, target_url, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            task.id,
            serde_json::to_string(&task.status)?,
            task.updated_at.to_rfc3339(),
            task.target_url,
            serde_json::to_string(task)?,
        ],
    )?;
    Ok(())
}

pub async fn run_task(
    paths: &AppPaths,
    task_id: &str,
    ai_settings: StartAiSettings,
    api_key: Option<String>,
) -> Result<TaskRecord> {
    let api_key = api_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            load_settings(paths)
                .ok()
                .map(|settings| settings.api_key.trim().to_string())
                .filter(|value| !value.is_empty())
        });

    let mut task = get_task(paths, task_id)?;
    if matches!(
        task.status,
        TaskStatus::Sampling
            | TaskStatus::Synthesizing
            | TaskStatus::Validating
            | TaskStatus::Repairing
    ) {
        bail!("任务正在执行中，请等待当前运行结束");
    }

    task.error_summary = None;
    task.source_candidate = None;
    task.validation_report = None;
    task.artifact_manifest = None;

    let execution: Result<()> = async {
        append_log(&mut task, TaskStatus::Draft, "开始新一轮执行，已清空上次中间结果。");
        persist_and_flush(paths, &mut task)?;

        append_log(&mut task, TaskStatus::Sampling, "开始采样页面与网络证据。");
        task.status = TaskStatus::Sampling;
        persist_and_flush(paths, &mut task)?;

        let sampled = worker::sample_site(
            paths,
            &task.id,
            &task.target_url,
            task.test_keyword.as_deref(),
        )?;
        task.artifact_manifest = Some(sampled.manifest.clone());
        append_log(
            &mut task,
            TaskStatus::Sampling,
            format!("采样完成，共记录 {} 个页面样本。", sampled.pages.len()),
        );
        persist_and_flush(paths, &mut task)?;

        append_log(&mut task, TaskStatus::Synthesizing, "开始合成书源规则候选。");
        task.status = TaskStatus::Synthesizing;
        persist_and_flush(paths, &mut task)?;

        let mut candidate = synthesize_candidate(paths, &sampled, &ai_settings, api_key).await?;
        task.source_candidate = Some(candidate.clone());
        append_log(
            &mut task,
            TaskStatus::Synthesizing,
            format!("规则生成完成，当前置信度 {:.2}。", candidate.confidence),
        );
        persist_and_flush(paths, &mut task)?;

        append_log(&mut task, TaskStatus::Validating, "开始本地兼容与执行验证。");
        task.status = TaskStatus::Validating;
        persist_and_flush(paths, &mut task)?;

        let mut report = validate_candidate(paths, &task.id, &sampled, &candidate)?;
        task.validation_report = Some(report.clone());
        persist_and_flush(paths, &mut task)?;

        if !report.search.passed
            || !report.book_info.passed
            || !report.toc.passed
            || !report.content.passed
            || !report.cleanup.passed
        {
            task.status = TaskStatus::Repairing;
            append_log(&mut task, TaskStatus::Repairing, "验证未完全通过，尝试执行首轮修复。");
            persist_and_flush(paths, &mut task)?;

            if try_repair_candidate(paths, &task.id, &sampled, &mut candidate, &report)? {
                report = validate_candidate(paths, &task.id, &sampled, &candidate)?;
                task.source_candidate = Some(candidate.clone());
                task.validation_report = Some(report.clone());
                append_log(&mut task, TaskStatus::Repairing, "已应用修复策略并重新验证。");
                persist_and_flush(paths, &mut task)?;
            } else {
                append_log(&mut task, TaskStatus::Repairing, "没有可用的自动修复策略。");
                persist_and_flush(paths, &mut task)?;
            }
        }

        let success = report.search.passed
            && report.book_info.passed
            && report.toc.passed
            && report.content.passed
            && report.cleanup.passed;

        if success {
            task.status = TaskStatus::Completed;
            task.error_summary = None;
            let exported = export_candidate(paths, &task.id, &candidate)?;
            let task_id = task.id.clone();
            append_output(paths, &task_id, &mut task, exported.0);
            append_log(&mut task, TaskStatus::Completed, "任务已完成，可导出书源 JSON。");
        } else {
            task.status = TaskStatus::Failed;
            task.error_summary = Some(first_failure_summary(&report));
            let summary = task
                .error_summary
                .clone()
                .unwrap_or_else(|| "任务失败".to_string());
            append_log(&mut task, TaskStatus::Failed, summary);
        }

        persist_and_flush(paths, &mut task)?;
        Ok(())
    }
    .await;

    if let Err(error) = execution {
        let summary = error.to_string();
        task.status = TaskStatus::Failed;
        task.error_summary = Some(summary.clone());
        append_log(&mut task, TaskStatus::Failed, format!("任务失败：{summary}"));
        persist_and_flush(paths, &mut task)?;
    }

    Ok(task)
}

pub fn get_task_artifacts(paths: &AppPaths, task_id: &str) -> Result<ArtifactBundle> {
    let task = get_task(paths, task_id)?;
    let manifest = task.artifact_manifest.unwrap_or_default();

    Ok(ArtifactBundle {
        screenshots: read_files(paths, task_id, &manifest.screenshots)?,
        dom: read_files(paths, task_id, &manifest.dom)?,
        network: read_files(paths, task_id, &manifest.network)?,
        logs: read_files(paths, task_id, &manifest.logs)?,
        output: read_files(paths, task_id, &manifest.output)?,
        manifest,
    })
}

pub fn export_task_source(paths: &AppPaths, task_id: &str) -> Result<String> {
    let task = get_task(paths, task_id)?;
    let candidate = task
        .source_candidate
        .as_ref()
        .ok_or_else(|| anyhow!("task has no source candidate"))?;
    let (relative_path, raw) = export_candidate(paths, &task.id, candidate)?;

    let mut updated = task.clone();
    append_output(paths, &task.id, &mut updated, relative_path);
    persist_and_flush(paths, &mut updated)?;
    Ok(raw)
}

fn append_output(paths: &AppPaths, task_id: &str, task: &mut TaskRecord, relative_path: String) {
    let manifest = task.artifact_manifest.get_or_insert_with(ArtifactManifest::default);
    if !manifest.output.iter().any(|entry| entry == &relative_path) {
        manifest.output.push(relative_path);
    }
    let _ = write_logs(paths, task);
    let _ = task_id;
}

fn export_candidate(paths: &AppPaths, task_id: &str, candidate: &SourceCandidate) -> Result<(String, String)> {
    let raw = serde_json::to_string_pretty(&candidate.book_source)?;
    let output_path = paths.task_path(task_id, "output/book-source.json");
    fs::write(&output_path, &raw)?;
    Ok((paths.relative_to_task(task_id, &output_path), raw))
}

fn read_files(paths: &AppPaths, task_id: &str, entries: &[String]) -> Result<Vec<ArtifactFile>> {
    entries
        .iter()
        .map(|relative| {
            let path = paths.task_path(task_id, relative);
            let content_type = content_type_for(&path);
            let contents = if content_type.starts_with("image/") && content_type != "image/svg+xml" {
                fs::read(&path)
                    .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
                    .unwrap_or_default()
            } else {
                fs::read_to_string(&path).unwrap_or_else(|_| String::new())
            };
            Ok(ArtifactFile {
                label: path
                    .file_name()
                    .map(|value| value.to_string_lossy().to_string())
                    .unwrap_or_else(|| relative.clone()),
                content_type,
                path: relative.clone(),
                contents,
            })
        })
        .collect()
}

fn content_type_for(path: &PathBuf) -> String {
    match path.extension().and_then(|value| value.to_str()).unwrap_or_default() {
        "svg" => "image/svg+xml".into(),
        "png" => "image/png".into(),
        "json" => "application/json".into(),
        "html" => "text/html".into(),
        "log" => "text/plain".into(),
        _ => "text/plain".into(),
    }
}

fn persist_and_flush(paths: &AppPaths, task: &mut TaskRecord) -> Result<()> {
    task.updated_at = Utc::now();
    persist_task(paths, task)?;
    write_logs(paths, task)?;
    Ok(())
}

fn write_logs(paths: &AppPaths, task: &TaskRecord) -> Result<()> {
    let log_path = paths.task_path(&task.id, "logs/task.log");
    let mut body = String::new();

    for entry in &task.logs {
        body.push_str(&format!(
            "{} [{}] {}\n",
            entry.at.to_rfc3339(),
            serde_json::to_string(&entry.stage)?,
            entry.message
        ));
    }

    fs::write(log_path, body)?;
    Ok(())
}

fn append_log(task: &mut TaskRecord, stage: TaskStatus, message: impl Into<String>) {
    task.logs.push(LogEntry {
        at: Utc::now(),
        stage,
        message: message.into(),
    });
}

async fn synthesize_candidate(
    paths: &AppPaths,
    sampled: &SamplingResult,
    ai_settings: &StartAiSettings,
    api_key: Option<String>,
) -> Result<SourceCandidate> {
    if ai_settings.base_url.starts_with("mock://") {
        return load_mock_candidate(paths, sampled, false);
    }

    let payload = json!({
        "model": ai_settings.model,
        "temperature": 0.1,
        "messages": [
            {
                "role": "system",
                "content": "Return a JSON object with keys strategy, confidence, evidenceRefs, repairHistory and bookSource. evidenceRefs must be an array of objects with label, artifactPath and optional detail. repairHistory must be an array. Do not include markdown fences."
            },
            {
                "role": "user",
                "content": build_prompt(sampled)
            }
        ]
    });

    let endpoint = normalize_openai_endpoint(&ai_settings.base_url);
    let key = api_key.ok_or_else(|| anyhow!("real model call requires api key"))?;
    let client = reqwest::Client::new();
    let response = client
        .post(endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(AUTHORIZATION, format!("Bearer {key}"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| anyhow!("模型请求失败：{error}"))?;

    if !response.status().is_success() {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        let snippet = body.chars().take(240).collect::<String>();
        bail!("模型请求失败：HTTP {status} {snippet}");
    }

    let body: OpenAiResponse = response.json().await?;
    parse_candidate_from_openai(paths, sampled, &body)
}

fn normalize_openai_endpoint(base_url: &str) -> String {
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
        "Base URL: {}\nPages:\n{}\nNetwork:\n{}\nGenerate a minimal Reader-Rust compatible book source.",
        sampled.base_url, pages, requests
    )
}

fn load_mock_candidate(paths: &AppPaths, sampled: &SamplingResult, broken: bool) -> Result<SourceCandidate> {
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

fn parse_candidate_from_openai(
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
    let mut book_source = candidate.book_source;
    hydrate_book_source_defaults(&mut book_source, sampled);

    Ok(SourceCandidate {
        strategy: candidate.strategy,
        confidence: candidate.confidence,
        evidence_refs: parse_evidence_refs(candidate.evidence_refs),
        repair_history: parse_repair_history(candidate.repair_history),
        book_source,
    })
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
        Value::Array(items) => items
            .into_iter()
            .filter_map(parse_evidence_ref)
            .collect(),
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
        Value::Array(items) => items
            .into_iter()
            .filter_map(parse_repair_step)
            .collect(),
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

fn validate_candidate(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationReport> {
    ensure_compatibility(&candidate.book_source)?;

    let search = validate_search(paths, task_id, sampled, candidate)?;
    let book_info = validate_book_info(paths, task_id, sampled, candidate)?;
    let toc = validate_toc(paths, task_id, sampled, candidate)?;
    let content = validate_content(paths, task_id, sampled, candidate)?;
    let cleanup = validate_cleanup(paths, task_id, sampled, candidate)?;

    Ok(ValidationReport {
        search,
        book_info,
        toc,
        content,
        cleanup,
    })
}

fn ensure_compatibility(source: &BookSource) -> Result<()> {
    if source.book_source_name.trim().is_empty() || source.book_source_url.trim().is_empty() {
        bail!("bookSourceName and bookSourceUrl are required");
    }

    let value = serde_json::to_value(source)?;
    if value.pointer("/ruleContent/title").is_some() {
        bail!("ruleContent.title is not supported by Reader-Rust model");
    }
    if value.pointer("/ruleToc/updateTime").is_some() {
        bail!("ruleToc.updateTime is not supported by Reader-Rust model");
    }

    Ok(())
}

fn validate_search(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(search_url) = candidate.book_source.search_url.as_ref() else {
        return Ok(failed_phase(
            "缺少 searchUrl",
            vec!["模型未输出 searchUrl，无法执行搜索验证。".into()],
            sampled_page_evidence("landing", sampled),
        ));
    };
    let Some(rule) = candidate.book_source.rule_search.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleSearch",
            vec!["模型未输出 ruleSearch，无法执行搜索结果提取验证。".into()],
            sampled_page_evidence("search", sampled),
        ));
    };
    if !search_url.contains("${key}") || !search_url.contains("${page}") {
        return Ok(failed_phase(
            "searchUrl 必须包含 ${key} 和 ${page}",
            vec!["当前 searchUrl 不具备最小替换能力。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let page = match sampled_page(sampled, "search") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少搜索结果页样本",
                vec!["当前任务只采样了落地页，尚未拿到真实搜索结果页。".into()],
                sampled_page_evidence("landing", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let book_list_selector = selector(rule.book_list.as_deref().unwrap_or_default())?;
    let items: Vec<_> = document.select(&book_list_selector).collect();

    if items.is_empty() {
        return Ok(failed_phase(
            "搜索列表为空",
            vec!["bookList 选择器没有命中任何结果。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let first = items[0];
    let name = evaluate_on_element(&first, rule.name.as_deref().unwrap_or_default())?;
    let book_url = evaluate_on_element(&first, rule.book_url.as_deref().unwrap_or_default())?;
    let resolved = normalize_url(&page.url, &book_url)?;

    if name.trim().is_empty() || resolved.trim().is_empty() {
        return Ok(failed_phase(
            "搜索结果缺少书名或详情链接",
            vec!["name 或 bookUrl 规则没有提取到有效结果。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    Ok(passed_phase(
        format!("搜索规则通过，共命中 {} 条结果。", items.len()),
        vec![
            format!("首条结果书名：{name}"),
            format!("首条结果链接：{resolved}"),
        ],
        sampled_page_evidence("search", sampled),
    ))
}

fn validate_book_info(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_book_info.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleBookInfo",
            vec!["模型未输出详情页规则。".into()],
            sampled_page_evidence("book", sampled),
        ));
    };
    let page = match sampled_page(sampled, "book") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少详情页样本",
                vec!["当前任务未采样到书籍详情页。".into()],
                sampled_page_evidence("search", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);

    let name = evaluate_on_document(&document, rule.name.as_deref().unwrap_or_default())?;
    let author = evaluate_on_document(&document, rule.author.as_deref().unwrap_or_default())?;
    let intro = evaluate_on_document(&document, rule.intro.as_deref().unwrap_or_default())?;
    let toc_url = evaluate_on_document(&document, rule.toc_url.as_deref().unwrap_or_default())?;
    let resolved_toc = normalize_url(&page.url, &toc_url)?;

    if name.trim().is_empty() || (author.trim().is_empty() && intro.trim().is_empty()) {
        return Ok(failed_phase(
            "详情规则缺少必要字段",
            vec!["ruleBookInfo 至少应提取 name，以及 author 或 intro。".into()],
            sampled_page_evidence("book", sampled),
        ));
    }

    Ok(passed_phase(
        "详情页规则通过。".into(),
        vec![
            format!("书名：{name}"),
            format!("作者：{author}"),
            format!("目录链接：{resolved_toc}"),
        ],
        sampled_page_evidence("book", sampled),
    ))
}

fn validate_toc(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_toc.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleToc",
            vec!["模型未输出目录页规则。".into()],
            sampled_page_evidence("toc", sampled),
        ));
    };
    let page = match sampled_page(sampled, "toc") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少目录页样本",
                vec!["当前任务未采样到目录页。".into()],
                sampled_page_evidence("book", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let chapter_selector = selector(rule.chapter_list.as_deref().unwrap_or_default())?;
    let chapters: Vec<_> = document.select(&chapter_selector).collect();

    if chapters.len() < 2 {
        return Ok(failed_phase(
            "目录章节不足",
            vec!["ruleToc.chapterList 没有命中足够的章节项。".into()],
            sampled_page_evidence("toc", sampled),
        ));
    }

    let first = chapters[0];
    let chapter_name =
        evaluate_on_element(&first, rule.chapter_name.as_deref().unwrap_or_default())?;
    let chapter_url =
        normalize_url(&page.url, &evaluate_on_element(&first, rule.chapter_url.as_deref().unwrap_or_default())?)?;

    Ok(passed_phase(
        format!("目录规则通过，共提取 {} 章。", chapters.len()),
        vec![
            format!("首章标题：{chapter_name}"),
            format!("首章链接：{chapter_url}"),
        ],
        sampled_page_evidence("toc", sampled),
    ))
}

fn validate_content(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_content.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleContent",
            vec!["模型未输出正文规则。".into()],
            sampled_page_evidence("content", sampled),
        ));
    };
    let page = match sampled_page(sampled, "content") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少正文页样本",
                vec!["当前任务未采样到正文页。".into()],
                sampled_page_evidence("toc", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let content = evaluate_on_document(&document, rule.content.as_deref().unwrap_or_default())?;

    if content.trim().is_empty() {
        return Ok(failed_phase(
            "正文为空",
            vec!["ruleContent.content 没有提取到正文。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    let next_url = evaluate_on_document(
        &document,
        rule.next_content_url.as_deref().unwrap_or_default(),
    )
    .unwrap_or_default();
    let resolved_next = if next_url.trim().is_empty() {
        String::new()
    } else {
        normalize_url(&page.url, &next_url)?
    };

    Ok(passed_phase(
        "正文规则通过。".into(),
        vec![
            format!("正文长度：{}", content.chars().count()),
            format!(
                "下一页：{}",
                if resolved_next.is_empty() {
                    "无".into()
                } else {
                    resolved_next
                }
            ),
        ],
        sampled_page_evidence("content", sampled),
    ))
}

fn validate_cleanup(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let rule = candidate
        .book_source
        .rule_content
        .as_ref()
        .ok_or_else(|| anyhow!("missing ruleContent"))?;
    let first_page = sampled_page(sampled, "content")?;
    let first_html = fs::read_to_string(paths.task_path(task_id, &first_page.html_path))?;
    let first_doc = Html::parse_document(&first_html);
    let mut combined = evaluate_on_document(&first_doc, rule.content.as_deref().unwrap_or_default())?;

    if let Some(next_page) = sampled.pages.iter().find(|page| page.role == "content-next") {
        let next_html = fs::read_to_string(paths.task_path(task_id, &next_page.html_path))?;
        let next_doc = Html::parse_document(&next_html);
        let next_content =
            evaluate_on_document(&next_doc, rule.content.as_deref().unwrap_or_default())?;
        combined.push('\n');
        combined.push_str(&next_content);
    }

    let cleaned = apply_replace_regex(&combined, rule.replace_regex.as_deref())?;
    let contains_ad = cleaned.contains("Sponsored by AI Source Designer");
    if contains_ad {
        return Ok(failed_phase(
            "广告清洗失败",
            vec!["replaceRegex 未能移除 fixture 中的广告文本。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    if cleaned.chars().count() < 50 {
        return Ok(failed_phase(
            "清洗后正文过短",
            vec!["内容清洗后没有保留足够的正文文本。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    Ok(passed_phase(
        "清洗规则通过。".into(),
        vec![format!("清洗后正文长度：{}", cleaned.chars().count())],
        sampled_page_evidence("content", sampled),
    ))
}

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

    let raw = fs::read_to_string(paths.fixtures_dir.join(fixture_name).join("expected-candidate.json"))?;
    let fallback: SourceCandidate = serde_json::from_str(&raw)?;

    if let Some(content_rule) = candidate.book_source.rule_content.as_mut() {
        if content_rule.replace_regex.as_deref().unwrap_or_default().is_empty() {
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

fn infer_search_url_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<Option<String>> {
    let form_selector = Selector::parse("form[action]").map_err(|_| anyhow!("invalid selector"))?;
    let input_selector = Selector::parse("input[name]").map_err(|_| anyhow!("invalid selector"))?;

    for role in ["landing", "search"] {
        let Ok(page) = sampled_page(sampled, role) else {
            continue;
        };
        let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
        let document = Html::parse_document(&html);

        for form in document.select(&form_selector) {
            let Some(action) = form.value().attr("action") else {
                continue;
            };
            let action = action.trim();
            if action.is_empty() {
                continue;
            }

            let query_name = form
                .select(&input_selector)
                .find_map(|input| {
                    let input_type = input
                        .value()
                        .attr("type")
                        .unwrap_or("text")
                        .trim()
                        .to_ascii_lowercase();
                    let name = input.value().attr("name").unwrap_or("").trim().to_string();
                    let lower_name = name.to_ascii_lowercase();
                    if name.is_empty() {
                        return None;
                    }
                    if input_type == "text"
                        || input_type == "search"
                        || lower_name.contains("key")
                        || lower_name.contains("search")
                        || lower_name.contains("query")
                        || lower_name.contains("wd")
                    {
                        return Some(name);
                    }
                    None
                });

            let Some(query_name) = query_name else {
                continue;
            };

            let resolved = normalize_url(&page.url, action)?;
            let separator = if resolved.contains('?') { "&" } else { "?" };
            return Ok(Some(format!(
                "{resolved}{separator}{query_name}=${{key}}&page=${{page}}"
            )));
        }
    }

    Ok(None)
}

fn first_failure_summary(report: &ValidationReport) -> String {
    for phase in [
        &report.search,
        &report.book_info,
        &report.toc,
        &report.content,
        &report.cleanup,
    ] {
        if !phase.passed {
            return phase.summary.clone();
        }
    }
    "任务失败".into()
}

fn selector(input: &str) -> Result<Selector> {
    let plain = input.split("##").next().unwrap_or(input).trim();
    let selector = plain
        .split('@')
        .next()
        .unwrap_or_default()
        .trim();

    Selector::parse(selector).map_err(|_| anyhow!("invalid selector: {selector}"))
}

fn evaluate_on_document(document: &Html, rule: &str) -> Result<String> {
    if rule.trim().is_empty() {
        return Ok(String::new());
    }

    let parsed = parse_rule(rule);
    let selector = Selector::parse(parsed.selector).map_err(|_| anyhow!("invalid selector"))?;
    let element = document
        .select(&selector)
        .next()
        .ok_or_else(|| anyhow!("selector did not match"))?;
    extract_value(element, &parsed.mode)
}

fn evaluate_on_element(element: &ElementRef<'_>, rule: &str) -> Result<String> {
    if rule.trim().is_empty() {
        return Ok(String::new());
    }

    let parsed = parse_rule(rule);
    let selector = Selector::parse(parsed.selector).map_err(|_| anyhow!("invalid selector"))?;
    let child = element
        .select(&selector)
        .next()
        .ok_or_else(|| anyhow!("selector did not match"))?;
    extract_value(child, &parsed.mode)
}

enum RuleMode {
    Text,
    Html,
    TextNodes,
    Attr(String),
}

struct ParsedRule<'a> {
    selector: &'a str,
    mode: RuleMode,
}

fn parse_rule(rule: &str) -> ParsedRule<'_> {
    let base = rule.split("##").next().unwrap_or(rule).trim();

    if let Some((selector, _)) = base.split_once("@textNodes") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::TextNodes,
        };
    }
    if let Some((selector, _)) = base.split_once("@html") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Html,
        };
    }
    if let Some((selector, _)) = base.split_once("@text") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Text,
        };
    }
    if let Some((selector, rest)) = base.split_once("@attr[") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Attr(rest.trim_end_matches(']').to_string()),
        };
    }

    ParsedRule {
        selector: base.trim(),
        mode: RuleMode::Text,
    }
}

fn extract_value(element: ElementRef<'_>, mode: &RuleMode) -> Result<String> {
    Ok(match mode {
        RuleMode::Text => element
            .text()
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        RuleMode::Html => element.inner_html(),
        RuleMode::TextNodes => element
            .text()
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        RuleMode::Attr(attr) => element.value().attr(attr).unwrap_or_default().to_string(),
    })
}

fn normalize_url(base: &str, maybe_relative: &str) -> Result<String> {
    if maybe_relative.trim().is_empty() {
        return Ok(String::new());
    }

    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return Ok(maybe_relative.to_string());
    }

    Ok(Url::parse(base)?.join(maybe_relative)?.to_string())
}

fn apply_replace_regex(content: &str, replace_regex: Option<&str>) -> Result<String> {
    let Some(value) = replace_regex else {
        return Ok(content.to_string());
    };

    if value.trim().is_empty() {
        return Ok(content.to_string());
    }

    let parts: Vec<_> = value.split("##").collect();
    let mut current = content.to_string();
    let mut index = 0;
    while index < parts.len() {
        let pattern = parts[index];
        let replacement = parts.get(index + 1).copied().unwrap_or("");
        let regex = Regex::new(pattern)?;
        current = regex.replace_all(&current, replacement).to_string();
        index += 2;
    }
    Ok(current)
}

fn sampled_page<'a>(sampled: &'a SamplingResult, role: &str) -> Result<&'a crate::domain::PageSample> {
    sampled
        .pages
        .iter()
        .find(|page| page.role == role)
        .ok_or_else(|| anyhow!("missing sampled page for role {role}"))
}

fn sampled_page_evidence(role: &str, sampled: &SamplingResult) -> Vec<EvidenceRef> {
    sampled
        .pages
        .iter()
        .find(|page| page.role == role)
        .map(|page| {
            let mut refs = Vec::new();
            if let Some(screenshot) = &page.screenshot_path {
                refs.push(EvidenceRef {
                    label: format!("{role} screenshot"),
                    artifact_path: screenshot.clone(),
                    detail: Some(page.url.clone()),
                });
            }
            for snippet in &page.snippet_paths {
                refs.push(EvidenceRef {
                    label: format!("{role} dom"),
                    artifact_path: snippet.clone(),
                    detail: Some(page.url.clone()),
                });
            }
            refs
        })
        .unwrap_or_default()
}

fn passed_phase(summary: String, details: Vec<String>, evidence_refs: Vec<EvidenceRef>) -> ValidationPhase {
    ValidationPhase {
        passed: true,
        summary,
        details,
        evidence_refs,
    }
}

fn failed_phase(summary: &str, details: Vec<String>, evidence_refs: Vec<EvidenceRef>) -> ValidationPhase {
    ValidationPhase {
        passed: false,
        summary: summary.into(),
        details,
        evidence_refs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{AiSettings, ArtifactManifest, BookInfoRule, ContentRule, SearchRule, SourceCandidate, TocRule};

    fn good_candidate() -> SourceCandidate {
        SourceCandidate {
            strategy: "test".into(),
            confidence: 0.9,
            evidence_refs: vec![],
            repair_history: vec![],
            book_source: BookSource {
                book_source_name: "Fixture".into(),
                book_source_url: "https://fixture.novel.world".into(),
                enabled: Some(true),
                search_url: Some("https://fixture.novel.world/search?q=${key}&page=${page}".into()),
                rule_search: Some(SearchRule {
                    book_list: Some(".book-item".into()),
                    name: Some(".book-title a@text".into()),
                    author: Some(".author@text".into()),
                    book_url: Some(".book-title a@attr[href]".into()),
                    intro: Some(".intro@text".into()),
                    ..Default::default()
                }),
                rule_book_info: Some(BookInfoRule {
                    name: Some("h1.title@text".into()),
                    author: Some(".meta .author@text".into()),
                    intro: Some(".intro@text".into()),
                    toc_url: Some(".toc-link@attr[href]".into()),
                    ..Default::default()
                }),
                rule_toc: Some(TocRule {
                    chapter_list: Some(".chapter-list li".into()),
                    chapter_name: Some("a@text".into()),
                    chapter_url: Some("a@attr[href]".into()),
                    ..Default::default()
                }),
                rule_content: Some(ContentRule {
                    content: Some(".content@textNodes".into()),
                    next_content_url: Some(".next-page@attr[href]".into()),
                    replace_regex: Some("Sponsored by AI Source Designer##".into()),
                }),
            },
        }
    }

    #[test]
    fn apply_replace_regex_removes_fixture_ad_copy() {
        let cleaned = apply_replace_regex(
            "正文\nSponsored by AI Source Designer\n继续正文",
            Some("Sponsored by AI Source Designer##"),
        )
        .unwrap();
        assert!(!cleaned.contains("Sponsored by AI Source Designer"));
    }

    #[test]
    fn normalize_url_expands_relative_links() {
        let resolved = normalize_url(
            "https://fixture.novel.world/book/solar-archive",
            "/book/solar-archive/chapter-1",
        )
        .unwrap();
        assert_eq!(resolved, "https://fixture.novel.world/book/solar-archive/chapter-1");
    }

    #[test]
    fn serialized_book_source_does_not_emit_doc_only_fields() {
        let source = serde_json::to_value(good_candidate().book_source).unwrap();
        assert!(source.pointer("/ruleContent/title").is_none());
        assert!(source.pointer("/ruleToc/updateTime").is_none());
    }

    #[test]
    fn task_roundtrip_payload_is_stable() {
        let task = TaskRecord {
            id: "task-1".into(),
            status: TaskStatus::Draft,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            target_url: "fixture://novel-world".into(),
            test_keyword: Some("星门".into()),
            error_summary: None,
            artifact_manifest: Some(ArtifactManifest::default()),
            source_candidate: Some(good_candidate()),
            validation_report: None,
            logs: vec![],
        };

        let raw = serde_json::to_string(&task).unwrap();
        let decoded: TaskRecord = serde_json::from_str(&raw).unwrap();
        assert_eq!(decoded.id, "task-1");
        assert_eq!(decoded.target_url, "fixture://novel-world");
        assert_eq!(decoded.test_keyword.as_deref(), Some("星门"));
    }

    #[test]
    fn parses_mock_openai_response_payload() {
        let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": "{\"strategy\":\"mock\",\"confidence\":0.91,\"evidenceRefs\":[],\"repairHistory\":[],\"bookSource\":{\"bookSourceName\":\"Fixture\",\"bookSourceUrl\":\"https://fixture.novel.world\"}}"
                    }
                }
            ]
        }))
        .unwrap();

        let parsed = parse_candidate_from_openai(
            &AppPaths {
                data_dir: PathBuf::from("."),
                db_path: PathBuf::from("./db"),
                tasks_dir: PathBuf::from("./tasks"),
                logs_dir: PathBuf::from("./logs"),
                settings_path: PathBuf::from("./settings.json"),
                fixtures_dir: PathBuf::from("./fixtures"),
                worker_script: PathBuf::from("./worker.mjs"),
            },
            &SamplingResult::default(),
            &body,
        )
        .unwrap();

        assert_eq!(parsed.strategy, "mock");
        assert_eq!(parsed.book_source.book_source_name, "Fixture");
    }

    #[test]
    fn parses_string_evidence_refs_from_model_output() {
        let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": r#"{
                          "strategy":"live-model",
                          "confidence":0.74,
                          "evidenceRefs":[
                            "Base URL: https://biquge.xin",
                            "Search entry appears in page header"
                          ],
                          "repairHistory":["first draft"],
                          "bookSource":{
                            "bookSourceName":"Biquge",
                            "bookSourceUrl":"https://biquge.xin"
                          }
                        }"#
                    }
                }
            ]
        }))
        .unwrap();

        let parsed = parse_candidate_from_openai(
            &AppPaths {
                data_dir: PathBuf::from("."),
                db_path: PathBuf::from("./db"),
                tasks_dir: PathBuf::from("./tasks"),
                logs_dir: PathBuf::from("./logs"),
                settings_path: PathBuf::from("./settings.json"),
                fixtures_dir: PathBuf::from("./fixtures"),
                worker_script: PathBuf::from("./worker.mjs"),
            },
            &SamplingResult::default(),
            &body,
        )
        .unwrap();

        assert_eq!(parsed.evidence_refs.len(), 2);
        assert_eq!(parsed.evidence_refs[0].label, "Base URL: https://biquge.xin");
        assert_eq!(parsed.repair_history.len(), 1);
        assert_eq!(parsed.repair_history[0].reason, "first draft");
    }

    #[test]
    fn fills_missing_book_source_fields_from_sampled_site() {
        let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": r#"{
                          "strategy":"live-model",
                          "confidence":0.74,
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "search_url":"https://biquge.xin/search?q=${key}&page=${page}"
                          }
                        }"#
                    }
                }
            ]
        }))
        .unwrap();

        let parsed = parse_candidate_from_openai(
            &AppPaths {
                data_dir: PathBuf::from("."),
                db_path: PathBuf::from("./db"),
                tasks_dir: PathBuf::from("./tasks"),
                logs_dir: PathBuf::from("./logs"),
                settings_path: PathBuf::from("./settings.json"),
                fixtures_dir: PathBuf::from("./fixtures"),
                worker_script: PathBuf::from("./worker.mjs"),
            },
            &SamplingResult {
                fixture_name: None,
                site_label: "biquge.xin".into(),
                base_url: "https://biquge.xin".into(),
                pages: vec![],
                network_samples: vec![],
                manifest: ArtifactManifest::default(),
            },
            &body,
        )
        .unwrap();

        assert_eq!(parsed.book_source.book_source_name, "biquge.xin");
        assert_eq!(parsed.book_source.book_source_url, "https://biquge.xin");
        assert_eq!(
            parsed.book_source.search_url.as_deref(),
            Some("https://biquge.xin/search?q=${key}&page=${page}")
        );
    }

    #[test]
    fn keeps_full_openai_endpoint_if_already_provided() {
        let endpoint = normalize_openai_endpoint("https://integrate.api.nvidia.com/v1/chat/completions");
        assert_eq!(endpoint, "https://integrate.api.nvidia.com/v1/chat/completions");
    }

    #[test]
    fn appends_openai_endpoint_when_base_url_is_root() {
        let endpoint = normalize_openai_endpoint("https://api.openai.com/v1");
        assert_eq!(endpoint, "https://api.openai.com/v1/chat/completions");
    }

    #[test]
    fn ai_settings_backward_compatible_with_old_flag_field() {
        let parsed: AiSettings = serde_json::from_str(
            r#"{"baseUrl":"mock://fixture","model":"mock-fixture","apiKeyPresent":true}"#,
        )
        .unwrap();
        assert!(parsed.api_key.is_empty());
    }
}
