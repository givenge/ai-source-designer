use super::*;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgentChatPayload {
    reply: String,
    #[serde(default)]
    book_source: Option<Value>,
}

pub(super) async fn chat_repair_task(
    state: &AppState,
    task_id: &str,
    message: &str,
    ai_settings: StartAiSettings,
    api_key: Option<String>,
) -> Result<crate::domain::AgentChatRepairResult> {
    let trimmed_message = message.trim();
    if trimmed_message.is_empty() {
        bail!("message is required");
    }

    let _guard = RunningTaskGuard::register(state, task_id)?;
    let paths = &state.paths;
    let mut task = get_task(paths, task_id)?;
    let sampled = load_sampled_from_task(paths, &task)?.unwrap_or_default();
    let Some(existing_candidate) = task.source_candidate.clone() else {
        bail!("task has no source candidate");
    };

    let key = api_key
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            load_settings(paths)
                .ok()
                .map(|settings| settings.api_key.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| anyhow!("chat repair requires api key"))?;

    if ai_settings.base_url.trim().is_empty() || ai_settings.base_url.starts_with("mock://") {
        bail!("chat repair requires a real model endpoint");
    }

    append_log(
        &mut task,
        TaskStatus::Repairing,
        format!("对话修复请求：{trimmed_message}"),
    );
    task.status = TaskStatus::Repairing;
    task.error_summary = None;
    persist_and_flush(state, &mut task)?;

    let operation = async {
        ensure_not_cancelled(state, &task.id)?;
        append_log(&mut task, TaskStatus::Repairing, "对话修复：准备上下文与提示词。");
        persist_and_flush(state, &mut task)?;
        let prompt = build_chat_repair_prompt(paths, &task, &sampled, trimmed_message)?;
        append_log(&mut task, TaskStatus::Repairing, "对话修复：请求模型生成修复建议。");
        persist_and_flush(state, &mut task)?;
        let content = request_model_message(
            paths,
            &task.id,
            "chat-repair",
            &ai_settings,
            &key,
            "You are a Reader-Rust book source repair assistant. Return exactly one JSON object with keys reply and optional bookSource. Do not output markdown fences, explanations, or trailing characters. If you propose no rule update, omit bookSource.",
            &prompt,
        )
        .await?;
        ensure_not_cancelled(state, &task.id)?;

        append_log(&mut task, TaskStatus::Repairing, "对话修复：解析模型响应。");
        persist_and_flush(state, &mut task)?;
        let model_raw_output = Some(content.clone());
        let payload = parse_agent_chat_payload(&content)?;
        let mut applied = false;
        let mut task_reply = payload.reply.trim().to_string();

        if let Some(book_source) = payload.book_source {
            append_log(&mut task, TaskStatus::Repairing, "对话修复：应用规则补丁并执行验证。");
            persist_and_flush(state, &mut task)?;
            let candidate_json = json!({
                "strategy": "chat-repair",
                "confidence": existing_candidate.confidence,
                "evidenceRefs": existing_candidate.evidence_refs.clone(),
                "repairHistory": existing_candidate.repair_history.clone(),
                "bookSource": book_source,
            });
            let body: OpenAiResponse = serde_json::from_value(json!({
                "choices": [
                    {
                        "message": {
                            "content": serde_json::to_string(&candidate_json)?
                        }
                    }
                ]
            }))?;

            let repaired_candidate = parse_candidate_from_openai(paths, &sampled, &body)?;
            let merged_candidate = merge_candidates(existing_candidate, repaired_candidate);
            let mut merged_candidate = merged_candidate;
            let _ = sanitize_candidate(&mut merged_candidate);
            task.source_candidate = Some(merged_candidate.clone());
            task.validation_report = Some(validate_candidate(
                paths,
                &task.id,
                &sampled,
                &merged_candidate,
                task.test_keyword.as_deref(),
            )?);

            if report_is_usable(task.validation_report.as_ref().expect("validated")) {
                task.status = TaskStatus::Completed;
                task.error_summary = None;
                let (relative_path, _) = export_candidate(paths, &task.id, &merged_candidate)?;
                let current_task_id = task.id.clone();
                append_output(paths, &current_task_id, &mut task, relative_path);
                append_log(
                    &mut task,
                    TaskStatus::Completed,
                    "对话修复后验证通过，已导出产物。",
                );
                if task_reply.is_empty() {
                    task_reply = "已应用修复并通过验证。".into();
                }
            } else {
                task.status = TaskStatus::Failed;
                task.error_summary = Some(first_failure_summary(
                    task.validation_report.as_ref().expect("validated"),
                ));
                let failure_summary = task
                    .error_summary
                    .clone()
                    .unwrap_or_else(|| "对话修复后仍未通过验证。".into());
                append_log(&mut task, TaskStatus::Failed, failure_summary);
                if task_reply.is_empty() {
                    task_reply = "已应用修复，但仍未通过验证。".into();
                }
            }

            applied = true;
        } else {
            append_log(
                &mut task,
                TaskStatus::Repairing,
                "对话回复未包含可应用的 bookSource patch。",
            );
            persist_and_flush(state, &mut task)?;
        }

        persist_and_flush(state, &mut task)?;

        Ok(crate::domain::AgentChatRepairResult {
            reply: if task_reply.is_empty() {
                "已收到。".into()
            } else {
                task_reply
            },
            applied,
            model_raw_output,
            task: task.clone(),
        })
    };

    match operation.await {
        Ok(result) => Ok(result),
        Err(error) => {
            let summary = format!("对话修复失败：{error}");
            task.status = TaskStatus::Failed;
            task.error_summary = Some(summary.clone());
            append_log(&mut task, TaskStatus::Failed, summary);
            let _ = persist_and_flush(state, &mut task);
            Err(error)
        }
    }
}

fn build_chat_repair_prompt(
    paths: &AppPaths,
    task: &TaskRecord,
    sampled: &SamplingResult,
    message: &str,
) -> Result<String> {
    let current_source = serde_json::to_string_pretty(
        &task
            .source_candidate
            .as_ref()
            .map(|candidate| candidate.book_source.clone())
            .unwrap_or_default(),
    )?;
    let validation = serde_json::to_string_pretty(&task.validation_report)?;
    let task_log = tail_file(&paths.task_path(&task.id, "logs/task.log"), 120);
    let worker_log = tail_file(&paths.task_path(&task.id, "logs/worker.log"), 160);
    let pages = sampled
        .pages
        .iter()
        .map(|page| format!("- {} {}", page.role, page.url))
        .collect::<Vec<_>>()
        .join("\n");

    Ok(format!(
        "User request:\n{message}\n\nTask id: {task_id}\nTarget: {target}\nLast error: {error}\n\nSampled pages:\n{pages}\n\nCurrent bookSource:\n{current_source}\n\nValidation report:\n{validation}\n\nTask log tail:\n{task_log}\n\nWorker log tail:\n{worker_log}\n\nReturn one JSON object:\n{{\"reply\":\"...\",\"bookSource\":{{...optional updated book source...}}}}\nOnly include bookSource when you want to apply a repair.",
        task_id = task.id,
        target = task.target_url,
        error = task.error_summary.clone().unwrap_or_default(),
    ))
}

fn tail_file(path: &Path, limit: usize) -> String {
    let Ok(content) = fs::read_to_string(path) else {
        return String::new();
    };
    let lines = content.lines().collect::<Vec<_>>();
    let start = lines.len().saturating_sub(limit);
    lines[start..].join("\n")
}

fn parse_agent_chat_payload(content: &str) -> Result<AgentChatPayload> {
    let trimmed = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim())
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();

    if let Ok(payload) = serde_json::from_str::<AgentChatPayload>(trimmed) {
        return Ok(payload);
    }

    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
    if let Some(value) = stream.next() {
        if let Ok(payload) = serde_json::from_value::<AgentChatPayload>(value?) {
            return Ok(payload);
        }
    }

    if let Some(object_json) = extract_first_json_object(trimmed) {
        if let Ok(payload) = serde_json::from_str::<AgentChatPayload>(&object_json) {
            return Ok(payload);
        }
    }

    Ok(serde_json::from_str(trimmed)?)
}

fn extract_first_json_object(text: &str) -> Option<String> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (index, ch) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if start.is_none() {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(begin) = start {
                        return Some(text[begin..=index].to_string());
                    }
                }
            }
            _ => {}
        }
    }

    None
}
