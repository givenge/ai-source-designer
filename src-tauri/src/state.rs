use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub tasks_dir: PathBuf,
    pub logs_dir: PathBuf,
    pub settings_path: PathBuf,
    pub fixtures_dir: PathBuf,
    pub worker_script: PathBuf,
}

impl AppPaths {
    pub fn resolve<R: Runtime>(app: &AppHandle<R>) -> Result<Self> {
        let home_dir = app
            .path()
            .home_dir()
            .context("failed to resolve home dir")?;
        let data_dir = home_dir.join(".aisourcedesign");
        let tasks_dir = data_dir.join("tasks");
        let logs_dir = data_dir.join("logs");
        let resource_dir = normalize_runtime_path(
            app.path()
                .resource_dir()
                .context("failed to resolve resource dir")?,
        );
        let source_root =
            normalize_runtime_path(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
        let runtime_root = if cfg!(debug_assertions)
            && source_root.join("playwright/worker/index.mjs").is_file()
        {
            source_root
        } else {
            resource_dir
        };

        let paths = Self {
            db_path: data_dir.join("app.sqlite"),
            settings_path: data_dir.join("settings.json"),
            fixtures_dir: runtime_root.join("fixtures"),
            worker_script: runtime_root
                .join("playwright")
                .join("worker")
                .join("index.mjs"),
            data_dir,
            tasks_dir,
            logs_dir,
        };

        fs::create_dir_all(&paths.data_dir)?;
        fs::create_dir_all(&paths.tasks_dir)?;
        fs::create_dir_all(&paths.logs_dir)?;

        Ok(paths)
    }

    pub fn task_dir(&self, task_id: &str) -> PathBuf {
        self.tasks_dir.join(task_id)
    }

    pub fn ensure_task_layout(&self, task_id: &str) -> Result<()> {
        let root = self.task_dir(task_id);
        for dir in ["screenshots", "dom", "network", "logs", "output"] {
            fs::create_dir_all(root.join(dir))?;
        }
        Ok(())
    }

    pub fn relative_to_task(&self, task_id: &str, path: &Path) -> String {
        path.strip_prefix(self.task_dir(task_id))
            .unwrap_or(path)
            .to_string_lossy()
            .to_string()
    }

    pub fn task_path(&self, task_id: &str, relative: &str) -> PathBuf {
        self.task_dir(task_id).join(relative)
    }
}

fn normalize_runtime_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        if let Some(stripped) = raw.strip_prefix(r"\\?\") {
            return PathBuf::from(stripped);
        }
    }

    path
}

#[derive(Debug, Clone)]
pub struct AppState {
    pub paths: AppPaths,
    pub runtime: Arc<TaskRuntime>,
}

#[derive(Debug, Default)]
pub struct TaskRuntime {
    running: Mutex<HashSet<String>>,
    runtime_repairing: Mutex<bool>,
    cancel_requested: Mutex<HashSet<String>>,
    delete_requested: Mutex<HashSet<String>>,
}

impl TaskRuntime {
    pub fn register(&self, task_id: &str) -> bool {
        let mut running = self.running.lock().expect("task runtime poisoned");
        if running.contains(task_id)
            || *self
                .runtime_repairing
                .lock()
                .expect("task runtime poisoned")
        {
            return false;
        }

        self.cancel_requested
            .lock()
            .expect("task runtime poisoned")
            .remove(task_id);
        self.delete_requested
            .lock()
            .expect("task runtime poisoned")
            .remove(task_id);
        running.insert(task_id.to_string());
        true
    }

    pub fn unregister(&self, task_id: &str) {
        self.running
            .lock()
            .expect("task runtime poisoned")
            .remove(task_id);
        self.cancel_requested
            .lock()
            .expect("task runtime poisoned")
            .remove(task_id);
        self.delete_requested
            .lock()
            .expect("task runtime poisoned")
            .remove(task_id);
    }

    pub fn request_cancel(&self, task_id: &str) {
        self.cancel_requested
            .lock()
            .expect("task runtime poisoned")
            .insert(task_id.to_string());
    }

    pub fn request_delete(&self, task_id: &str) {
        self.delete_requested
            .lock()
            .expect("task runtime poisoned")
            .insert(task_id.to_string());
    }

    pub fn is_cancel_requested(&self, task_id: &str) -> bool {
        self.cancel_requested
            .lock()
            .expect("task runtime poisoned")
            .contains(task_id)
    }

    pub fn is_delete_requested(&self, task_id: &str) -> bool {
        self.delete_requested
            .lock()
            .expect("task runtime poisoned")
            .contains(task_id)
    }

    pub fn is_running(&self, task_id: &str) -> bool {
        self.running
            .lock()
            .expect("task runtime poisoned")
            .contains(task_id)
    }

    pub fn begin_runtime_repair(&self) -> bool {
        let running = self.running.lock().expect("task runtime poisoned");
        let mut repairing = self
            .runtime_repairing
            .lock()
            .expect("task runtime poisoned");
        if !running.is_empty() || *repairing {
            return false;
        }
        *repairing = true;
        true
    }

    pub fn end_runtime_repair(&self) {
        *self
            .runtime_repairing
            .lock()
            .expect("task runtime poisoned") = false;
    }

    pub fn is_runtime_repairing(&self) -> bool {
        *self
            .runtime_repairing
            .lock()
            .expect("task runtime poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_repair_and_task_execution_exclude_each_other() {
        let runtime = TaskRuntime::default();
        assert!(runtime.register("task"));
        assert!(!runtime.begin_runtime_repair());
        runtime.unregister("task");
        assert!(runtime.begin_runtime_repair());
        assert!(!runtime.begin_runtime_repair());
        assert!(!runtime.register("other-task"));
        runtime.end_runtime_repair();
        assert!(runtime.register("other-task"));
    }
}
