use std::cmp::Reverse;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use chrono::Utc;
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rusqlite::{params, Connection};
use scraper::{Html, Selector};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::domain::{
    AiSettings, ArtifactBundle, ArtifactFile, ArtifactManifest, BookInfoRule, BookSource,
    ContentRule, EvidenceRef, ExploreRule, LogEntry, PageForm, PageInspection, RepairStep,
    SamplingResult, SearchRule, SourceCandidate, StartAiSettings, TaskMode, TaskRecord, TaskStatus,
    TocRule, ValidationPhase, ValidationReport,
};
use crate::state::{AppPaths, AppState, TaskRuntime};
use crate::worker;

#[path = "services/agent_flow.rs"]
mod agent_flow;
#[path = "services/ai_runtime.rs"]
mod ai_runtime;
#[path = "services/candidate_parser.rs"]
mod candidate_parser;
#[path = "services/chat.rs"]
mod chat;
#[path = "services/common.rs"]
mod common;
#[path = "services/exporter.rs"]
mod exporter;
#[path = "services/inference.rs"]
mod inference;
#[path = "services/merge.rs"]
mod merge;
#[path = "services/repair.rs"]
mod repair;
#[path = "services/rule_syntax.rs"]
mod rule_syntax;
#[path = "services/sampling_agent.rs"]
mod sampling_agent;
#[path = "services/task_io.rs"]
mod task_io;
#[path = "services/task_repo.rs"]
mod task_repo;
#[path = "services/task_runner.rs"]
mod task_runner;
#[path = "services/validation.rs"]
mod validation;
#[allow(unused_imports)]
use agent_flow::{
    agent_has_minimum_coverage, build_observed_action_inputs, canonical_agent_key,
    infer_followup_book_url, infer_followup_book_url_from_dom, infer_followup_content_url,
    infer_followup_content_url_from_dom, infer_followup_toc_url, infer_followup_toc_url_from_dom,
    load_inspections_from_sampled, load_sampled_from_task, merge_inspection, normalize_agent_role,
    request_agent_plan, validate_agent_action,
};
#[allow(unused_imports)]
use ai_runtime::{
    build_repair_prompt, log_to_task_file, normalize_openai_endpoint, request_model_message,
    synthesize_candidate,
};
use candidate_parser::{parse_candidate_from_openai, sanitize_candidate};
#[allow(unused_imports)]
use common::{canonical_compare_url, keyword_match_score, normalize_url, sampled_page};
#[allow(unused_imports)]
use exporter::{append_output, export_candidate, exportable_book_source_value, write_export_file};
#[allow(unused_imports)]
use inference::{
    first_matching_document_rule, infer_book_info_rule_from_sampled, infer_candidate_from_sampled,
    infer_content_rule_from_sampled, infer_search_rule_from_sampled, infer_search_url_from_sampled,
    infer_toc_rule_from_sampled, sampled_document,
};
#[allow(unused_imports)]
use merge::merge_candidates;
#[allow(unused_imports)]
use repair::{try_repair_candidate_with_model, try_repair_candidate_with_sampled};
use rule_syntax::{
    evaluate_on_document_safe, evaluate_on_element_safe, normalize_export_value_for_reader,
    normalize_reader_rule_syntax, selector,
};
#[allow(unused_imports)]
use sampling_agent::run_agent_sampling;
#[allow(unused_imports)]
use task_io::{
    append_log, content_type_for, finalize_pending_delete, persist_and_flush, read_files,
    write_logs,
};
#[allow(unused_imports)]
use task_repo::{
    create_task as create_task_impl, delete_task as delete_task_impl, get_task as get_task_impl,
    init_database as init_database_impl, is_task_cancelled, list_tasks as list_tasks_impl,
    load_settings as load_settings_impl, persist_task, purge_task_storage,
    save_settings as save_settings_impl,
};
#[allow(unused_imports)]
use validation::{
    apply_replace_regex, failed_phase, first_failure_summary, passed_phase, report_is_usable,
    report_requires_repair, sampled_page_evidence, validate_candidate, validate_content,
    validate_search,
};

const PLAYWRIGHT_BOOK_SOURCE_SKILL: &str = include_str!("agent_skills/playwright-book-source.md");
const PLAYWRIGHT_DISCOVERY_SKILL: &str = include_str!("agent_skills/playwright-discovery.md");

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FixtureResponse {
    strategy: String,
    confidence: Value,
    #[serde(default)]
    evidence_refs: Value,
    #[serde(default)]
    repair_history: Value,
    book_source: Value,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentPlanResponse {
    summary: Option<String>,
    #[serde(default)]
    completed: bool,
    #[serde(default)]
    actions: Vec<AgentAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentAction {
    kind: String,
    url: String,
    role: String,
    reason: Option<String>,
    #[serde(default)]
    form_action: Option<String>,
    #[serde(default)]
    form_method: Option<String>,
    #[serde(default)]
    query_param: Option<String>,
    #[serde(default)]
    wait_for_selector: Option<String>,
}

struct ObservedActionInputs {
    page_urls: std::collections::HashSet<String>,
    link_urls: std::collections::HashSet<String>,
    search_forms_by_page: std::collections::HashMap<String, Vec<PageForm>>,
}

const TASK_CANCELLED_ERROR: &str = "task cancelled by user";
const MODEL_REQUEST_TIMEOUT: Duration = Duration::from_secs(300);

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
    init_database_impl(paths)
}

pub fn load_settings(paths: &AppPaths) -> Result<AiSettings> {
    load_settings_impl(paths)
}

pub fn save_settings(paths: &AppPaths, settings: &AiSettings) -> Result<AiSettings> {
    save_settings_impl(paths, settings)
}

pub fn create_task(paths: &AppPaths, draft: crate::domain::TaskDraft) -> Result<TaskRecord> {
    create_task_impl(paths, draft)
}

pub fn list_tasks(paths: &AppPaths) -> Result<Vec<TaskRecord>> {
    list_tasks_impl(paths)
}

pub fn get_task(paths: &AppPaths, task_id: &str) -> Result<TaskRecord> {
    get_task_impl(paths, task_id)
}

pub fn delete_task(state: &AppState, task_id: &str) -> Result<()> {
    delete_task_impl(state, task_id)
}

fn ensure_not_cancelled(state: &AppState, task_id: &str) -> Result<()> {
    if state.runtime.is_cancel_requested(task_id) {
        bail!(TASK_CANCELLED_ERROR);
    }

    Ok(())
}

pub async fn run_task(
    state: &AppState,
    task_id: &str,
    ai_settings: StartAiSettings,
    api_key: Option<String>,
) -> Result<TaskRecord> {
    task_runner::run_task(state, task_id, ai_settings, api_key).await
}

pub async fn chat_repair_task(
    state: &AppState,
    task_id: &str,
    message: &str,
    ai_settings: StartAiSettings,
    api_key: Option<String>,
) -> Result<crate::domain::AgentChatRepairResult> {
    chat::chat_repair_task(state, task_id, message, ai_settings, api_key).await
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

pub fn export_task_source(
    paths: &AppPaths,
    task_id: &str,
    output_path: Option<&str>,
) -> Result<String> {
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
    updated.updated_at = Utc::now();
    persist_task(paths, &updated)?;
    write_logs(paths, &updated)?;
    Ok(raw)
}

#[cfg(test)]
#[path = "services/tests.rs"]
mod tests;
