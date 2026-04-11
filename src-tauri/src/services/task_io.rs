use super::*;

pub(super) fn read_files(
    paths: &AppPaths,
    task_id: &str,
    entries: &[String],
) -> Result<Vec<ArtifactFile>> {
    entries
        .iter()
        .map(|relative| {
            let path = paths.task_path(task_id, relative);
            let content_type = content_type_for(&path);
            let contents = if content_type.starts_with("image/") && content_type != "image/svg+xml"
            {
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

pub(super) fn content_type_for(path: &PathBuf) -> String {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
    {
        "svg" => "image/svg+xml".into(),
        "png" => "image/png".into(),
        "json" => "application/json".into(),
        "html" => "text/html".into(),
        "log" => "text/plain".into(),
        _ => "text/plain".into(),
    }
}

pub(super) fn persist_and_flush(state: &AppState, task: &mut TaskRecord) -> Result<()> {
    if state.runtime.is_delete_requested(&task.id) {
        return Ok(());
    }
    task.updated_at = Utc::now();
    persist_task(&state.paths, task)?;
    write_logs(&state.paths, task)?;
    Ok(())
}

pub(super) fn finalize_pending_delete(state: &AppState, task_id: &str) -> Result<()> {
    if state.runtime.is_delete_requested(task_id) {
        purge_task_storage(&state.paths, task_id)?;
    }
    Ok(())
}

pub(super) fn write_logs(paths: &AppPaths, task: &TaskRecord) -> Result<()> {
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

pub(super) fn append_log(task: &mut TaskRecord, stage: TaskStatus, message: impl Into<String>) {
    task.logs.push(LogEntry {
        at: Utc::now(),
        stage,
        message: message.into(),
    });
}
