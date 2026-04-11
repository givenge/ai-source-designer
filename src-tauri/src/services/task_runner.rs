use super::*;

pub(super) async fn run_task(
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

    let execution: Result<()> = async {
        let existing_sampled = load_sampled_from_task(paths, &task)?;
        let had_progress = existing_sampled
            .as_ref()
            .is_some_and(|sampled| !sampled.pages.is_empty())
            || task.source_candidate.is_some()
            || task.validation_report.is_some();

        ensure_not_cancelled(state, &task.id)?;
        append_log(
            &mut task,
            TaskStatus::Draft,
            if had_progress {
                "继续执行当前任务，保留已有采样、候选与验证进度。"
            } else {
                "开始新一轮执行。"
            },
        );
        persist_and_flush(state, &mut task)?;

        let need_sampling = existing_sampled
            .as_ref()
            .map(|sampled| sampled.pages.is_empty() || !agent_has_minimum_coverage(&sampled.pages))
            .unwrap_or(true);

        let sampled = if need_sampling {
            ensure_not_cancelled(state, &task.id)?;
            let sampling_message = match task.task_mode {
                TaskMode::Agent if existing_sampled.is_some() => {
                    "继续 agent 探索，补齐缺失页面与证据。"
                }
                TaskMode::Agent => "开始 agent 探索，由模型驱动 Playwright 多轮取证。",
                TaskMode::Classic if existing_sampled.is_some() => "继续采样页面与网络证据。",
                TaskMode::Classic => "开始采样页面与网络证据。",
            };
            append_log(&mut task, TaskStatus::Sampling, sampling_message);
            task.status = TaskStatus::Sampling;
            persist_and_flush(state, &mut task)?;

            let sampled = match task.task_mode {
                TaskMode::Agent => {
                    run_agent_sampling(
                        state,
                        &task,
                        &ai_settings,
                        api_key.clone(),
                        existing_sampled.clone(),
                    )
                    .await?
                }
                TaskMode::Classic => existing_sampled.clone().unwrap_or(worker::sample_site(
                    state,
                    &task.id,
                    &task.target_url,
                    task.test_keyword.as_deref(),
                )?),
            };
            ensure_not_cancelled(state, &task.id)?;
            task.artifact_manifest = Some(sampled.manifest.clone());
            append_log(
                &mut task,
                TaskStatus::Sampling,
                format!("采样完成，共记录 {} 个页面样本。", sampled.pages.len()),
            );
            persist_and_flush(state, &mut task)?;
            sampled
        } else {
            let sampled = existing_sampled.unwrap_or_default();
            task.artifact_manifest = Some(sampled.manifest.clone());
            append_log(
                &mut task,
                TaskStatus::Sampling,
                format!(
                    "复用已有采样结果，共记录 {} 个页面样本。",
                    sampled.pages.len()
                ),
            );
            persist_and_flush(state, &mut task)?;
            sampled
        };

        ensure_not_cancelled(state, &task.id)?;
        let mut candidate = if let Some(existing) = task.source_candidate.clone() {
            append_log(
                &mut task,
                TaskStatus::Synthesizing,
                "复用已有规则候选，继续后续验证与修复。",
            );
            let mut existing = existing;
            let _ = sanitize_candidate(&mut existing);
            task.source_candidate = Some(existing.clone());
            persist_and_flush(state, &mut task)?;
            existing
        } else {
            append_log(
                &mut task,
                TaskStatus::Synthesizing,
                "开始合成书源规则候选。",
            );
            task.status = TaskStatus::Synthesizing;
            persist_and_flush(state, &mut task)?;

            let candidate =
                synthesize_candidate(paths, &task.id, &sampled, &ai_settings, api_key.clone())
                    .await?;
            let mut candidate = candidate;
            let _ = sanitize_candidate(&mut candidate);
            ensure_not_cancelled(state, &task.id)?;
            task.source_candidate = Some(candidate.clone());
            append_log(
                &mut task,
                TaskStatus::Synthesizing,
                format!("规则生成完成，当前置信度 {:.2}。", candidate.confidence),
            );
            persist_and_flush(state, &mut task)?;
            candidate
        };

        ensure_not_cancelled(state, &task.id)?;
        append_log(
            &mut task,
            TaskStatus::Validating,
            "开始本地兼容与执行验证。",
        );
        task.status = TaskStatus::Validating;
        persist_and_flush(state, &mut task)?;

        let mut validation_error: Option<String> = None;
        let mut report = match validate_candidate(
            paths,
            &task.id,
            &sampled,
            &candidate,
            task.test_keyword.as_deref(),
        ) {
            Ok(report) => report,
            Err(error) => {
                validation_error = Some(error.to_string());
                ValidationReport::default()
            }
        };
        ensure_not_cancelled(state, &task.id)?;
        task.validation_report = Some(report.clone());
        persist_and_flush(state, &mut task)?;

        if validation_error.is_some() || report_requires_repair(&report) {
            task.status = TaskStatus::Repairing;
            append_log(
                &mut task,
                TaskStatus::Repairing,
                "验证未完全通过，尝试执行首轮修复。",
            );
            persist_and_flush(state, &mut task)?;

            let failure_summary = validation_error
                .clone()
                .unwrap_or_else(|| first_failure_summary(&report));
            let mut repaired = try_repair_candidate_with_model(
                paths,
                &task.id,
                &sampled,
                &ai_settings,
                api_key.clone(),
                &mut candidate,
                &failure_summary,
            )
            .await?;

            if !repaired {
                repaired = try_repair_candidate_with_sampled(
                    paths,
                    &task.id,
                    &sampled,
                    &mut candidate,
                    &report,
                )?;
            }

            if repaired {
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
                append_log(
                    &mut task,
                    TaskStatus::Repairing,
                    "已应用修复策略并重新验证。",
                );
                persist_and_flush(state, &mut task)?;
            } else {
                append_log(&mut task, TaskStatus::Repairing, "没有可用的自动修复策略。");
                persist_and_flush(state, &mut task)?;
            }
        }

        let success = report_is_usable(&report);

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

        persist_and_flush(state, &mut task)?;
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
            append_log(
                &mut task,
                TaskStatus::Failed,
                format!("任务失败：{summary}"),
            );
        }
        persist_and_flush(state, &mut task)?;
    }

    finalize_pending_delete(state, &task.id)?;

    Ok(task)
}
