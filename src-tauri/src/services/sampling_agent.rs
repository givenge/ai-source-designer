use super::*;

const AGENT_MAX_ROUNDS: usize = 4;
const AGENT_MAX_ACTIONS_PER_ROUND: usize = 2;

fn merge_search_fallback_result(
    pages: &mut Vec<crate::domain::PageSample>,
    network_samples: &mut Vec<crate::domain::NetworkSample>,
    manifest: &mut ArtifactManifest,
    sampled: SamplingResult,
) {
    for page in sampled.pages {
        if page.role != "search" {
            continue;
        }
        if !pages
            .iter()
            .any(|existing| existing.role == page.role && existing.url == page.url)
        {
            pages.push(page);
        }
    }

    for entry in sampled.manifest.dom {
        if entry.contains("/search.") && !manifest.dom.iter().any(|existing| existing == &entry) {
            manifest.dom.push(entry);
        }
    }
    for entry in sampled.manifest.screenshots {
        if entry.contains("/search.")
            && !manifest
                .screenshots
                .iter()
                .any(|existing| existing == &entry)
        {
            manifest.screenshots.push(entry);
        }
    }
    for entry in sampled.manifest.network {
        if entry.contains("/live-") && !manifest.network.iter().any(|existing| existing == &entry) {
            manifest.network.push(entry);
        }
    }
    for entry in sampled.manifest.logs {
        if !manifest.logs.iter().any(|existing| existing == &entry) {
            manifest.logs.push(entry);
        }
    }

    for sample in sampled.network_samples {
        if !network_samples
            .iter()
            .any(|existing| existing.response_path == sample.response_path)
        {
            network_samples.push(sample);
        }
    }
}

pub(super) async fn run_agent_sampling(
    state: &AppState,
    task: &TaskRecord,
    ai_settings: &StartAiSettings,
    api_key: Option<String>,
    existing: Option<SamplingResult>,
) -> Result<SamplingResult> {
    if task.target_url.starts_with("fixture://") {
        return worker::sample_site(
            state,
            &task.id,
            &task.target_url,
            task.test_keyword.as_deref(),
        );
    }

    let api_key = api_key
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            load_settings(&state.paths)
                .ok()
                .map(|settings| settings.api_key)
                .filter(|value| !value.trim().is_empty())
        });

    if ai_settings.base_url.starts_with("mock://") || api_key.is_none() {
        return worker::sample_site(
            state,
            &task.id,
            &task.target_url,
            task.test_keyword.as_deref(),
        );
    }

    let mut pages = existing
        .as_ref()
        .map(|sampled| sampled.pages.clone())
        .unwrap_or_default();
    let mut network_samples = existing
        .as_ref()
        .map(|sampled| sampled.network_samples.clone())
        .unwrap_or_default();
    let mut manifest = existing
        .as_ref()
        .map(|sampled| sampled.manifest.clone())
        .unwrap_or_default();
    let mut inspections = if let Some(sampled) = existing.as_ref() {
        load_inspections_from_sampled(&state.paths, &task.id, sampled)?
    } else {
        Vec::new()
    };
    let mut opened_keys = pages
        .iter()
        .map(|page| {
            format!(
                "{}::{}",
                canonical_agent_key(&page.url),
                normalize_agent_role(&page.role)
            )
        })
        .collect::<std::collections::HashSet<_>>();

    if !pages.iter().any(|page| page.role == "landing") {
        let landing = worker::inspect_page(state, &task.id, &task.target_url, "landing", None)?;
        merge_inspection(
            &mut pages,
            &mut network_samples,
            &mut manifest,
            &mut inspections,
            landing,
        );
    }

    let key = api_key.expect("checked api key presence");
    for round in 0..AGENT_MAX_ROUNDS {
        ensure_not_cancelled(state, &task.id)?;
        if agent_has_minimum_coverage(&pages) {
            break;
        }

        let plan = request_agent_plan(
            &state.paths,
            &task.id,
            ai_settings,
            &key,
            &task.target_url,
            task.test_keyword.as_deref(),
            &inspections,
        )
        .await?;

        if let Some(summary) = plan
            .summary
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            log_to_task_file(
                &state.paths,
                &task.id,
                &format!(
                    "{} [\"sampling\"] agent-plan: {}\n",
                    Utc::now().to_rfc3339(),
                    summary
                ),
            )?;
        }

        if plan.completed || plan.actions.is_empty() {
            break;
        }

        let observed = build_observed_action_inputs(&inspections);

        for action in plan.actions.into_iter().take(AGENT_MAX_ACTIONS_PER_ROUND) {
            ensure_not_cancelled(state, &task.id)?;
            let url = action.url.trim();
            let role = normalize_agent_role(&action.role);
            if url.is_empty() {
                continue;
            }
            let key_ref = format!("{}::{role}", canonical_agent_key(url));
            if opened_keys.contains(&key_ref) {
                continue;
            }

            if let Some(reason) = validate_agent_action(&action, &observed) {
                log_to_task_file(
                    &state.paths,
                    &task.id,
                    &format!(
                        "{} [\"sampling\"] agent-action-rejected round={} kind={} role={} url={} reason={}\n",
                        Utc::now().to_rfc3339(),
                        round + 1,
                        action.kind,
                        role,
                        url,
                        reason
                    ),
                )?;
                continue;
            }

            log_to_task_file(
                &state.paths,
                &task.id,
                &format!(
                    "{} [\"sampling\"] agent-action-plan round={} kind={} role={} url={} waitForSelector={} formAction={} formMethod={} queryParam={}\n",
                    Utc::now().to_rfc3339(),
                    round + 1,
                    action.kind,
                    role,
                    url,
                    action.wait_for_selector.as_deref().unwrap_or(""),
                    action.form_action.as_deref().unwrap_or(""),
                    action.form_method.as_deref().unwrap_or(""),
                    action.query_param.as_deref().unwrap_or("")
                ),
            )?;

            let inspection = match action.kind.trim().to_ascii_lowercase().as_str() {
                "search" => {
                    let Some(keyword) = task
                        .test_keyword
                        .as_deref()
                        .filter(|value| !value.trim().is_empty())
                    else {
                        continue;
                    };
                    worker::inspect_search_page(
                        state,
                        &task.id,
                        url,
                        keyword,
                        &role,
                        action.form_action.as_deref(),
                        action.form_method.as_deref(),
                        action.query_param.as_deref(),
                        action.wait_for_selector.as_deref(),
                    )?
                }
                _ => worker::inspect_page(
                    state,
                    &task.id,
                    url,
                    &role,
                    action.wait_for_selector.as_deref(),
                )?,
            };

            opened_keys.insert(key_ref);
            if let Some(reason) = action
                .reason
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                log_to_task_file(
                    &state.paths,
                    &task.id,
                    &format!(
                        "{} [\"sampling\"] agent-action round={} role={} url={} reason={}\n",
                        Utc::now().to_rfc3339(),
                        round + 1,
                        role,
                        url,
                        reason
                    ),
                )?;
            }
            merge_inspection(
                &mut pages,
                &mut network_samples,
                &mut manifest,
                &mut inspections,
                inspection,
            );

            if (role == "explore" || role == "search")
                && !pages.iter().any(|page| page.role == "book")
            {
                if let Some(book_url) = inspections.last().and_then(|page| {
                    infer_followup_book_url_from_dom(
                        &state.paths,
                        &task.id,
                        page,
                        task.test_keyword.as_deref(),
                    )
                    .or_else(|| infer_followup_book_url(page, task.test_keyword.as_deref()))
                }) {
                    let book_key = format!("{}::book", canonical_agent_key(&book_url));
                    if !opened_keys.contains(&book_key) {
                        log_to_task_file(
                            &state.paths,
                            &task.id,
                            &format!(
                                "{} [\"sampling\"] auto-followup role=book url={} reason={} page exposed detail link\n",
                                Utc::now().to_rfc3339(),
                                book_url,
                                role
                            ),
                        )?;
                        let book_inspection =
                            worker::inspect_page(state, &task.id, &book_url, "book", None)?;
                        opened_keys.insert(book_key);
                        merge_inspection(
                            &mut pages,
                            &mut network_samples,
                            &mut manifest,
                            &mut inspections,
                            book_inspection,
                        );
                    }
                }
            }

            if !pages.iter().any(|page| page.role == "toc")
                && inspections
                    .last()
                    .is_some_and(|page| page.page.role == "book")
            {
                if let Some(toc_url) = inspections.last().and_then(|page| {
                    infer_followup_toc_url_from_dom(&state.paths, &task.id, page)
                        .or_else(|| infer_followup_toc_url(page))
                }) {
                    let toc_key = format!("{}::toc", canonical_agent_key(&toc_url));
                    if !opened_keys.contains(&toc_key) {
                        log_to_task_file(
                            &state.paths,
                            &task.id,
                            &format!(
                                "{} [\"sampling\"] auto-followup role=toc url={} reason=book page exposed toc link\n",
                                Utc::now().to_rfc3339(),
                                toc_url
                            ),
                        )?;
                        let toc_inspection = worker::inspect_page(
                            state,
                            &task.id,
                            &toc_url,
                            "toc",
                            Some("a[href], dd, li"),
                        )?;
                        opened_keys.insert(toc_key);
                        merge_inspection(
                            &mut pages,
                            &mut network_samples,
                            &mut manifest,
                            &mut inspections,
                            toc_inspection,
                        );
                    }
                }
            }

            if !pages.iter().any(|page| page.role == "content")
                && inspections.last().is_some_and(|page| {
                    matches!(
                        page.page.role.as_str(),
                        "explore" | "search" | "book" | "toc"
                    )
                })
            {
                if let Some(content_url) = inspections.last().and_then(|page| {
                    infer_followup_content_url_from_dom(&state.paths, &task.id, page)
                        .or_else(|| infer_followup_content_url(page))
                }) {
                    let content_key = format!("{}::content", canonical_agent_key(&content_url));
                    if !opened_keys.contains(&content_key) {
                        let content_seed = inspections
                            .last()
                            .map(|page| page.page.role.clone())
                            .unwrap_or_else(|| role.clone());
                        log_to_task_file(
                            &state.paths,
                            &task.id,
                            &format!(
                                "{} [\"sampling\"] auto-followup role=content url={} reason={} page exposed readable chapter link\n",
                                Utc::now().to_rfc3339(),
                                content_url,
                                content_seed
                            ),
                        )?;
                        let content_inspection =
                            worker::inspect_page(state, &task.id, &content_url, "content", None)?;
                        opened_keys.insert(content_key);
                        merge_inspection(
                            &mut pages,
                            &mut network_samples,
                            &mut manifest,
                            &mut inspections,
                            content_inspection,
                        );
                    }
                }
            }
        }
    }

    if !pages.iter().any(|page| page.role == "search") {
        let fallback = worker::sample_site(
            state,
            &task.id,
            &task.target_url,
            task.test_keyword.as_deref(),
        )?;
        merge_search_fallback_result(&mut pages, &mut network_samples, &mut manifest, fallback);
    }

    Ok(SamplingResult {
        fixture_name: None,
        site_label: Url::parse(&task.target_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_string))
            .unwrap_or_else(|| task.target_url.clone()),
        base_url: Url::parse(&task.target_url)
            .ok()
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_else(|| task.target_url.clone()),
        pages,
        network_samples,
        manifest,
    })
}
