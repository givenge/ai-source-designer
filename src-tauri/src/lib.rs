mod domain;
mod services;
mod state;
mod worker;

use anyhow::Result;
use domain::{AgentChatRepairResult, AiSettings, ArtifactBundle, TaskDraft, TaskRecord};
use services::{
    chat_repair_task as chat_repair_task_impl, create_task as create_task_impl,
    delete_task as delete_task_impl, export_task_source as export_task_source_impl,
    get_task as get_task_impl, get_task_artifacts as get_task_artifacts_impl, init_database,
    list_tasks as list_tasks_impl, load_settings as load_settings_impl, run_task as run_task_impl,
    save_settings as save_settings_impl,
};
use state::{AppPaths, AppState, TaskRuntime};
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_log::{Target, TargetKind};

fn initialize<R: Runtime>(app: &AppHandle<R>) -> Result<AppState> {
    let paths = AppPaths::resolve(app)?;
    init_database(&paths)?;
    Ok(AppState {
        paths,
        runtime: Arc::new(TaskRuntime::default()),
    })
}

#[tauri::command]
fn load_settings(state: State<'_, AppState>) -> Result<AiSettings, String> {
    load_settings_impl(&state.paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, settings: AiSettings) -> Result<AiSettings, String> {
    save_settings_impl(&state.paths, &settings).map_err(|error| error.to_string())
}

#[tauri::command]
fn list_tasks(state: State<'_, AppState>) -> Result<Vec<TaskRecord>, String> {
    list_tasks_impl(&state.paths).map_err(|error| error.to_string())
}

#[tauri::command]
fn create_task(state: State<'_, AppState>, draft: TaskDraft) -> Result<TaskRecord, String> {
    create_task_impl(&state.paths, draft).map_err(|error| error.to_string())
}

#[tauri::command]
fn delete_task(state: State<'_, AppState>, task_id: String) -> Result<(), String> {
    delete_task_impl(state.inner(), &task_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn get_task(state: State<'_, AppState>, task_id: String) -> Result<TaskRecord, String> {
    get_task_impl(&state.paths, &task_id).map_err(|error| error.to_string())
}

#[tauri::command]
async fn start_task(
    state: State<'_, AppState>,
    task_id: String,
    ai_settings: domain::StartAiSettings,
    api_key: Option<String>,
) -> Result<TaskRecord, String> {
    let app_state = state.inner().clone();
    run_task_impl(&app_state, &task_id, ai_settings, api_key)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
async fn chat_repair_task(
    state: State<'_, AppState>,
    task_id: String,
    message: String,
    ai_settings: domain::StartAiSettings,
    api_key: Option<String>,
) -> Result<AgentChatRepairResult, String> {
    let app_state = state.inner().clone();
    chat_repair_task_impl(&app_state, &task_id, &message, ai_settings, api_key)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn get_task_artifacts(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<ArtifactBundle, String> {
    get_task_artifacts_impl(&state.paths, &task_id).map_err(|error| error.to_string())
}

#[tauri::command]
fn export_task_source(
    state: State<'_, AppState>,
    task_id: String,
    output_path: Option<String>,
) -> Result<String, String> {
    export_task_source_impl(&state.paths, &task_id, output_path.as_deref())
        .map_err(|error| error.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            load_settings,
            save_settings,
            list_tasks,
            create_task,
            delete_task,
            get_task,
            start_task,
            chat_repair_task,
            get_task_artifacts,
            export_task_source,
        ])
        .setup(|app| {
            let state = initialize(&app.handle()).map_err(|error| error.to_string())?;
            app.handle()
                .plugin(tauri_plugin_dialog::init())
                .map_err(|error| error.to_string())?;
            app.handle()
                .plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .clear_targets()
                        .target(Target::new(TargetKind::Folder {
                            path: state.paths.logs_dir.clone(),
                            file_name: Some("app".into()),
                        }))
                        .target(Target::new(TargetKind::Stdout))
                        .build(),
                )
                .map_err(|error| error.to_string())?;

            app.manage(state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
