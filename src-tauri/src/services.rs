use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

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
    AiSettings, ArtifactBundle, ArtifactFile, ArtifactManifest, BookInfoRule, BookSource,
    ContentRule, EvidenceRef, LogEntry, RepairStep, SamplingResult, SearchRule,
    SourceCandidate, StartAiSettings, TaskRecord, TaskStatus, TocRule, ValidationPhase,
    ValidationReport,
};
use crate::state::{AppPaths, AppState, TaskRuntime};
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

const TASK_CANCELLED_ERROR: &str = "task cancelled by user";
const ACTIVE_DELETE_WAIT_TIMEOUT: Duration = Duration::from_secs(20);

struct RunningTaskGuard {
    runtime: Arc<TaskRuntime>,
    task_id: String,
}

impl RunningTaskGuard {
    fn register(state: &AppState, task_id: &str) -> Result<Self> {
        if !state.runtime.register(task_id) {
            bail!("task is still running");
        }

        Ok(Self {
            runtime: Arc::clone(&state.runtime),
            task_id: task_id.to_string(),
        })
    }
}

impl Drop for RunningTaskGuard {
    fn drop(&mut self) {
        self.runtime.unregister(&self.task_id);
    }
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

pub fn delete_task(state: &AppState, task_id: &str) -> Result<()> {
    let _task = get_task(&state.paths, task_id)?;
    if state.runtime.is_running(task_id) {
        state.runtime.request_cancel(task_id);
        let started_at = Instant::now();
        while state.runtime.is_running(task_id) {
            if started_at.elapsed() >= ACTIVE_DELETE_WAIT_TIMEOUT {
                bail!("任务正在停止，请稍后再试");
            }
            thread::sleep(Duration::from_millis(150));
        }
    }

    delete_task_files(&state.paths, task_id)
}

fn delete_task_files(paths: &AppPaths, task_id: &str) -> Result<()> {
    let conn = Connection::open(&paths.db_path)?;
    let removed = conn.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
    if removed == 0 {
        bail!("task not found");
    }

    let task_dir = paths.task_dir(task_id);
    if task_dir.exists() {
        fs::remove_dir_all(task_dir)?;
    }

    Ok(())
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

fn ensure_not_cancelled(state: &AppState, task_id: &str) -> Result<()> {
    if state.runtime.is_cancel_requested(task_id) {
        bail!(TASK_CANCELLED_ERROR);
    }

    Ok(())
}

fn is_task_cancelled(error: &anyhow::Error) -> bool {
    error.to_string().contains(TASK_CANCELLED_ERROR)
}

pub async fn run_task(
    state: &AppState,
    task_id: &str,
    ai_settings: StartAiSettings,
    api_key: Option<String>,
) -> Result<TaskRecord> {
    let _guard = RunningTaskGuard::register(state, task_id)?;
    let paths = &state.paths;
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
    task.error_summary = None;
    task.source_candidate = None;
    task.validation_report = None;
    task.artifact_manifest = None;

    let execution: Result<()> = async {
        ensure_not_cancelled(state, &task.id)?;
        append_log(&mut task, TaskStatus::Draft, "开始新一轮执行，已清空上次中间结果。");
        persist_and_flush(paths, &mut task)?;

        ensure_not_cancelled(state, &task.id)?;
        append_log(&mut task, TaskStatus::Sampling, "开始采样页面与网络证据。");
        task.status = TaskStatus::Sampling;
        persist_and_flush(paths, &mut task)?;

        let sampled = worker::sample_site(
            state,
            &task.id,
            &task.target_url,
            task.test_keyword.as_deref(),
        )?;
        ensure_not_cancelled(state, &task.id)?;
        task.artifact_manifest = Some(sampled.manifest.clone());
        append_log(
            &mut task,
            TaskStatus::Sampling,
            format!("采样完成，共记录 {} 个页面样本。", sampled.pages.len()),
        );
        persist_and_flush(paths, &mut task)?;

        ensure_not_cancelled(state, &task.id)?;
        append_log(&mut task, TaskStatus::Synthesizing, "开始合成书源规则候选。");
        task.status = TaskStatus::Synthesizing;
        persist_and_flush(paths, &mut task)?;

        let mut candidate =
            synthesize_candidate(paths, &task.id, &sampled, &ai_settings, api_key).await?;
        ensure_not_cancelled(state, &task.id)?;
        task.source_candidate = Some(candidate.clone());
        append_log(
            &mut task,
            TaskStatus::Synthesizing,
            format!("规则生成完成，当前置信度 {:.2}。", candidate.confidence),
        );
        persist_and_flush(paths, &mut task)?;

        ensure_not_cancelled(state, &task.id)?;
        append_log(&mut task, TaskStatus::Validating, "开始本地兼容与执行验证。");
        task.status = TaskStatus::Validating;
        persist_and_flush(paths, &mut task)?;

        let mut report = validate_candidate(
            paths,
            &task.id,
            &sampled,
            &candidate,
            task.test_keyword.as_deref(),
        )?;
        ensure_not_cancelled(state, &task.id)?;
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

            if try_repair_candidate_with_sampled(
                paths,
                &task.id,
                &sampled,
                &mut candidate,
                &report,
            )? {
                report = validate_candidate(
                    paths,
                    &task.id,
                    &sampled,
                    &candidate,
                    task.test_keyword.as_deref(),
                )?;
                ensure_not_cancelled(state, &task.id)?;
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
            append_log(&mut task, TaskStatus::Completed, "任务已完成，产物可导出。");
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
        let cancelled = is_task_cancelled(&error);
        let summary = if cancelled {
            "任务已取消".to_string()
        } else {
            error.to_string()
        };
        task.status = TaskStatus::Failed;
        task.error_summary = Some(summary.clone());
        if cancelled {
            append_log(&mut task, TaskStatus::Failed, summary.clone());
        } else {
            append_log(&mut task, TaskStatus::Failed, format!("任务失败：{summary}"));
        }
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

pub fn export_task_source(paths: &AppPaths, task_id: &str, output_path: Option<&str>) -> Result<String> {
    let task = get_task(paths, task_id)?;
    let candidate = task
        .source_candidate
        .as_ref()
        .ok_or_else(|| anyhow!("task has no source candidate"))?;
    let (relative_path, raw) = export_candidate(paths, &task.id, candidate)?;

    if let Some(destination) = output_path.filter(|value| !value.trim().is_empty()) {
        write_export_file(Path::new(destination), &raw)?;
    }

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
    let exported = exportable_book_source_value(&candidate.book_source)?;
    let raw = serde_json::to_string_pretty(&exported)?;
    let output_path = paths.task_path(task_id, "output/book-source.json");
    write_export_file(&output_path, &raw)?;
    Ok((paths.relative_to_task(task_id, &output_path), raw))
}

fn write_export_file(path: &Path, raw: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, raw)?;
    Ok(())
}

fn exportable_book_source_value(book_source: &BookSource) -> Result<Value> {
    let mut value = serde_json::to_value(book_source)?;

    if is_22biqu_mobile_template(book_source) {
        apply_22biqu_mobile_export_template(&mut value, book_source);
    }

    normalize_export_value_for_reader(&mut value);

    Ok(value)
}

fn normalize_export_value_for_reader(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_export_value_for_reader(nested);
            }
        }
        Value::Array(items) => {
            for nested in items {
                normalize_export_value_for_reader(nested);
            }
        }
        Value::String(text) => {
            *text = normalize_reader_rule_syntax(text);
        }
        _ => {}
    }
}

fn normalize_reader_rule_syntax(value: &str) -> String {
    let attr_regex = Regex::new(r"@attr\[([^\]]+)\]").expect("valid attr regex");
    attr_regex.replace_all(value, "@$1").into_owned()
}

fn is_22biqu_mobile_template(book_source: &BookSource) -> bool {
    let url = book_source.book_source_url.trim().to_ascii_lowercase();
    let Some(search) = book_source.rule_search.as_ref() else {
        return false;
    };
    let Some(book_info) = book_source.rule_book_info.as_ref() else {
        return false;
    };
    let Some(content) = book_source.rule_content.as_ref() else {
        return false;
    };

    (url.contains("22biqu") || url.contains("biqu"))
        && search.book_list.as_deref() == Some(".bookbox")
        && search.name.as_deref() == Some(".bookname a@text")
        && book_info.name.as_deref() == Some(".channelHeader .title@text")
        && content.content.as_deref() == Some("#chaptercontent@textNodes")
}

fn apply_22biqu_mobile_export_template(value: &mut Value, book_source: &BookSource) {
    let base_url = ensure_trailing_slash(&book_source.book_source_url);
    let host = url::Url::parse(&base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "m.22biqu.net".into());
    let host_pattern = regex::escape(&host);

    if let Some(map) = value.as_object_mut() {
        map.insert("bookSourceName".into(), Value::String("笔趣阁".into()));
        map.insert("bookSourceGroup".into(), Value::String("小说".into()));
        map.insert("bookSourceUrl".into(), Value::String(base_url.clone()));
        map.insert("bookSourceType".into(), Value::from(0));
        map.insert(
            "bookUrlPattern".into(),
            Value::String(format!("https?://{host_pattern}/biqu\\d+/")),
        );
        map.insert("enabledExplore".into(), Value::Bool(true));
        map.insert("customOrder".into(), Value::from(-99880));
        map.insert("weight".into(), Value::from(0));
        map.insert("respondTime".into(), Value::from(0));
        map.insert(
            "searchUrl".into(),
            Value::String(
                "{{cookie.removeCookie(source.getKey())}}/ss/,\n{\n  \"body\": \"searchkey={{key}}\",\n  \"method\": \"POST\"\n}"
                    .into(),
            ),
        );
        map.insert(
            "ruleSearch".into(),
            json!({
                "bookList": ".bookbox",
                "name": ".bookname@a@text",
                "author": ".author.0@text##作者：",
                "intro": Value::Null,
                "kind": ".author.1@text##类型：",
                "lastChapter": ".update@a@text##最后章节：",
                "updateTime": Value::Null,
                "bookUrl": ".bookname@a@href",
                "coverUrl": "img@src",
                "wordCount": Value::Null
            }),
        );
        map.insert(
            "ruleBookInfo".into(),
            json!({
                "init": "@put:{n:\"[property$=book_name]@content\",a:\"[property$=author]@content\",k:\"[property~=category|status|update_time]@content\",l:\"[property~=las?test_chapter_name]@content\",i:\"[property$=description]@content\",c:\"[property$=image]@content\"}",
                "name": "@get:{n}",
                "author": "@get:{a}",
                "intro": "@get:{i}",
                "kind": "@get:{k}",
                "lastChapter": "@get:{l}",
                "updateTime": Value::Null,
                "coverUrl": "@get:{c}",
                "wordCount": "",
                "tocUrl": Value::Null
            }),
        );
        map.insert(
            "ruleToc".into(),
            json!({
                "init": Value::Null,
                "chapterList": ".directoryArea p a",
                "chapterName": "text",
                "chapterUrl": "href",
                "nextTocUrl": "option@value"
            }),
        );
        map.insert(
            "ruleContent".into(),
            json!({
                "content": "#chaptercontent@html",
                "nextContentUrl": "text.下一页@href",
                "replaceRegex": "##\\\\s*({{ book.durChapterTitle }}|.*作者：.*|PS：.*求推荐！|PS：.*求收藏！|感谢.*打赏.*|感谢.*推荐票.*|感谢.*月票.*|（.*月票.*）|（为大家的.*票加更.*）|第二更在.*|为防止采集.*支持！|网址：sudugu\\\\.com|更多.*書吧看！|无错.*小说。|必应.*速读谷|loadAdv.*)\\\\s*"
            }),
        );
    }
}

fn ensure_trailing_slash(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
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
    let model_candidate = parse_candidate_from_openai(paths, sampled, &body)?;
    Ok(merge_candidates(inferred, model_candidate))
}

fn infer_candidate_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<SourceCandidate> {
    let search = infer_search_rule_from_sampled(paths, task_id, sampled)?;
    let book_info = infer_book_info_rule_from_sampled(paths, task_id, sampled)?;
    let toc = infer_toc_rule_from_sampled(paths, task_id, sampled)?;
    let content = infer_content_rule_from_sampled(paths, task_id, sampled)?;

    let mut evidence_refs = Vec::new();
    for role in ["search", "book", "toc", "content"] {
        evidence_refs.extend(sampled_page_evidence(role, sampled));
    }

    Ok(SourceCandidate {
        strategy: "sampled-dom-inference".into(),
        confidence: 0.78,
        evidence_refs,
        repair_history: Vec::new(),
        book_source: BookSource {
            book_source_name: sampled.site_label.clone(),
            book_source_url: sampled.base_url.clone(),
            enabled: Some(true),
            search_url: infer_search_url_from_sampled(paths, task_id, sampled)?,
            rule_search: Some(search),
            rule_book_info: Some(book_info),
            rule_toc: Some(toc),
            rule_content: Some(content),
        },
    })
}

fn infer_search_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<SearchRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "search") else {
        return Ok(SearchRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(SearchRule {
        book_list: first_matching_selector(
            &document,
            &[
                ".librarylist li",
                ".result-list .result-item",
                ".novellist .item",
                ".search-result li",
                ".book-list li",
                ".bookbox",
            ],
        ),
        name: first_matching_element_rule(
            &document,
            &[
                ".bookname a@text",
                ".bookinfo .bookname a@text",
                ".novelname@text",
                ".result-title a@text",
                ".book-title a@text",
                "h3 a@text",
                "a[href]@text",
            ],
        ),
        author: first_matching_element_rule(
            &document,
            &[
                ".info span:nth-child(2) a@text",
                ".info span:nth-child(2)@text",
                ".info a[href*=\"/author/\"]@text",
                ".author@text",
                ".writer@text",
                ".novelauthor@text",
                ".info@text",
                "p@text",
            ],
        ),
        cover_url: first_matching_element_rule(
            &document,
            &[
                ".pt-ll-l img@src",
                ".bookbox img@src",
                "img@src",
            ],
        ),
        intro: first_matching_element_rule(
            &document,
            &[
                ".intro@text",
                ".description@text",
                ".desc@text",
                ".bookintro@text",
                "p@text",
            ],
        ),
        book_url: first_matching_element_rule(
            &document,
            &[
                ".bookname a@href",
                ".bookinfo .bookname a@href",
                ".novelname@href",
                ".result-title a@href",
                ".book-title a@href",
                "h3 a@href",
                "a[href]@href",
            ],
        ),
        ..Default::default()
    })
}

fn infer_book_info_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<BookInfoRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "book") else {
        return Ok(BookInfoRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    let has_inline_toc = first_matching_selector(
        &document,
        &[
            ".recommend .directoryArea p",
            ".directoryArea p",
            ".fulldir .dirlist li",
            ".dirlist li",
            ".chapter-list li",
            ".listmain dd",
            "#list dd",
            ".booklist li",
        ],
    )
    .is_some();

    Ok(BookInfoRule {
        name: first_matching_document_rule(
            &document,
            &[
                ".channelHeader .title@text",
                "meta[property=\"og:novel:book_name\"]@content",
                "meta[property=\"og:title\"]@content",
                "#info h1@text",
                ".bookinfo h1@text",
                ".novelinfo h1@text",
                "h1@text",
            ],
        ),
        author: first_matching_document_rule(
            &document,
            &[
                ".synopsisArea_detail .author@text",
                "meta[property=\"og:novel:author\"]@content",
                ".novelinfo-l li a@text",
                ".meta .author@text",
                "#info p a@text",
                ".author a@text",
                ".author@text",
            ],
        ),
        cover_url: first_matching_document_rule(
            &document,
            &[
                ".novelinfo-r img@src",
                "meta[property=\"og:image\"]@content",
                ".book-cover img@src",
                ".cover img@src",
                "img@src",
            ],
        ),
        intro: first_matching_document_rule(
            &document,
            &[
                ".review@text",
                "meta[property=\"og:description\"]@content",
                ".novelintro@text",
                "#intro@text",
                ".intro@text",
                ".bookintro@text",
                ".desc@text",
            ],
        ),
        toc_url: if has_inline_toc {
            None
        } else {
            first_matching_document_rule(
                &document,
                &[
                    ".toc-link@href",
                    "a[href*=\"list\"]@href",
                    "a[href*=\"catalog\"]@href",
                ],
            )
        },
        ..Default::default()
    })
}

fn infer_toc_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<TocRule> {
    let Some(page) = sampled
        .pages
        .iter()
        .find(|page| page.role == "toc" || page.role == "book")
    else {
        return Ok(TocRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(TocRule {
        chapter_list: first_matching_selector(
            &document,
            &[
                ".recommend .directoryArea p",
                ".directoryArea p",
                ".fulldir .dirlist li",
                ".dirlist li",
                ".chapter-list li",
                ".listmain dd",
                "#list dd",
                ".booklist li",
            ],
        ),
        chapter_name: Some("a@text".into()),
        chapter_url: Some("a@href".into()),
        ..Default::default()
    })
}

fn infer_content_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<ContentRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "content") else {
        return Ok(ContentRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(ContentRule {
        content: first_matching_document_rule(
            &document,
            &[
                "#chaptercontent@textNodes",
                "#content@textNodes",
                ".content@textNodes",
                ".txt@textNodes",
                ".read-content@textNodes",
                ".article-content@textNodes",
            ],
        ),
        next_content_url: first_matching_document_rule(
            &document,
            &[
                "#pt_next@href",
                ".Readpage #pt_next@href",
                ".next a@href",
                "a.next@href",
                "a#next_url@href",
            ],
        ),
        replace_regex: None,
    })
}

fn sampled_document(
    paths: &AppPaths,
    task_id: &str,
    page: &crate::domain::PageSample,
) -> Result<Html> {
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    Ok(Html::parse_document(&html))
}

fn first_matching_selector(document: &Html, selectors: &[&str]) -> Option<String> {
    selectors.iter().find_map(|candidate| {
        let Ok(selector) = Selector::parse(candidate) else {
            return None;
        };
        document
            .select(&selector)
            .next()
            .map(|_| (*candidate).to_string())
    })
}

fn first_matching_document_rule(document: &Html, rules: &[&str]) -> Option<String> {
    rules.iter().find_map(|rule| {
        evaluate_on_document_safe(document, rule)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|_| (*rule).to_string())
    })
}

fn first_matching_element_rule(document: &Html, rules: &[&str]) -> Option<String> {
    let containers = [
        ".librarylist li",
        ".result-list .result-item",
        ".novellist .item",
        ".search-result li",
        ".book-list li",
        ".bookbox",
    ];

    for container_rule in containers {
        let Ok(container_selector) = Selector::parse(container_rule) else {
            continue;
        };
        let Some(element) = document.select(&container_selector).next() else {
            continue;
        };
        for rule in rules {
            if evaluate_on_element_safe(&element, rule)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
            {
                return Some((*rule).to_string());
            }
        }
    }

    None
}

fn merge_candidates(mut inferred: SourceCandidate, model: SourceCandidate) -> SourceCandidate {
    inferred.strategy = format!("{} + {}", inferred.strategy, model.strategy);
    inferred.confidence = inferred.confidence.max(model.confidence);
    inferred.evidence_refs.extend(model.evidence_refs);
    inferred.repair_history.extend(model.repair_history);

    let model_source = model.book_source;
    if inferred.book_source.book_source_name.trim().is_empty() {
        inferred.book_source.book_source_name = model_source.book_source_name;
    }
    if inferred.book_source.book_source_url.trim().is_empty() {
        inferred.book_source.book_source_url = model_source.book_source_url;
    }
    if inferred.book_source.search_url.as_deref().unwrap_or_default().trim().is_empty() {
        inferred.book_source.search_url = model_source.search_url;
    }

    merge_search_rule(
        inferred.book_source.rule_search.get_or_insert_with(SearchRule::default),
        model_source.rule_search,
    );
    merge_book_info_rule(
        inferred
            .book_source
            .rule_book_info
            .get_or_insert_with(BookInfoRule::default),
        model_source.rule_book_info,
    );
    merge_toc_rule(
        inferred.book_source.rule_toc.get_or_insert_with(TocRule::default),
        model_source.rule_toc,
    );
    merge_content_rule(
        inferred
            .book_source
            .rule_content
            .get_or_insert_with(ContentRule::default),
        model_source.rule_content,
    );

    inferred
}

fn merge_search_rule(target: &mut SearchRule, source: Option<SearchRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.book_list, source.book_list);
    fill_option(&mut target.name, source.name);
    fill_option(&mut target.author, source.author);
    fill_option(&mut target.intro, source.intro);
    fill_option(&mut target.kind, source.kind);
    fill_option(&mut target.last_chapter, source.last_chapter);
    fill_option(&mut target.update_time, source.update_time);
    fill_option(&mut target.book_url, source.book_url);
    fill_option(&mut target.cover_url, source.cover_url);
    fill_option(&mut target.word_count, source.word_count);
}

fn merge_book_info_rule(target: &mut BookInfoRule, source: Option<BookInfoRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.init, source.init);
    fill_option(&mut target.name, source.name);
    fill_option(&mut target.author, source.author);
    fill_option(&mut target.intro, source.intro);
    fill_option(&mut target.kind, source.kind);
    fill_option(&mut target.last_chapter, source.last_chapter);
    fill_option(&mut target.update_time, source.update_time);
    fill_option(&mut target.cover_url, source.cover_url);
    fill_option(&mut target.word_count, source.word_count);
    fill_option(&mut target.toc_url, source.toc_url);
}

fn merge_toc_rule(target: &mut TocRule, source: Option<TocRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.init, source.init);
    fill_option(&mut target.chapter_list, source.chapter_list);
    fill_option(&mut target.chapter_name, source.chapter_name);
    fill_option(&mut target.chapter_url, source.chapter_url);
    fill_option(&mut target.next_toc_url, source.next_toc_url);
}

fn merge_content_rule(target: &mut ContentRule, source: Option<ContentRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.content, source.content);
    fill_option(&mut target.next_content_url, source.next_content_url);
    fill_option(&mut target.replace_regex, source.replace_regex);
}

fn fill_option(target: &mut Option<String>, source: Option<String>) {
    if target.as_deref().unwrap_or_default().trim().is_empty() {
        *target = source.filter(|value| !value.trim().is_empty());
    }
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
    expected_keyword: Option<&str>,
) -> Result<ValidationReport> {
    ensure_compatibility(&candidate.book_source)?;

    let search = validate_search(paths, task_id, sampled, candidate, expected_keyword)?;
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
    expected_keyword: Option<&str>,
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
    if !search_url.contains("{key}") || !search_url.contains("{page}") {
        return Ok(failed_phase(
            "searchUrl 必须包含 {key} 和 {page}",
            vec!["当前 searchUrl 不具备最小替换能力。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    if rule.book_list.as_deref().unwrap_or_default().trim().is_empty() {
        return Ok(failed_phase(
            "ruleSearch.bookList missing",
            vec!["bookList selector is empty".into()],
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

    let mut extracted = Vec::new();
    for item in &items {
        let name = evaluate_on_element_safe(item, rule.name.as_deref().unwrap_or_default())?;
        let book_url =
            evaluate_on_element_safe(item, rule.book_url.as_deref().unwrap_or_default())?;
        let resolved = normalize_url(&page.url, &book_url)?;
        if !name.trim().is_empty() && !resolved.trim().is_empty() {
            extracted.push((name, resolved));
        }
    }

    if extracted.is_empty() {
        return Ok(failed_phase(
            "搜索结果缺少书名或详情链接",
            vec!["name 或 bookUrl 规则没有提取到有效结果。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let expected_keyword = expected_keyword.filter(|value| !value.trim().is_empty());
    let best = if let Some(keyword) = expected_keyword {
        extracted
            .iter()
            .enumerate()
            .map(|(index, (name, resolved))| {
                (
                    index,
                    name,
                    resolved,
                    keyword_match_score(name, keyword),
                )
            })
            .max_by(|left, right| {
                left.3
                    .cmp(&right.3)
                    .then_with(|| right.1.len().cmp(&left.1.len()))
            })
    } else {
        extracted
            .iter()
            .enumerate()
            .map(|(index, (name, resolved))| (index, name, resolved, 1))
            .next()
    };

    let Some((_, name, resolved, score)) = best else {
        return Ok(failed_phase(
            "搜索结果缺少有效候选",
            vec!["无法从搜索结果中确定可用的候选书籍。".into()],
            sampled_page_evidence("search", sampled),
        ));
    };

    if expected_keyword.is_some() && score <= 0 {
        return Ok(failed_phase(
            "搜索结果未命中测试关键词",
            vec![format!("没有找到书名匹配测试关键词的结果：{}", expected_keyword.unwrap_or_default())],
            sampled_page_evidence("search", sampled),
        ));
    }

    if let Ok(book_page) = sampled_page(sampled, "book") {
        let sampled_book_url = canonical_compare_url(&book_page.url)?;
        let expected_book_url = canonical_compare_url(resolved)?;
        if sampled_book_url != expected_book_url {
            return Ok(failed_phase(
                "采样详情页与搜索最佳匹配不一致",
                vec![
                    format!("测试关键词最佳匹配结果：{name}"),
                    format!("最佳匹配链接：{resolved}"),
                    format!("实际采样详情页：{}", book_page.url),
                ],
                sampled_page_evidence("search", sampled),
            ));
        }
    }

    Ok(passed_phase(
        format!("搜索规则通过，共命中 {} 条结果。", extracted.len()),
        vec![
            format!("最佳匹配书名：{name}"),
            format!("最佳匹配链接：{resolved}"),
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

    let name = evaluate_on_document_safe(&document, rule.name.as_deref().unwrap_or_default())?;
    let author =
        evaluate_on_document_safe(&document, rule.author.as_deref().unwrap_or_default())?;
    let intro = evaluate_on_document_safe(&document, rule.intro.as_deref().unwrap_or_default())?;
    let toc_url =
        evaluate_on_document_safe(&document, rule.toc_url.as_deref().unwrap_or_default())?;
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
    if rule.chapter_list.as_deref().unwrap_or_default().trim().is_empty() {
        return Ok(failed_phase(
            "ruleToc.chapterList missing",
            vec!["chapterList selector is empty".into()],
            sampled_page_evidence("toc", sampled),
        ));
    }
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
        evaluate_on_element_safe(&first, rule.chapter_name.as_deref().unwrap_or_default())?;
    let chapter_url =
        normalize_url(
            &page.url,
            &evaluate_on_element_safe(&first, rule.chapter_url.as_deref().unwrap_or_default())?,
        )?;

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
    let content =
        evaluate_on_document_safe(&document, rule.content.as_deref().unwrap_or_default())?;

    if content.trim().is_empty() {
        return Ok(failed_phase(
            "正文为空",
            vec!["ruleContent.content 没有提取到正文。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    let next_url = evaluate_on_document_safe(
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
    let Some(rule) = candidate.book_source.rule_content.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleContent",
            vec!["当前候选缺少正文规则，无法执行清洗验证。".into()],
            sampled_page_evidence("content", sampled),
        ));
    };
    let first_page = match sampled_page(sampled, "content") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少正文页样本",
                vec!["当前任务未采样到正文页，暂时无法执行清洗验证。".into()],
                sampled_page_evidence("toc", sampled),
            ));
        }
    };
    let first_html = fs::read_to_string(paths.task_path(task_id, &first_page.html_path))?;
    let first_doc = Html::parse_document(&first_html);
    let mut combined =
        evaluate_on_document_safe(&first_doc, rule.content.as_deref().unwrap_or_default())?;

    if let Some(next_page) = sampled.pages.iter().find(|page| page.role == "content-next") {
        let next_html = fs::read_to_string(paths.task_path(task_id, &next_page.html_path))?;
        let next_doc = Html::parse_document(&next_html);
        let next_content =
            evaluate_on_document_safe(&next_doc, rule.content.as_deref().unwrap_or_default())?;
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

fn try_repair_candidate_with_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &mut SourceCandidate,
    report: &ValidationReport,
) -> Result<bool> {
    let inferred = infer_candidate_from_sampled(paths, task_id, sampled)?;
    let mut repaired = false;

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
                "{resolved}{separator}{query_name}={{key}}&page={{page}}"
            )));
        }

        if role == "search" {
            let Ok(mut url) = Url::parse(&page.url) else {
                continue;
            };
            let key_name = url
                .query_pairs()
                .find_map(|(name, _)| {
                    let lower = name.to_ascii_lowercase();
                    if lower.contains("key")
                        || lower.contains("search")
                        || lower.contains("query")
                        || lower == "wd"
                    {
                        Some(name.into_owned())
                    } else {
                        None
                    }
                });

            let Some(key_name) = key_name else {
                continue;
            };

            let existing_page = url
                .query_pairs()
                .any(|(name, _)| name.eq_ignore_ascii_case("page"));
            let mut pairs = vec![(key_name, "{key}".to_string())];
            if existing_page {
                pairs.push(("page".into(), "{page}".into()));
            }

            url.query_pairs_mut().clear().extend_pairs(pairs);
            let rendered = url
                .to_string()
                .replace("%7Bkey%7D", "{key}")
                .replace("%7Bpage%7D", "{page}");
            return Ok(Some(rendered));
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

fn evaluate_on_document_safe(document: &Html, rule: &str) -> Result<String> {
    match evaluate_on_document(document, rule) {
        Ok(value) => Ok(value),
        Err(error) if error.to_string().contains("selector did not match") => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn evaluate_on_element_safe(element: &ElementRef<'_>, rule: &str) -> Result<String> {
    match evaluate_on_element(element, rule) {
        Ok(value) => Ok(value),
        Err(error) if error.to_string().contains("selector did not match") => Ok(String::new()),
        Err(error) => Err(error),
    }
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
    if let Some((selector, attr)) = base.rsplit_once('@') {
        let attr = attr.trim();
        if !selector.trim().is_empty() && !attr.is_empty() {
            return ParsedRule {
                selector: selector.trim(),
                mode: match attr {
                    "text" => RuleMode::Text,
                    "html" => RuleMode::Html,
                    "textNodes" => RuleMode::TextNodes,
                    other => RuleMode::Attr(other.to_string()),
                },
            };
        }
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

fn canonical_compare_url(url: &str) -> Result<String> {
    let mut parsed = Url::parse(url)?;
    if parsed.path() != "/" {
        let trimmed = parsed.path().trim_end_matches('/').to_string();
        parsed.set_path(if trimmed.is_empty() { "/" } else { &trimmed });
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn keyword_match_score(title: &str, keyword: &str) -> i32 {
    let normalized_title = normalize_match_text(title);
    let normalized_keyword = normalize_match_text(keyword);

    if normalized_keyword.is_empty() || normalized_title.is_empty() {
        return 0;
    }

    if normalized_title == normalized_keyword {
        100
    } else if normalized_title.starts_with(&normalized_keyword) {
        80
    } else if normalized_title.contains(&normalized_keyword) {
        60
    } else {
        0
    }
}

fn normalize_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
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
    use std::fs;

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
                search_url: Some("https://fixture.novel.world/search?q={key}&page={page}".into()),
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
                            "search_url":"https://biquge.xin/search?q={key}&page={page}"
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
            Some("https://biquge.xin/search?q={key}&page={page}")
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

    #[test]
    fn inferred_rules_handle_biquge_style_templates() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-biquge";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();

        fs::write(
            task_root.join("dom/search.html"),
            r#"
            <html><body>
              <ul class="librarylist">
                <li>
                  <a class="novelname" href="/book/9058/">剑来</a>
                  <div class="author">烽火戏诸侯</div>
                  <div class="intro">大千世界，无奇不有。</div>
                </li>
              </ul>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            task_root.join("dom/search-mobile.html"),
            r#"
            <html><body>
              <div class="read_book">
                <div class="bookbox">
                  <div class="bookinfo">
                    <h4 class="bookname"><a href="/biqu42106/">星门</a></h4>
                    <div class="author">作者：老鹰吃小鸡</div>
                  </div>
                </div>
              </div>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            task_root.join("dom/book.html"),
            r#"
            <html><body>
              <h1>剑来</h1>
              <ul class="novelinfo-l"><li><a href="/author/1">烽火戏诸侯</a></li></ul>
              <div class="novelintro">这是一本长篇小说简介。</div>
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/9058/1.html">第一章 小镇</a></li>
                  <li><a href="/book/9058/2.html">第二章 少年</a></li>
                </ul>
              </div>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            task_root.join("dom/toc.html"),
            r#"
            <html><body>
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/9058/1.html">第一章 小镇</a></li>
                  <li><a href="/book/9058/2.html">第二章 少年</a></li>
                </ul>
              </div>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            task_root.join("dom/content.html"),
            r#"
            <html><body>
              <div id="content">
                这里是第一段正文。
                这里是第二段正文。
                这里是第三段正文。
                这里是第四段正文。
              </div>
            </body></html>
            "#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };

        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "biquge.xin".into(),
            base_url: "https://biquge.xin".into(),
            pages: vec![
                crate::domain::PageSample {
                    role: "search".into(),
                    url: "https://biquge.xin/search/result.html?searchkey=%E5%89%91%E6%9D%A5&page=1".into(),
                    html_path: "dom/search.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
                crate::domain::PageSample {
                    role: "book".into(),
                    url: "https://biquge.xin/book/9058/".into(),
                    html_path: "dom/book.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
                crate::domain::PageSample {
                    role: "toc".into(),
                    url: "https://biquge.xin/book/9058/".into(),
                    html_path: "dom/toc.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
                crate::domain::PageSample {
                    role: "content".into(),
                    url: "https://biquge.xin/book/9058/1.html".into(),
                    html_path: "dom/content.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
            ],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };

        let candidate = infer_candidate_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(
            candidate.book_source.search_url.as_deref(),
            Some("https://biquge.xin/search/result.html?searchkey={key}&page={page}")
        );
        assert_eq!(
            candidate
                .book_source
                .rule_search
                .as_ref()
                .and_then(|rule| rule.book_list.as_deref()),
            Some(".librarylist li")
        );
        assert_eq!(
            candidate
                .book_source
                .rule_book_info
                .as_ref()
                .and_then(|rule| rule.intro.as_deref()),
            Some(".novelintro@text")
        );
        assert_eq!(
            candidate
                .book_source
                .rule_toc
                .as_ref()
                .and_then(|rule| rule.chapter_list.as_deref()),
            Some(".fulldir .dirlist li")
        );
        assert_eq!(
            candidate
                .book_source
                .rule_content
                .as_ref()
                .and_then(|rule| rule.content.as_deref()),
            Some("#content@textNodes")
        );

        let report = validate_candidate(&paths, task_id, &sampled, &candidate, Some("剑来")).unwrap();
        assert!(report.search.passed);
        assert!(report.book_info.passed);
        assert!(report.toc.passed);
        assert!(report.content.passed);
        assert!(report.cleanup.passed);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_search_rule_handles_mobile_biquge_cards() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-biquge-mobile";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();
        fs::write(
            task_root.join("dom/search.html"),
            r#"
            <html><body>
              <div class="read_book">
                <div class="bookbox">
                  <div class="bookinfo">
                    <h4 class="bookname"><a href="/biqu42106/">星门</a></h4>
                    <div class="author">作者：老鹰吃小鸡</div>
                  </div>
                </div>
              </div>
            </body></html>
            "#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };
        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "m.22biqu.net".into(),
            base_url: "https://m.22biqu.net".into(),
            pages: vec![crate::domain::PageSample {
                role: "search".into(),
                url: "https://m.22biqu.net/ss/?searchkey=%E6%98%9F%E9%97%A8&page=1".into(),
                html_path: "dom/search.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            }],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };

        let rule = infer_search_rule_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(rule.book_list.as_deref(), Some(".bookbox"));
        assert_eq!(rule.name.as_deref(), Some(".bookname a@text"));
        assert_eq!(rule.book_url.as_deref(), Some(".bookname a@href"));
        assert_eq!(rule.cover_url.as_deref(), None);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_book_info_handles_mobile_biquge_detail_page() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-biquge-detail";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();
        fs::write(
            task_root.join("dom/book.html"),
            r#"
            <html><body>
              <header class="channelHeader"><span class="title">星门</span></header>
              <div class="synopsisArea_detail">
                <b class="author">作者：老鹰吃小鸡</b>
              </div>
              <div class="review">这是一本简介。</div>
              <div class="btn"><a href="/biqu42106/123.html">开始阅读</a></div>
              <div id="chapterlist"><p><a href="/biqu42106/456.html">最新章节</a></p></div>
              <div class="directoryArea"><p><a href="/biqu42106/123.html">第一章</a></p></div>
            </body></html>
            "#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };
        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "m.22biqu.net".into(),
            base_url: "https://m.22biqu.net".into(),
            pages: vec![crate::domain::PageSample {
                role: "book".into(),
                url: "https://m.22biqu.net/biqu42106/".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            }],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };

        let book_rule = infer_book_info_rule_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(book_rule.name.as_deref(), Some(".channelHeader .title@text"));
        assert_eq!(book_rule.author.as_deref(), Some(".synopsisArea_detail .author@text"));
        assert_eq!(book_rule.intro.as_deref(), Some(".review@text"));
        assert_eq!(book_rule.cover_url.as_deref(), None);
        assert_eq!(book_rule.toc_url.as_deref(), None);

        let toc_rule = infer_toc_rule_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(toc_rule.chapter_list.as_deref(), Some(".directoryArea p"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn inferred_content_rule_handles_mobile_reader_page() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-biquge-reader";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();
        fs::write(
            task_root.join("dom/content.html"),
            r#"
            <html><body>
              <div id="chaptercontent">
                <p>第一段正文。</p>
                <p>第二段正文。</p>
                <p>第三段正文。</p>
              </div>
              <p class="Readpage"><a id="pt_next" href="/biqu42106/123_2.html">下一页</a></p>
            </body></html>
            "#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };
        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "m.22biqu.net".into(),
            base_url: "https://m.22biqu.net".into(),
            pages: vec![crate::domain::PageSample {
                role: "content".into(),
                url: "https://m.22biqu.net/biqu42106/123.html".into(),
                html_path: "dom/content.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            }],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };

        let rule = infer_content_rule_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(rule.content.as_deref(), Some("#chaptercontent@textNodes"));
        assert_eq!(rule.next_content_url.as_deref(), Some("#pt_next@href"));

        let candidate = SourceCandidate {
            strategy: "test".into(),
            confidence: 1.0,
            evidence_refs: vec![],
            repair_history: vec![],
            book_source: BookSource {
                book_source_name: "m.22biqu.net".into(),
                book_source_url: "https://m.22biqu.net".into(),
                enabled: Some(true),
                search_url: None,
                rule_search: None,
                rule_book_info: None,
                rule_toc: None,
                rule_content: Some(rule),
            },
        };

        let report = validate_content(&paths, task_id, &sampled, &candidate).unwrap();
        assert!(report.passed);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn export_22biqu_mobile_template_uses_reader_compatible_rules() {
        let source = BookSource {
            book_source_name: "m.22biqu.net".into(),
            book_source_url: "https://m.22biqu.net".into(),
            enabled: Some(true),
            search_url: Some("https://m.22biqu.net/ss/?searchkey={key}&page={page}".into()),
            rule_search: Some(SearchRule {
                book_list: Some(".bookbox".into()),
                name: Some(".bookname a@text".into()),
                book_url: Some(".bookname a@href".into()),
                ..Default::default()
            }),
            rule_book_info: Some(BookInfoRule {
                name: Some(".channelHeader .title@text".into()),
                ..Default::default()
            }),
            rule_toc: Some(TocRule {
                chapter_list: Some(".directoryArea p".into()),
                ..Default::default()
            }),
            rule_content: Some(ContentRule {
                content: Some("#chaptercontent@textNodes".into()),
                ..Default::default()
            }),
        };

        let exported = exportable_book_source_value(&source).unwrap();
        assert_eq!(
            exported.get("searchUrl").and_then(Value::as_str),
            Some("{{cookie.removeCookie(source.getKey())}}/ss/,\n{\n  \"body\": \"searchkey={{key}}\",\n  \"method\": \"POST\"\n}")
        );
        assert_eq!(
            exported.pointer("/ruleSearch/name").and_then(Value::as_str),
            Some(".bookname@a@text")
        );
        assert_eq!(
            exported.pointer("/ruleToc/chapterList").and_then(Value::as_str),
            Some(".directoryArea p a")
        );
        assert_eq!(
            exported.pointer("/ruleContent/content").and_then(Value::as_str),
            Some("#chaptercontent@html")
        );
        assert_eq!(
            exported.get("bookUrlPattern").and_then(Value::as_str),
            Some("https?://m\\.22biqu\\.net/biqu\\d+/")
        );
    }

    #[test]
    fn export_value_normalizes_attr_syntax_for_reader() {
        let source = BookSource {
            book_source_name: "biquge.xin".into(),
            book_source_url: "https://biquge.xin".into(),
            enabled: Some(true),
            search_url: Some("https://biquge.xin/search/result.html?searchkey={key}&page={page}".into()),
            rule_search: Some(SearchRule {
                book_list: Some(".librarylist li".into()),
                name: Some(".novelname@text".into()),
                author: Some(".info span:nth-child(2) a@text".into()),
                book_url: Some(".novelname@attr[href]".into()),
                cover_url: Some(".pt-ll-l img@attr[src]".into()),
                ..Default::default()
            }),
            rule_book_info: Some(BookInfoRule {
                name: Some("meta[property=\"og:novel:book_name\"]@attr[content]".into()),
                author: Some("meta[property=\"og:novel:author\"]@attr[content]".into()),
                cover_url: Some("meta[property=\"og:image\"]@attr[content]".into()),
                ..Default::default()
            }),
            rule_toc: Some(TocRule {
                chapter_list: Some(".fulldir .dirlist li".into()),
                chapter_name: Some("a@text".into()),
                chapter_url: Some("a@attr[href]".into()),
                ..Default::default()
            }),
            rule_content: Some(ContentRule {
                content: Some("#chaptercontent@textNodes".into()),
                next_content_url: Some("#pt_next@attr[href]".into()),
                ..Default::default()
            }),
        };

        let exported = exportable_book_source_value(&source).unwrap();
        assert_eq!(
            exported.pointer("/ruleSearch/bookUrl").and_then(Value::as_str),
            Some(".novelname@href")
        );
        assert_eq!(
            exported.pointer("/ruleSearch/coverUrl").and_then(Value::as_str),
            Some(".pt-ll-l img@src")
        );
        assert_eq!(
            exported.pointer("/ruleBookInfo/author").and_then(Value::as_str),
            Some("meta[property=\"og:novel:author\"]@content")
        );
        assert_eq!(
            exported.pointer("/ruleBookInfo/coverUrl").and_then(Value::as_str),
            Some("meta[property=\"og:image\"]@content")
        );
        assert_eq!(
            exported.pointer("/ruleToc/chapterUrl").and_then(Value::as_str),
            Some("a@href")
        );
        assert_eq!(
            exported.pointer("/ruleContent/nextContentUrl").and_then(Value::as_str),
            Some("#pt_next@href")
        );
    }

    #[test]
    fn export_task_source_also_writes_to_selected_path() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };

        fs::create_dir_all(&paths.tasks_dir).unwrap();
        fs::create_dir_all(&paths.logs_dir).unwrap();
        init_database(&paths).unwrap();

        let mut task = TaskRecord::new(crate::domain::TaskDraft {
            target_url: "https://m.22biqu.net".into(),
            test_keyword: Some("星门".into()),
        });
        task.source_candidate = Some(SourceCandidate {
            strategy: "test".into(),
            confidence: 1.0,
            evidence_refs: vec![],
            repair_history: vec![],
            book_source: good_candidate().book_source,
        });
        task.artifact_manifest = Some(ArtifactManifest::default());

        paths.ensure_task_layout(&task.id).unwrap();
        persist_task(&paths, &task).unwrap();

        let selected_path = root.join("exports").join("book-source.json");
        let exported = export_task_source(
            &paths,
            &task.id,
            Some(selected_path.to_string_lossy().as_ref()),
        )
        .unwrap();

        assert_eq!(fs::read_to_string(&selected_path).unwrap(), exported);
        assert!(paths.task_path(&task.id, "output/book-source.json").exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn delete_task_removes_database_row_and_task_directory() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };

        fs::create_dir_all(&paths.tasks_dir).unwrap();
        fs::create_dir_all(&paths.logs_dir).unwrap();
        init_database(&paths).unwrap();

        let task = create_task(
            &paths,
            crate::domain::TaskDraft {
                target_url: "fixture://novel-world".into(),
                test_keyword: Some("星门".into()),
            },
        )
        .unwrap();
        fs::write(paths.task_path(&task.id, "output/temp.json"), "{}").unwrap();

        let state = AppState {
            paths: paths.clone(),
            runtime: Arc::new(TaskRuntime::default()),
        };

        delete_task(&state, &task.id).unwrap();

        assert!(get_task(&paths, &task.id).is_err());
        assert!(!paths.task_dir(&task.id).exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn validate_search_rejects_when_sampled_book_is_not_best_keyword_match() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-search-mismatch";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();

        fs::write(
            task_root.join("dom/search.html"),
            r#"
            <html><body>
              <ul class="librarylist">
                <li><a class="novelname" href="/book/9058/">拉格朗日星门</a></li>
                <li><a class="novelname" href="/book/44021/">星门</a></li>
              </ul>
            </body></html>
            "#,
        )
        .unwrap();
        fs::write(
            task_root.join("dom/book.html"),
            r#"<html><body><h1>拉格朗日星门</h1></body></html>"#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };
        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "biquge.xin".into(),
            base_url: "https://biquge.xin".into(),
            pages: vec![
                crate::domain::PageSample {
                    role: "search".into(),
                    url: "https://biquge.xin/search/result.html?searchkey=%E6%98%9F%E9%97%A8&page=1".into(),
                    html_path: "dom/search.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
                crate::domain::PageSample {
                    role: "book".into(),
                    url: "https://biquge.xin/book/9058/".into(),
                    html_path: "dom/book.html".into(),
                    screenshot_path: None,
                    snippet_paths: vec![],
                },
            ],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };
        let candidate = SourceCandidate {
            strategy: "test".into(),
            confidence: 0.9,
            evidence_refs: vec![],
            repair_history: vec![],
            book_source: BookSource {
                book_source_name: "biquge.xin".into(),
                book_source_url: "https://biquge.xin".into(),
                enabled: Some(true),
                search_url: Some("https://biquge.xin/search/result.html?searchkey={key}&page={page}".into()),
                rule_search: Some(SearchRule {
                    book_list: Some(".librarylist li".into()),
                    name: Some(".novelname@text".into()),
                    book_url: Some(".novelname@attr[href]".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        };

        let phase = validate_search(&paths, task_id, &sampled, &candidate, Some("星门")).unwrap();
        assert!(!phase.passed);
        assert!(phase.summary.contains("采样详情页与搜索最佳匹配不一致"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn infer_book_info_rule_omits_toc_url_when_book_page_contains_inline_toc() {
        let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
        let task_id = "task-inline-toc";
        let task_root = root.join("tasks").join(task_id);
        fs::create_dir_all(task_root.join("dom")).unwrap();
        fs::write(
            task_root.join("dom/book.html"),
            r#"
            <html><body>
              <meta property="og:novel:author" content="老鹰吃小鸡" />
              <meta property="og:novel:book_name" content="星门" />
              <meta property="og:description" content="简介" />
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/44021/1.html">第一章</a></li>
                  <li><a href="/book/44021/2.html">第二章</a></li>
                </ul>
              </div>
            </body></html>
            "#,
        )
        .unwrap();

        let paths = AppPaths {
            data_dir: root.clone(),
            db_path: root.join("app.sqlite"),
            tasks_dir: root.join("tasks"),
            logs_dir: root.join("logs"),
            settings_path: root.join("settings.json"),
            fixtures_dir: root.join("fixtures"),
            worker_script: root.join("worker.mjs"),
        };
        let sampled = SamplingResult {
            fixture_name: None,
            site_label: "biquge.xin".into(),
            base_url: "https://biquge.xin".into(),
            pages: vec![crate::domain::PageSample {
                role: "book".into(),
                url: "https://biquge.xin/book/44021/".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            }],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        };

        let rule = infer_book_info_rule_from_sampled(&paths, task_id, &sampled).unwrap();
        assert_eq!(rule.toc_url.as_deref(), None);

        let _ = fs::remove_dir_all(root);
    }
}
