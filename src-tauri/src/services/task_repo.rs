use super::*;

pub(super) fn init_database(paths: &AppPaths) -> Result<()> {
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

pub(super) fn load_settings(paths: &AppPaths) -> Result<AiSettings> {
    if !paths.settings_path.exists() {
        return Ok(AiSettings::default());
    }

    let raw = fs::read_to_string(&paths.settings_path)?;
    Ok(serde_json::from_str(&raw)?)
}

pub(super) fn save_settings(paths: &AppPaths, settings: &AiSettings) -> Result<AiSettings> {
    let raw = serde_json::to_string_pretty(settings)?;
    fs::write(&paths.settings_path, raw)?;
    Ok(settings.clone())
}

pub(super) fn create_task(paths: &AppPaths, draft: crate::domain::TaskDraft) -> Result<TaskRecord> {
    let task = TaskRecord::new(draft);
    paths.ensure_task_layout(&task.id)?;
    persist_task(paths, &task)?;
    write_logs(paths, &task)?;
    Ok(task)
}

pub(super) fn list_tasks(paths: &AppPaths) -> Result<Vec<TaskRecord>> {
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

pub(super) fn get_task(paths: &AppPaths, task_id: &str) -> Result<TaskRecord> {
    let conn = Connection::open(&paths.db_path)?;
    let payload: String = conn.query_row(
        "SELECT payload FROM tasks WHERE id = ?1",
        [task_id],
        |row| row.get(0),
    )?;

    Ok(serde_json::from_str(&payload)?)
}

pub(super) fn delete_task(state: &AppState, task_id: &str) -> Result<()> {
    if state.runtime.is_running(task_id) {
        state.runtime.request_delete(task_id);
        state.runtime.request_cancel(task_id);
        delete_task_row(&state.paths, task_id)?;
        return Ok(());
    }

    purge_task_storage(&state.paths, task_id)
}

fn delete_task_row(paths: &AppPaths, task_id: &str) -> Result<()> {
    let conn = Connection::open(&paths.db_path)?;
    let removed = conn.execute("DELETE FROM tasks WHERE id = ?1", [task_id])?;
    if removed == 0 && !paths.task_dir(task_id).exists() {
        bail!("task not found");
    }

    Ok(())
}

pub(super) fn purge_task_storage(paths: &AppPaths, task_id: &str) -> Result<()> {
    delete_task_row(paths, task_id)?;

    let task_dir = paths.task_dir(task_id);
    if task_dir.exists() {
        if let Err(error) = fs::remove_dir_all(&task_dir) {
            relocate_task_dir_to_trash(paths, task_id, &task_dir)
                .map_err(|trash_error| anyhow!("failed to delete task dir: {error}; failed to relocate task dir: {trash_error}"))?;
        }
    }

    Ok(())
}

fn relocate_task_dir_to_trash(paths: &AppPaths, task_id: &str, task_dir: &Path) -> Result<()> {
    let trash_dir = paths.data_dir.join(".trash");
    fs::create_dir_all(&trash_dir)?;
    let relocated = trash_dir.join(format!("{task_id}-{}", Utc::now().timestamp_millis()));
    fs::rename(task_dir, relocated)?;
    Ok(())
}

pub(super) fn persist_task(paths: &AppPaths, task: &TaskRecord) -> Result<()> {
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

pub(super) fn is_task_cancelled(error: &anyhow::Error) -> bool {
    error.to_string().contains(TASK_CANCELLED_ERROR)
}
