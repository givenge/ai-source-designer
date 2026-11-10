use super::*;
use crate::domain::{ConnectionTestResult, RuntimeDiagnostics};
use crate::worker_runtime::{self, RuntimeCheck};
use std::process::Command;

pub async fn test_ai_connection(settings: &AiSettings) -> Result<ConnectionTestResult> {
    if settings.model.trim().is_empty() {
        bail!("请填写模型名称");
    }
    if settings.base_url.starts_with("mock://") {
        return Ok(ConnectionTestResult {
            model: settings.model.clone(),
            latency_ms: 0,
            mock: true,
        });
    }
    let base = normalize_openai_endpoint(&settings.base_url);
    let endpoint = format!("{}/models", base.trim_end_matches("/chat/completions"));
    let url = Url::parse(&endpoint)?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Base URL 必须使用 http 或 https");
    }
    let secret = settings.api_key.trim();
    let redact = |value: &str| {
        if secret.is_empty() {
            value.to_string()
        } else {
            value.replace(secret, "[REDACTED]")
        }
    };
    let started = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()?;
    let mut request = client.get(url);
    if !settings.api_key.trim().is_empty() {
        request = request.bearer_auth(settings.api_key.trim());
    }
    let response = request
        .send()
        .await
        .map_err(|error| anyhow!("{}", redact(&error.to_string())))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| anyhow!("{}", redact(&error.to_string())))?;
    if !status.is_success() {
        bail!(
            "HTTP {}：{}",
            status.as_u16(),
            redact(&body).chars().take(240).collect::<String>()
        );
    }
    let data: Value =
        serde_json::from_str(&body).map_err(|_| anyhow!("服务未返回兼容的模型列表"))?;
    let models = data
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("服务未返回兼容的模型列表"))?;
    if !models
        .iter()
        .any(|model| model.get("id").and_then(Value::as_str) == Some(settings.model.trim()))
    {
        bail!("模型列表中未找到 {}", settings.model.trim());
    }
    Ok(ConnectionTestResult {
        model: settings.model.trim().into(),
        latency_ms: started.elapsed().as_millis(),
        mock: false,
    })
}

pub fn runtime_diagnostics(paths: &AppPaths) -> RuntimeDiagnostics {
    build_runtime_diagnostics(paths, worker_runtime::check_runtime(paths))
}

fn build_runtime_diagnostics(paths: &AppPaths, result: Result<RuntimeCheck>) -> RuntimeDiagnostics {
    let database_ready =
        Connection::open_with_flags(&paths.db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .and_then(|connection| connection.query_row("SELECT 1", [], |row| row.get::<_, i32>(0)))
            .is_ok();
    let runtime = result.unwrap_or_else(|error| RuntimeCheck {
        error: Some(error.to_string()),
        ..Default::default()
    });
    RuntimeDiagnostics {
        version: env!("CARGO_PKG_VERSION").into(),
        data_dir: paths.data_dir.to_string_lossy().into(),
        db_path: paths.db_path.to_string_lossy().into(),
        settings_path: paths.settings_path.to_string_lossy().into(),
        worker_script: paths.worker_script.to_string_lossy().into(),
        worker_ready: runtime.browser_version.is_some()
            && runtime.error.is_none()
            && paths.worker_script.is_file(),
        database_ready,
        node_version: runtime.node_version,
        node_path: runtime.node_path,
        playwright_path: runtime.playwright_path,
        playwright_version: runtime.playwright_version,
        browser_version: runtime.browser_version,
        error: runtime.error,
    }
}

struct RuntimeRepairGuard<'a>(&'a TaskRuntime);
impl Drop for RuntimeRepairGuard<'_> {
    fn drop(&mut self) {
        self.0.end_runtime_repair();
    }
}

pub fn repair_playwright(state: &AppState) -> Result<RuntimeDiagnostics> {
    if !state.runtime.begin_runtime_repair() {
        bail!("任务执行中或运行环境正在修复，请稍后重试。");
    }
    let _guard = RuntimeRepairGuard(&state.runtime);
    let result = worker_runtime::repair_runtime(&state.paths)?;
    Ok(build_runtime_diagnostics(&state.paths, Ok(result)))
}

pub fn open_data_directory(paths: &AppPaths) -> Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let result = Command::new(program).arg(&paths.data_dir).status()?;
    if !result.success() {
        bail!("打开数据目录失败");
    }
    Ok(())
}

pub fn export_task_log(
    paths: &AppPaths,
    task_id: &str,
    kind: &str,
    output_path: &str,
) -> Result<()> {
    get_task(paths, task_id)?;
    let filename = match kind {
        "task" => "task.log",
        "worker" => "worker.log",
        _ => bail!("未知日志类型"),
    };
    if output_path.trim().is_empty() {
        bail!("请选择导出位置");
    }
    let content = fs::read_to_string(paths.task_path(task_id, &format!("logs/{filename}")))?;
    write_export_file(Path::new(output_path), &content)
}

pub fn revalidate_task(state: &AppState, task_id: &str) -> Result<TaskRecord> {
    let _guard = RunningTaskGuard::register(state, task_id)?;
    let result = (|| {
        let mut task = get_task(&state.paths, task_id)?;
        let mut candidate = task
            .source_candidate
            .clone()
            .ok_or_else(|| anyhow!("尚未生成书源规则"))?;
        let sampled = load_sampled_from_task(&state.paths, &task)?
            .ok_or_else(|| anyhow!("没有可验证的采样证据"))?;
        task.status = TaskStatus::Validating;
        task.error_summary = None;
        append_log(
            &mut task,
            TaskStatus::Validating,
            "重新验证已有规则和采样证据。",
        );
        persist_and_flush(state, &mut task)?;
        let operation: Result<()> = (|| {
            ensure_not_cancelled(state, task_id)?;
            let report = validate_candidate(
                &state.paths,
                task_id,
                &sampled,
                &candidate,
                task.test_keyword.as_deref(),
            )?;
            if report_is_usable(&report) {
                if !report.explore.passed && candidate.book_source.explore_url.is_some() {
                    candidate.book_source.enabled_explore = Some(false);
                    append_log(
                        &mut task,
                        TaskStatus::Validating,
                        format!(
                            "书海未通过，保留已验证的搜索能力：{}",
                            report.explore.summary
                        ),
                    );
                }
                let (relative, _) = export_candidate(&state.paths, task_id, &candidate)?;
                append_output(&state.paths, task_id, &mut task, relative);
                task.status = TaskStatus::Completed;
            } else {
                task.status = TaskStatus::Failed;
                task.error_summary = Some(first_failure_summary(&report));
            }
            task.validation_report = Some(report);
            task.source_candidate = Some(candidate);
            Ok(())
        })();
        if let Err(error) = operation {
            task.status = TaskStatus::Failed;
            task.error_summary = Some(error.to_string());
        }
        let stage = task.status.clone();
        let message = task
            .error_summary
            .clone()
            .unwrap_or_else(|| "重新验证通过。".into());
        append_log(&mut task, stage, message);
        persist_and_flush(state, &mut task)?;
        Ok(task)
    })();
    finalize_pending_delete(state, task_id)?;
    result
}
