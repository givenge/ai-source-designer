use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Draft,
    Sampling,
    Synthesizing,
    Validating,
    Repairing,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum TaskMode {
    #[default]
    Classic,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub base_url: String,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            base_url: "mock://fixture".into(),
            model: "mock-fixture".into(),
            api_key: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub model: String,
    pub latency_ms: u128,
    pub mock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeDiagnostics {
    pub version: String,
    pub data_dir: String,
    pub db_path: String,
    pub settings_path: String,
    pub worker_script: String,
    pub worker_ready: bool,
    pub database_ready: bool,
    pub node_version: Option<String>,
    pub node_path: Option<String>,
    pub playwright_path: Option<String>,
    pub playwright_version: Option<String>,
    pub browser_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDraft {
    pub target_url: String,
    pub test_keyword: Option<String>,
    #[serde(default)]
    pub task_mode: TaskMode,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactManifest {
    pub screenshots: Vec<String>,
    pub dom: Vec<String>,
    pub network: Vec<String>,
    pub logs: Vec<String>,
    pub output: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactFile {
    pub path: String,
    pub label: String,
    pub content_type: String,
    pub contents: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactBundle {
    pub manifest: ArtifactManifest,
    pub screenshots: Vec<ArtifactFile>,
    pub dom: Vec<ArtifactFile>,
    pub network: Vec<ArtifactFile>,
    pub logs: Vec<ArtifactFile>,
    pub output: Vec<ArtifactFile>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceRef {
    pub label: String,
    pub artifact_path: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepairStep {
    pub label: String,
    pub reason: String,
    pub applied_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookListRule {
    #[serde(default, alias = "check_key_word")]
    pub check_key_word: Option<String>,
    #[serde(default, alias = "book_list")]
    pub book_list: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub intro: Option<String>,
    pub kind: Option<String>,
    #[serde(default, alias = "last_chapter")]
    pub last_chapter: Option<String>,
    #[serde(
        default,
        alias = "update_time",
        skip_serializing_if = "Option::is_none"
    )]
    pub update_time: Option<String>,
    #[serde(default, alias = "book_url")]
    pub book_url: Option<String>,
    #[serde(default, alias = "cover_url")]
    pub cover_url: Option<String>,
    #[serde(default, alias = "word_count")]
    pub word_count: Option<String>,
}

pub type SearchRule = BookListRule;
pub type ExploreRule = BookListRule;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookInfoRule {
    pub init: Option<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub intro: Option<String>,
    pub kind: Option<String>,
    #[serde(default, alias = "last_chapter")]
    pub last_chapter: Option<String>,
    #[serde(
        default,
        alias = "update_time",
        skip_serializing_if = "Option::is_none"
    )]
    pub update_time: Option<String>,
    #[serde(default, alias = "cover_url")]
    pub cover_url: Option<String>,
    #[serde(default, alias = "word_count")]
    pub word_count: Option<String>,
    #[serde(default, alias = "toc_url")]
    pub toc_url: Option<String>,
    #[serde(default, alias = "can_re_name")]
    pub can_re_name: Option<String>,
    #[serde(default, alias = "download_urls")]
    pub download_urls: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TocRule {
    #[serde(default, alias = "pre_update_js")]
    pub pre_update_js: Option<String>,
    pub init: Option<String>,
    #[serde(default, alias = "chapter_list")]
    pub chapter_list: Option<String>,
    #[serde(default, alias = "chapter_name")]
    pub chapter_name: Option<String>,
    #[serde(default, alias = "chapter_url")]
    pub chapter_url: Option<String>,
    #[serde(default, alias = "format_js")]
    pub format_js: Option<String>,
    #[serde(default, alias = "is_volume")]
    pub is_volume: Option<String>,
    #[serde(default, alias = "is_vip")]
    pub is_vip: Option<String>,
    #[serde(default, alias = "is_pay")]
    pub is_pay: Option<String>,
    #[serde(
        default,
        alias = "update_time",
        skip_serializing_if = "Option::is_none"
    )]
    pub update_time: Option<String>,
    #[serde(default, alias = "next_toc_url")]
    pub next_toc_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ContentRule {
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, alias = "next_content_url")]
    pub next_content_url: Option<String>,
    #[serde(default, alias = "web_js")]
    pub web_js: Option<String>,
    #[serde(default, alias = "source_regex")]
    pub source_regex: Option<String>,
    #[serde(default, alias = "replace_regex")]
    pub replace_regex: Option<String>,
    #[serde(default, alias = "image_style")]
    pub image_style: Option<String>,
    #[serde(default, alias = "image_decode")]
    pub image_decode: Option<String>,
    #[serde(default, alias = "pay_action")]
    pub pay_action: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BookSource {
    #[serde(
        default,
        alias = "book_source_name",
        alias = "sourceName",
        alias = "source_name",
        alias = "name",
        alias = "title"
    )]
    pub book_source_name: String,
    #[serde(
        default,
        alias = "book_source_url",
        alias = "sourceUrl",
        alias = "source_url",
        alias = "baseUrl",
        alias = "base_url",
        alias = "url"
    )]
    pub book_source_url: String,
    #[serde(default, alias = "book_source_group")]
    pub book_source_group: Option<String>,
    #[serde(default, alias = "book_source_type")]
    pub book_source_type: Option<i64>,
    #[serde(default, alias = "book_url_pattern")]
    pub book_url_pattern: Option<String>,
    #[serde(default, alias = "custom_order")]
    pub custom_order: Option<i64>,
    pub enabled: Option<bool>,
    #[serde(default, alias = "enabled_explore")]
    pub enabled_explore: Option<bool>,
    #[serde(default, alias = "enabled_cookie_jar")]
    pub enabled_cookie_jar: Option<bool>,
    #[serde(default, alias = "js_lib")]
    pub js_lib: Option<String>,
    #[serde(default, alias = "concurrent_rate")]
    pub concurrent_rate: Option<String>,
    pub header: Option<Value>,
    #[serde(default, alias = "login_url")]
    pub login_url: Option<String>,
    #[serde(default, alias = "login_ui")]
    pub login_ui: Option<Value>,
    #[serde(default, alias = "login_check_js")]
    pub login_check_js: Option<String>,
    #[serde(default, alias = "cover_decode_js")]
    pub cover_decode_js: Option<String>,
    #[serde(default, alias = "last_update_time")]
    pub last_update_time: Option<i64>,
    pub weight: Option<i64>,
    #[serde(default, alias = "explore_url")]
    pub explore_url: Option<String>,
    #[serde(default, alias = "explore_screen")]
    pub explore_screen: Option<Value>,
    #[serde(default, alias = "rule_explore")]
    pub rule_explore: Option<ExploreRule>,
    #[serde(default, alias = "search_url")]
    pub search_url: Option<String>,
    #[serde(default, alias = "rule_search")]
    pub rule_search: Option<SearchRule>,
    #[serde(default, alias = "rule_book_info")]
    pub rule_book_info: Option<BookInfoRule>,
    #[serde(default, alias = "rule_toc")]
    pub rule_toc: Option<TocRule>,
    #[serde(default, alias = "rule_content")]
    pub rule_content: Option<ContentRule>,
    #[serde(default, alias = "rule_review")]
    pub rule_review: Option<Value>,
    #[serde(default, alias = "book_source_comment")]
    pub book_source_comment: Option<String>,
    #[serde(default, alias = "variable_comment")]
    pub variable_comment: Option<String>,
    #[serde(default, alias = "respond_time")]
    pub respond_time: Option<i64>,
    #[serde(default, alias = "load_with_base_url")]
    pub load_with_base_url: Option<bool>,
    #[serde(default, alias = "single_url")]
    pub single_url: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceCandidate {
    pub strategy: String,
    pub confidence: f64,
    pub evidence_refs: Vec<EvidenceRef>,
    pub repair_history: Vec<RepairStep>,
    pub book_source: BookSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValidationPhase {
    pub passed: bool,
    pub summary: String,
    pub details: Vec<String>,
    pub evidence_refs: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub explore: ValidationPhase,
    pub search: ValidationPhase,
    pub book_info: ValidationPhase,
    pub toc: ValidationPhase,
    pub content: ValidationPhase,
    pub cleanup: ValidationPhase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub at: DateTime<Utc>,
    pub stage: TaskStatus,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskRecord {
    pub id: String,
    pub status: TaskStatus,
    #[serde(default)]
    pub task_mode: TaskMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub target_url: String,
    #[serde(default)]
    pub test_keyword: Option<String>,
    pub error_summary: Option<String>,
    pub artifact_manifest: Option<ArtifactManifest>,
    pub source_candidate: Option<SourceCandidate>,
    pub validation_report: Option<ValidationReport>,
    pub logs: Vec<LogEntry>,
}

impl TaskRecord {
    pub fn new(draft: TaskDraft) -> Self {
        let TaskDraft {
            target_url,
            test_keyword,
            task_mode,
        } = draft;
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            status: TaskStatus::Draft,
            task_mode,
            created_at: now,
            updated_at: now,
            target_url,
            test_keyword,
            error_summary: None,
            artifact_manifest: None,
            source_candidate: None,
            validation_report: None,
            logs: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartAiSettings {
    pub base_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentChatRepairResult {
    pub reply: String,
    pub applied: bool,
    #[serde(default)]
    pub model_raw_output: Option<String>,
    pub task: TaskRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageSample {
    pub role: String,
    pub url: String,
    pub html_path: String,
    pub screenshot_path: Option<String>,
    pub snippet_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NetworkSample {
    pub label: String,
    pub request_url: String,
    pub method: String,
    pub response_path: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SamplingResult {
    pub fixture_name: Option<String>,
    pub site_label: String,
    pub base_url: String,
    pub pages: Vec<PageSample>,
    pub network_samples: Vec<NetworkSample>,
    pub manifest: ArtifactManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageLink {
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageForm {
    pub action: String,
    pub method: String,
    pub input_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageInspection {
    pub page: PageSample,
    pub title: String,
    pub excerpt: String,
    pub links: Vec<PageLink>,
    pub forms: Vec<PageForm>,
    #[serde(default)]
    pub hints: PageHints,
    pub network_samples: Vec<NetworkSample>,
    pub manifest: ArtifactManifest,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PageHints {
    pub book_list_candidates: Vec<String>,
    pub title_link_candidates: Vec<String>,
    pub toc_link_candidates: Vec<String>,
    pub content_candidates: Vec<String>,
    pub next_page_candidates: Vec<String>,
}
