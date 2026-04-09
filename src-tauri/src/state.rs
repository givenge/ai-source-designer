use std::fs;
use std::path::{Path, PathBuf};

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
        let home_dir = app.path().home_dir().context("failed to resolve home dir")?;
        let data_dir = home_dir.join(".aisourcedesign");
        let tasks_dir = data_dir.join("tasks");
        let logs_dir = data_dir.join("logs");
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .canonicalize()
            .context("failed to resolve repository root")?;

        let paths = Self {
            db_path: data_dir.join("app.sqlite"),
            settings_path: data_dir.join("settings.json"),
            fixtures_dir: repo_root.join("fixtures"),
            worker_script: repo_root.join("playwright").join("worker").join("index.mjs"),
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

#[derive(Debug, Clone)]
pub struct AppState {
    pub paths: AppPaths,
}
