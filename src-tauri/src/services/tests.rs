use super::*;
use crate::domain::{
    AiSettings, ArtifactManifest, BookInfoRule, ContentRule, SearchRule, SourceCandidate, TocRule,
};
use crate::services::agent_flow::parse_agent_plan_response;
use std::fs;

fn good_candidate() -> SourceCandidate {
    SourceCandidate {
        strategy: "test".into(),
        confidence: 0.9,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "Fixture".into(),
            book_source_url: "https://fixture.novel.world".into(),
            enabled: Some(true),
            search_url: Some("https://fixture.novel.world/search?q={key}&page={page}".into()),
            rule_search: Some(SearchRule {
                book_list: Some(".book-item".into()),
                name: Some(".book-title a@text".into()),
                author: Some(".author@text".into()),
                book_url: Some(".book-title a@attr[href]".into()),
                intro: Some(".intro@text".into()),
                ..Default::default()
            }),
            rule_book_info: Some(BookInfoRule {
                name: Some("h1.title@text".into()),
                author: Some(".meta .author@text".into()),
                intro: Some(".intro@text".into()),
                toc_url: Some(".toc-link@attr[href]".into()),
                ..Default::default()
            }),
            rule_toc: Some(TocRule {
                chapter_list: Some(".chapter-list li".into()),
                chapter_name: Some("a@text".into()),
                chapter_url: Some("a@attr[href]".into()),
                ..Default::default()
            }),
            rule_content: Some(ContentRule {
                content: Some(".content@textNodes".into()),
                next_content_url: Some(".next-page@attr[href]".into()),
                replace_regex: Some("Sponsored by AI Source Designer##".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    }
}

fn diagnostic_test_paths() -> AppPaths {
    let root = std::env::temp_dir().join(format!("asd-diagnostics-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("tasks")).unwrap();
    AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures"),
        worker_script: root.join("worker.mjs"),
    }
}

fn mock_model_server(status: u16, body: String) -> (String, std::thread::JoinHandle<Vec<u8>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut chunk = [0; 4096];
            let count = stream.read(&mut chunk).unwrap();
            assert!(count > 0, "request ended before its body was received");
            request.extend_from_slice(&chunk[..count]);
            if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap_or("0")
                    .parse::<usize>()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        write!(
            stream,
            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        request
    });
    (endpoint, server)
}

#[test]
fn connection_probe_checks_models_redacts_errors_and_supports_local_no_auth() {
    for (status, body, secret, expected_error) in [
        (
            200,
            json!({"data":[{"id":"local-model"}]}).to_string(),
            "",
            None,
        ),
        (
            401,
            "unauthorized test-private-key".into(),
            "test-private-key",
            Some("HTTP 401"),
        ),
        (
            200,
            json!({"data":[{"id":"other-model"}]}).to_string(),
            "test-private-key",
            Some("未找到"),
        ),
        (200, "<html>not an API</html>".into(), "", Some("模型列表")),
    ] {
        let (base_url, server) = mock_model_server(status, body);
        let result = tauri::async_runtime::block_on(test_ai_connection(&AiSettings {
            base_url: format!("{base_url}/chat/completions"),
            model: " local-model ".into(),
            api_key: secret.into(),
        }));
        let request = String::from_utf8(server.join().unwrap()).unwrap();
        assert!(request.starts_with("GET /v1/models HTTP/1.1"));
        assert_eq!(
            request.to_ascii_lowercase().contains("authorization:"),
            !secret.is_empty()
        );
        if let Some(expected) = expected_error {
            let error = result.unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
            assert!(!error.contains("test-private-key"));
        } else {
            let result = result.unwrap();
            assert_eq!(result.model, "local-model");
            assert!(!result.mock);
        }
    }
    let mock = tauri::async_runtime::block_on(test_ai_connection(&AiSettings::default())).unwrap();
    assert!(mock.mock);
    assert!(
        tauri::async_runtime::block_on(test_ai_connection(&AiSettings {
            base_url: "file:///tmp/api".into(),
            model: "local".into(),
            api_key: "".into(),
        }))
        .unwrap_err()
        .to_string()
        .contains("http 或 https")
    );
}

#[test]
fn model_key_fallback_is_scoped_to_the_saved_endpoint_and_respects_empty_input() {
    let paths = diagnostic_test_paths();
    save_settings(
        &paths,
        &AiSettings {
            base_url: "https://saved.example/v1/".into(),
            model: "saved".into(),
            api_key: "saved-private-key".into(),
        },
    )
    .unwrap();
    let mut requested = StartAiSettings {
        base_url: "https://saved.example/v1/chat/completions".into(),
        model: "another-model".into(),
    };
    assert_eq!(
        resolve_model_key(&paths, &requested, None),
        "saved-private-key"
    );
    assert_eq!(resolve_model_key(&paths, &requested, Some("  ".into())), "");
    requested.base_url = "http://127.0.0.1:11434/v1".into();
    assert_eq!(resolve_model_key(&paths, &requested, None), "");
    assert_eq!(
        resolve_model_key(&paths, &requested, Some(" explicit-key ".into())),
        "explicit-key"
    );
    assert_eq!(load_settings(&paths).unwrap().api_key, "saved-private-key");
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn runtime_diagnostics_report_missing_runtime_without_creating_a_database() {
    let paths = diagnostic_test_paths();
    let missing = runtime_diagnostics(&paths);
    assert!(!missing.database_ready);
    assert!(!missing.worker_ready);
    assert!(missing.error.unwrap().contains("Worker 文件缺失"));
    assert!(!paths.db_path.exists());
    init_database(&paths).unwrap();
    assert!(runtime_diagnostics(&paths).database_ready);
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn runtime_repair_obeys_task_lock_and_releases_it_after_failure() {
    let paths = diagnostic_test_paths();
    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::new(TaskRuntime::default()),
    };
    assert!(state.runtime.register("busy-task"));
    assert!(repair_playwright(&state)
        .unwrap_err()
        .to_string()
        .contains("任务执行中"));
    assert!(!paths.logs_dir.exists());
    state.runtime.unregister("busy-task");
    assert!(repair_playwright(&state)
        .unwrap_err()
        .to_string()
        .contains("Worker 文件缺失"));
    assert!(!state.runtime.is_runtime_repairing());
    assert!(state.runtime.register("next-task"));
    assert!(crate::worker_runtime::repair_log(&paths).contains("Worker 文件缺失"));
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn log_export_copies_the_original_and_rejects_unknown_log_types() {
    let paths = diagnostic_test_paths();
    init_database(&paths).unwrap();
    let task = create_task(
        &paths,
        crate::domain::TaskDraft {
            target_url: "fixture://novel-world".into(),
            test_keyword: None,
            task_mode: TaskMode::Classic,
        },
    )
    .unwrap();
    let original = paths.task_path(&task.id, "logs/task.log");
    let raw = "original task log\nmodel-response: {\"ok\":true}\n";
    fs::write(&original, raw).unwrap();
    let destination = paths.data_dir.join("exported.log");
    export_task_log(&paths, &task.id, "task", destination.to_str().unwrap()).unwrap();
    assert_eq!(fs::read_to_string(&destination).unwrap(), raw);
    assert_eq!(fs::read_to_string(original).unwrap(), raw);
    assert!(export_task_log(
        &paths,
        &task.id,
        "../../settings.json",
        destination.to_str().unwrap()
    )
    .is_err());
    assert!(export_task_log(&paths, &task.id, "task", "").is_err());
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn revalidation_uses_cached_evidence_keeps_failed_reports_and_obeys_the_run_lock() {
    let paths = diagnostic_test_paths();
    init_database(&paths).unwrap();
    let mut task = create_task(
        &paths,
        crate::domain::TaskDraft {
            target_url: "https://fixture.novel.world".into(),
            test_keyword: Some("星门".into()),
            task_mode: TaskMode::Classic,
        },
    )
    .unwrap();
    let fixture = paths.fixtures_dir.join("novel-world");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(fixture.join("manifest.json")).unwrap()).unwrap();
    let mut captures = String::new();
    for page in manifest["pages"].as_array().unwrap() {
        let role = page["role"].as_str().unwrap();
        fs::copy(
            fixture.join(page["htmlFile"].as_str().unwrap()),
            paths.task_path(&task.id, &format!("dom/{role}.html")),
        )
        .unwrap();
        captures.push_str(&format!(
            "capture role={role} url={}\n",
            page["url"].as_str().unwrap()
        ));
    }
    fs::write(paths.task_path(&task.id, "logs/worker.log"), captures).unwrap();
    let original = fs::read(paths.task_path(&task.id, "dom/content.html")).unwrap();
    let mut candidate = good_candidate();
    candidate.book_source.explore_url =
        Some("https://fixture.novel.world/ranks?page={{page}}".into());
    candidate.book_source.enabled_explore = Some(true);
    task.source_candidate = Some(candidate);
    persist_task(&paths, &task).unwrap();
    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::new(TaskRuntime::default()),
    };
    assert!(state.runtime.register(&task.id));
    assert!(revalidate_task(&state, &task.id)
        .unwrap_err()
        .to_string()
        .contains("still running"));
    assert!(state.runtime.is_running(&task.id));
    state.runtime.unregister(&task.id);
    let mut result = revalidate_task(&state, &task.id).unwrap();
    assert!(matches!(result.status, TaskStatus::Completed));
    assert!(report_is_usable(result.validation_report.as_ref().unwrap()));
    assert_eq!(
        result
            .source_candidate
            .as_ref()
            .unwrap()
            .book_source
            .enabled_explore,
        Some(false)
    );
    assert!(paths
        .task_path(&task.id, "output/book-source.json")
        .is_file());
    assert!(!state.runtime.is_running(&task.id));
    result
        .source_candidate
        .as_mut()
        .unwrap()
        .book_source
        .rule_content
        .as_mut()
        .unwrap()
        .content = Some(".missing-content@text".into());
    persist_task(&paths, &result).unwrap();
    let failed = revalidate_task(&state, &task.id).unwrap();
    assert!(matches!(failed.status, TaskStatus::Failed));
    assert!(failed.error_summary.is_some());
    assert!(!failed.validation_report.unwrap().content.passed);
    assert_eq!(
        fs::read(paths.task_path(&task.id, "dom/content.html")).unwrap(),
        original
    );
    assert!(get_task(&paths, &task.id)
        .unwrap()
        .validation_report
        .is_some());
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn model_prompts_include_sampled_dom_json_and_object_contract() {
    let paths = diagnostic_test_paths();
    let task_id = "evidence";
    paths.ensure_task_layout(task_id).unwrap();
    let mut sampled = SamplingResult {
        site_label: "Evidence".into(),
        base_url: "https://evidence.example".into(),
        ..Default::default()
    };
    for role in ["explore", "book", "toc", "content"] {
        let html_path = format!("dom/{role}.html");
        fs::write(
            paths.task_path(task_id, &html_path),
            format!(
                "<html><head><title>{role} 标题</title><script>{}</script></head><body><main class=\"actual-{role}\"><a href=\"/observed-book\">真实链接</a></main></body></html>",
                "ignore-script".repeat(2000)
            ),
        )
        .unwrap();
        sampled.pages.push(crate::domain::PageSample {
            role: role.into(),
            url: format!("https://evidence.example/{role}"),
            html_path,
            ..Default::default()
        });
    }
    fs::write(
        paths.task_path(task_id, "network/search.json"),
        r#"{"books":[{"name":"JSON 书籍","url":"/observed-book"}]}"#,
    )
    .unwrap();
    sampled.network_samples.push(crate::domain::NetworkSample {
        method: "GET".into(),
        request_url: "https://evidence.example/api".into(),
        response_path: "network/search.json".into(),
        ..Default::default()
    });
    let generation = build_prompt(&paths, task_id, &sampled).unwrap();
    let repair = build_repair_prompt(
        &paths,
        task_id,
        &sampled,
        &good_candidate(),
        "bookInfo: author missing; toc: chapterList missing",
    )
    .unwrap();
    for prompt in [&generation, &repair] {
        for role in ["explore", "book", "toc", "content"] {
            assert!(prompt.contains(&format!("actual-{role}")));
            assert!(prompt.contains(&format!("dom/{role}.html")));
        }
        assert!(prompt.contains("JSON 书籍"));
        assert!(prompt.contains("network/search.json"));
        assert!(prompt.contains("JSON objects or null"));
        assert!(prompt.contains("\"ruleBookInfo\": {"));
        assert!(prompt.contains("{{key}}"));
        assert!(!prompt.contains("ignore-script"));
    }
    assert!(repair.contains("bookInfo: author missing; toc: chapterList missing"));
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn repair_merge_replaces_existing_rules_and_keeps_omitted_fields() {
    let mut current = good_candidate();
    current.book_source.book_source_group = Some("用户分组".into());
    let repaired = SourceCandidate {
        strategy: "repair".into(),
        book_source: BookSource {
            book_source_name: "模型默认名称".into(),
            book_source_url: "https://different.example".into(),
            search_url: Some("https://fixture.novel.world/find?q={{key}}".into()),
            rule_search: Some(SearchRule {
                book_list: Some(".correct-results".into()),
                author: Some(" ".into()),
                ..Default::default()
            }),
            rule_book_info: Some(BookInfoRule {
                name: Some("h1.correct-title@text".into()),
                ..Default::default()
            }),
            rule_toc: Some(TocRule {
                chapter_list: Some("#correct-chapters li".into()),
                ..Default::default()
            }),
            rule_content: Some(ContentRule {
                content: Some("article@textNodes".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        ..Default::default()
    };
    let conservative = merge_candidates(current.clone(), repaired.clone());
    assert_eq!(
        conservative
            .book_source
            .rule_search
            .unwrap()
            .book_list
            .as_deref(),
        Some(".book-item")
    );
    let merged = merge_repair_candidates(current, repaired).book_source;
    assert_eq!(merged.book_source_name, "Fixture");
    assert_eq!(merged.book_source_url, "https://fixture.novel.world");
    assert_eq!(merged.book_source_group.as_deref(), Some("用户分组"));
    assert_eq!(
        merged.search_url.as_deref(),
        Some("https://fixture.novel.world/find?q={{key}}")
    );
    let search = merged.rule_search.unwrap();
    assert_eq!(search.book_list.as_deref(), Some(".correct-results"));
    assert_eq!(search.author.as_deref(), Some(".author@text"));
    assert_eq!(
        merged.rule_book_info.unwrap().name.as_deref(),
        Some("h1.correct-title@text")
    );
    assert_eq!(
        merged.rule_toc.unwrap().chapter_list.as_deref(),
        Some("#correct-chapters li")
    );
    let content = merged.rule_content.unwrap();
    assert_eq!(content.content.as_deref(), Some("article@textNodes"));
    assert_eq!(
        content.replace_regex.as_deref(),
        Some("Sponsored by AI Source Designer##")
    );
}

#[test]
fn task_log_flush_preserves_model_diagnostics_without_duplicate_events() {
    let paths = diagnostic_test_paths();
    let mut task = TaskRecord::new(crate::domain::TaskDraft {
        target_url: "https://evidence.example".into(),
        test_keyword: None,
        task_mode: TaskMode::Agent,
    });
    paths.ensure_task_layout(&task.id).unwrap();
    append_log(&mut task, TaskStatus::Draft, "开始");
    write_logs(&paths, &task).unwrap();
    log_to_task_file(
        &paths,
        &task.id,
        "model-request retained\nmodel-response retained\n",
    )
    .unwrap();
    append_log(&mut task, TaskStatus::Validating, "验证结果：\n第二行");
    write_logs(&paths, &task).unwrap();
    write_logs(&paths, &task).unwrap();
    let log = fs::read_to_string(paths.task_path(&task.id, "logs/task.log")).unwrap();
    assert!(log.contains("model-request retained\nmodel-response retained"));
    assert_eq!(log.matches("开始").count(), 1);
    assert_eq!(log.matches("验证结果：\n第二行").count(), 1);
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn model_exchange_logs_preserve_response_and_redact_api_key() {
    let paths = diagnostic_test_paths();
    let secret = "test-api-key-private";
    for status in [200, 520] {
        let body = if status == 200 {
            json!({"choices":[{"message":{"content":format!("model reply {secret}")}}]}).to_string()
        } else {
            json!({"error":format!("upstream failed {secret}")}).to_string()
        };
        let (base_url, server) = mock_model_server(status, body);
        let task_id = format!("http-{status}");
        let result = tauri::async_runtime::block_on(request_model_message(
            &paths,
            &task_id,
            "test",
            &StartAiSettings {
                base_url,
                model: "test-model".into(),
            },
            secret,
            "test system",
            &format!("test user {secret}"),
        ));
        server.join().unwrap();
        if status == 200 {
            assert_eq!(result.unwrap(), format!("model reply {secret}"));
        } else {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("HTTP 520"));
            assert!(!error.contains(secret));
        }
        let log = fs::read_to_string(paths.task_path(&task_id, "logs/task.log")).unwrap();
        assert!(log.contains("model-request-payload"));
        assert!(log.contains("model-response"));
        assert!(log.contains(&format!("status={status}")));
        assert!(log.contains("[REDACTED]"));
        assert!(!log.contains(secret));
    }
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn failed_model_repair_still_runs_sampled_fallback_and_keeps_validation_logs() {
    let paths = diagnostic_test_paths();
    init_database(&paths).unwrap();
    let mut task = create_task(
        &paths,
        crate::domain::TaskDraft {
            target_url: "https://fixture.novel.world".into(),
            test_keyword: Some("星门".into()),
            task_mode: TaskMode::Classic,
        },
    )
    .unwrap();
    let fixture = paths.fixtures_dir.join("novel-world");
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(fixture.join("manifest.json")).unwrap()).unwrap();
    let mut artifacts = ArtifactManifest::default();
    let mut captures = String::new();
    for page in manifest["pages"].as_array().unwrap() {
        let role = page["role"].as_str().unwrap();
        let relative = format!("dom/{role}.html");
        fs::copy(
            fixture.join(page["htmlFile"].as_str().unwrap()),
            paths.task_path(&task.id, &relative),
        )
        .unwrap();
        artifacts.dom.push(relative);
        captures.push_str(&format!(
            "capture role={role} url={}\n",
            page["url"].as_str().unwrap()
        ));
    }
    fs::write(paths.task_path(&task.id, "logs/worker.log"), captures).unwrap();
    artifacts.logs = vec!["logs/task.log".into(), "logs/worker.log".into()];
    task.artifact_manifest = Some(artifacts);
    let mut candidate = good_candidate();
    candidate.book_source.rule_book_info.as_mut().unwrap().name = Some(".wrong-title@text".into());
    task.source_candidate = Some(candidate);
    persist_task(&paths, &task).unwrap();
    let (base_url, server) = mock_model_server(520, "error code: 520".into());
    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::new(TaskRuntime::default()),
    };
    let result = tauri::async_runtime::block_on(run_task(
        &state,
        &task.id,
        StartAiSettings {
            base_url,
            model: "test-model".into(),
        },
        Some("test-key".into()),
    ))
    .unwrap();
    server.join().unwrap();
    assert!(matches!(result.status, TaskStatus::Completed));
    assert!(report_is_usable(result.validation_report.as_ref().unwrap()));
    let log = fs::read_to_string(paths.task_path(&task.id, "logs/task.log")).unwrap();
    assert!(log.contains("HTTP 520"));
    assert!(log.contains("继续尝试本地采样证据修复"));
    assert!(log.contains("验证结果"));
    assert!(log.contains("model-request-payload"));
    assert!(log.contains("model-response"));
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn apply_replace_regex_removes_fixture_ad_copy() {
    let cleaned = apply_replace_regex(
        "正文\nSponsored by AI Source Designer\n继续正文",
        Some("Sponsored by AI Source Designer##"),
    )
    .unwrap();
    assert!(!cleaned.contains("Sponsored by AI Source Designer"));
}

#[test]
fn normalize_url_expands_relative_links() {
    let resolved = normalize_url(
        "https://fixture.novel.world/book/solar-archive",
        "/book/solar-archive/chapter-1",
    )
    .unwrap();
    assert_eq!(
        resolved,
        "https://fixture.novel.world/book/solar-archive/chapter-1"
    );
}

#[test]
fn serialized_book_source_omits_unset_optional_fields() {
    let source = serde_json::to_value(good_candidate().book_source).unwrap();
    assert!(source.pointer("/ruleContent/title").is_none());
    assert!(source.pointer("/ruleToc/updateTime").is_none());
}

#[test]
fn css_descendant_rules_survive_normalization_and_reader_export() {
    let mut candidate = good_candidate();
    let search = candidate.book_source.rule_search.as_mut().unwrap();
    search.book_list = Some("main .item".into());
    search.name = Some(" @css: h3 a@text ".into());
    search.book_url = Some("@CSS: h3 a@attr[href]".into());
    search.author = Some("@css::is(p.byline, p.writer)@text".into());
    search.check_key_word = Some("h3 a".into());
    let toc = candidate.book_source.rule_toc.as_mut().unwrap();
    toc.chapter_list = Some("ul.catalog li".into());
    toc.pre_update_js = Some("return response".into());
    let content = candidate.book_source.rule_content.as_mut().unwrap();
    content.content = Some("article p@textNodes".into());
    content.replace_regex = Some("h3 a##replacement".into());
    content.source_regex = Some("h3 a".into());

    sanitize_candidate(&mut candidate).unwrap();
    let exported = exportable_book_source_value(&candidate.book_source).unwrap();
    assert_eq!(exported["ruleSearch"]["bookList"], "@css::is(main .item)");
    assert_eq!(exported["ruleSearch"]["name"], "@css::is(h3 a)@text");
    assert_eq!(exported["ruleSearch"]["bookUrl"], "@css::is(h3 a)@href");
    assert_eq!(
        exported["ruleSearch"]["author"],
        "@css::is(p.byline, p.writer)@text"
    );
    assert_eq!(exported["ruleSearch"]["checkKeyWord"], "h3 a");
    assert_eq!(
        exported["ruleToc"]["chapterList"],
        "@css::is(ul.catalog li)"
    );
    assert_eq!(exported["ruleToc"]["preUpdateJs"], "return response");
    assert_eq!(
        exported["ruleContent"]["content"],
        "@css::is(article p)@textNodes"
    );
    assert_eq!(exported["ruleContent"]["replaceRegex"], "h3 a##replacement");
    assert_eq!(exported["ruleContent"]["sourceRegex"], "h3 a");
    for rule in [
        "class.book item@text",
        "id.books a@href",
        "tag.div a@text",
        "@json:$.books",
        "//div/a/@href",
        "@js:return response",
    ] {
        assert_eq!(rule_syntax::normalize_css_rule_for_reader(rule), rule);
    }
    assert_eq!(
        rule_syntax::normalize_css_rule_for_reader(" @css: h3 a@text##prefix## "),
        "@css::is(h3 a)@text##prefix##"
    );

    let document = Html::parse_document(
        r#"<main><div class="item"><h3><a href="/one">One</a></h3></div></main>"#,
    );
    let item = document
        .select(&selector(exported["ruleSearch"]["bookList"].as_str().unwrap()).unwrap())
        .next()
        .unwrap();
    assert_eq!(
        evaluate_on_element_safe(&item, exported["ruleSearch"]["name"].as_str().unwrap()).unwrap(),
        "One"
    );
    assert_eq!(
        evaluate_on_element_safe(&item, exported["ruleSearch"]["bookUrl"].as_str().unwrap())
            .unwrap(),
        "/one"
    );
}

#[test]
fn list_field_rules_cannot_escape_the_item_scope() {
    let document = Html::parse_document(
        r#"<section class="catalog"><div class="item"><h3><a href="/one">One</a></h3></div><div class="item"><h3><a href="/two">Two</a></h3></div></section>"#,
    );
    let item = document.select(&selector(".item").unwrap()).next().unwrap();
    assert_eq!(
        evaluate_on_element_safe(&item, "h3 a@href").unwrap(),
        "/one"
    );
    assert_eq!(
        evaluate_on_element_safe(&item, "@css::is(h3 a)@href").unwrap(),
        "/one"
    );
    assert_eq!(
        document
            .select(&selector("@css::is(div.item h3 a)").unwrap())
            .count(),
        2
    );
    assert_eq!(
        evaluate_on_element_safe(&item, ".item h3 a@href").unwrap(),
        ""
    );
    assert_eq!(
        evaluate_on_element_safe(&item, ".catalog a@href").unwrap(),
        ""
    );
    assert_eq!(
        evaluate_on_element_safe(&item, "h3 a, a@href").unwrap(),
        "/one"
    );
    assert_eq!(evaluate_on_element_safe(&item, "@text").unwrap(), "One");
    let anchor = item.select(&selector("a").unwrap()).next().unwrap();
    assert_eq!(evaluate_on_element_safe(&anchor, "@href").unwrap(), "/one");
    let table = Html::parse_document(
        r#"<table><tbody><tr class="item"><td><a href="/cell">Cell</a></td></tr></tbody></table>"#,
    );
    let row = table.select(&selector("tr").unwrap()).next().unwrap();
    assert_eq!(
        evaluate_on_element_safe(&row, "td a@href").unwrap(),
        "/cell"
    );
    let nested = Html::parse_document(
        r#"<div class="item"><div class="item"><h3><a href="/nested">Nested</a></h3></div></div>"#,
    );
    let outer = nested.select(&selector(".item").unwrap()).next().unwrap();
    assert_eq!(
        evaluate_on_element_safe(&outer, ".item h3 a@href").unwrap(),
        "/nested"
    );
}

#[test]
fn url_templates_are_not_exported_as_book_url_patterns() {
    for pattern in [
        "https://books.test/{{id}}",
        "https://books.test/${id}",
        "[invalid",
    ] {
        let mut candidate = good_candidate();
        candidate.book_source.book_url_pattern = Some(pattern.into());
        let exported = exportable_book_source_value(&candidate.book_source).unwrap();
        assert!(exported["bookUrlPattern"].is_null(), "{pattern}");
        sanitize_candidate(&mut candidate).unwrap();
        assert!(
            candidate.book_source.book_url_pattern.is_none(),
            "{pattern}"
        );
    }
    let mut candidate = good_candidate();
    candidate.book_source.book_url_pattern = Some(r"https://books\.test/book/\d{1,6}$".into());
    sanitize_candidate(&mut candidate).unwrap();
    assert_eq!(
        candidate.book_source.book_url_pattern.as_deref(),
        Some(r"https://books\.test/book/\d{1,6}$")
    );
}

#[test]
fn explore_validation_checks_every_list_item_and_its_scope() {
    let paths = diagnostic_test_paths();
    let task_id = "explore-scope";
    paths.ensure_task_layout(task_id).unwrap();
    let page_path = paths.task_path(task_id, "dom/explore.html");
    let sampled = SamplingResult {
        pages: vec![crate::domain::PageSample {
            role: "explore".into(),
            url: "https://books.test/catalog".into(),
            html_path: "dom/explore.html".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut candidate = good_candidate();
    candidate.book_source.explore_url = Some("All::https://books.test/catalog".into());
    candidate.book_source.rule_explore = Some(ExploreRule {
        book_list: Some(".item".into()),
        name: Some("h3 a@text".into()),
        book_url: Some("h3 a@href".into()),
        ..Default::default()
    });
    fs::write(&page_path, r#"<div class="item"><h3><a href="/one">One</a></h3></div><div class="item"><h3><a>Two</a></h3></div>"#).unwrap();
    let report = validate_candidate(&paths, task_id, &sampled, &candidate, None).unwrap();
    assert!(!report.explore.passed);
    assert!(report.explore.details[0].contains("第 2 条"));
    fs::write(&page_path, r#"<div class="item"><h3><a href="/one">One</a></h3></div><div class="item"><h3><a href="/two">Two</a></h3></div>"#).unwrap();
    assert!(
        validate_candidate(&paths, task_id, &sampled, &candidate, None)
            .unwrap()
            .explore
            .passed
    );
    candidate
        .book_source
        .rule_explore
        .as_mut()
        .unwrap()
        .book_url = Some(".item h3 a@href".into());
    assert!(
        !validate_candidate(&paths, task_id, &sampled, &candidate, None)
            .unwrap()
            .explore
            .passed
    );
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn explore_pagination_templates_follow_observed_urls() {
    for (first, next, third) in [
        (
            "https://books.test/catalog",
            "https://books.test/catalog/2",
            "https://books.test/catalog/3",
        ),
        (
            "https://books.test/list?category=fantasy&page=1",
            "https://books.test/list?category=fantasy&page=2",
            "https://books.test/list?category=fantasy&page=3",
        ),
        (
            "https://books.test/catalogue/index.html",
            "https://books.test/catalogue/page-2.html",
            "https://books.test/catalogue/page-3.html",
        ),
        (
            "http://[::1]/catalog",
            "http://[::1]/catalog/2",
            "http://[::1]/catalog/3",
        ),
    ] {
        let template = explore::pagination_template(first, next).unwrap();
        assert_eq!(
            explore::page_url(&template, first, 1).as_deref(),
            Some(first)
        );
        assert_eq!(
            explore::page_url(&template, first, 2).as_deref(),
            Some(next)
        );
        assert_eq!(
            explore::page_url(&template, first, 3).as_deref(),
            Some(third)
        );
    }
    for (first, next) in [
        (
            "https://books.test/catalog",
            "https://books.test/catalog/20",
        ),
        (
            "https://books.test/list?page=10",
            "https://books.test/list?page=20",
        ),
        (
            "https://books.test/catalog",
            "https://external.test/catalog/2",
        ),
        ("https://books.test/catalog", "https://books.test/unrelated"),
        (
            "https://books.test/catalog",
            "https://books.test/catalog%2Fnext",
        ),
    ] {
        assert!(
            explore::pagination_template(first, next).is_none(),
            "{next}"
        );
    }
    let first = "https://books.test/list?category=fantasy&page=1";
    assert_eq!(
        explore::urls("http://[::1]/catalog"),
        vec!["http://[::1]/catalog"]
    );
    let template =
        explore::pagination_template(first, "https://books.test/list?category=fantasy&page=2")
            .unwrap();
    let sibling = explore::category_template(
        first,
        &template,
        "https://books.test/list?category=scifi&page=1",
    )
    .unwrap();
    assert_eq!(
        explore::page_url(&sibling, first, 3).as_deref(),
        Some("https://books.test/list?category=scifi&page=3")
    );
    let first = "https://books.test/catalogue/index.html";
    let template =
        explore::pagination_template(first, "https://books.test/catalogue/page-2.html").unwrap();
    let sibling =
        explore::category_template(first, &template, "https://books.test/fiction/index.html")
            .unwrap();
    assert_eq!(
        explore::page_url(&sibling, first, 3).as_deref(),
        Some("https://books.test/fiction/page-3.html")
    );
    assert!(explore::category_template(
        first,
        &template,
        "https://external.test/fiction/index.html"
    )
    .is_none());
    assert!(explore::category_template(
        first,
        &template,
        "https://books.test/other/depth/index.html"
    )
    .is_none());
}

#[test]
fn explore_pagination_preserves_groups_styles_and_existing_templates() {
    let paths = diagnostic_test_paths();
    let task_id = "explore-template";
    paths.ensure_task_layout(task_id).unwrap();
    fs::write(
        paths.task_path(task_id, "dom/explore.html"),
        r#"<a href="/catalog/2" rel="next">More</a>"#,
    )
    .unwrap();
    let sampled = SamplingResult {
        pages: vec![crate::domain::PageSample {
            role: "explore".into(),
            url: "https://books.test/catalog".into(),
            html_path: "dom/explore.html".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let original = json!([
        {"title":"Categories", "url":"", "style":{"layout_flexGrow":1, "layout_flexBasisPercent":1}},
        {"title":"All", "url":"/catalog", "style":{"layout_flexBasisPercent":0.2}},
        {"title":"Fiction", "url":"/fiction"},
        {"title":"Custom", "url":"/custom{{page == 1 ? '' : '/' + page}}"},
        {"title":"Different structure", "url":"/other/list"}
    ]);
    let mut source = good_candidate().book_source;
    source.book_source_url = "https://books.test".into();
    source.explore_url = Some(original.to_string());
    explore::complete_pagination(&paths, task_id, &sampled, &mut source).unwrap();
    let updated: Value = serde_json::from_str(source.explore_url.as_ref().unwrap()).unwrap();
    assert_eq!(updated[0], original[0]);
    assert_eq!(updated[1]["style"], original[1]["style"]);
    assert_eq!(updated[3], original[3]);
    assert_eq!(updated[4], original[4]);
    for (index, expected) in [
        (1, "https://books.test/catalog/2"),
        (2, "https://books.test/fiction/2"),
    ] {
        assert_eq!(
            explore::page_url(
                updated[index]["url"].as_str().unwrap(),
                &source.book_source_url,
                2
            )
            .as_deref(),
            Some(expected)
        );
    }
    source.explore_url = Some("All::/catalog".into());
    explore::complete_pagination(&paths, task_id, &sampled, &mut source).unwrap();
    assert_eq!(
        explore::page_url(
            &explore::urls(source.explore_url.as_ref().unwrap())[0],
            &source.book_source_url,
            2
        )
        .as_deref(),
        Some("https://books.test/catalog/2")
    );
    let document = Html::parse_document(
        r#"<a href="https://external.test/catalog/2" rel="next">Next</a><a href="/catalog#top">下一页</a><a href="/catalog/2">下一页</a>"#,
    );
    assert_eq!(
        explore::next_page_url("https://books.test/catalog", &document).as_deref(),
        Some("https://books.test/catalog/2")
    );
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn explore_validation_requires_real_new_books_on_the_next_page() {
    let paths = diagnostic_test_paths();
    let task_id = "explore-pagination";
    paths.ensure_task_layout(task_id).unwrap();
    let first_html = r#"<div class="item"><h3><a href="/one">One</a></h3></div><a href="/catalog/2" rel="next">Next</a>"#;
    fs::write(paths.task_path(task_id, "dom/explore.html"), first_html).unwrap();
    let mut sampled = SamplingResult {
        pages: vec![crate::domain::PageSample {
            role: "explore".into(),
            url: "https://books.test/catalog".into(),
            html_path: "dom/explore.html".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let mut candidate = good_candidate();
    candidate.book_source.book_source_url = "https://books.test".into();
    candidate.book_source.explore_url = Some("All::/catalog".into());
    candidate.book_source.rule_explore = Some(ExploreRule {
        book_list: Some(".item".into()),
        name: Some("h3 a@text".into()),
        book_url: Some("h3 a@href".into()),
        ..Default::default()
    });
    let validate = |sampled: &SamplingResult, candidate: &SourceCandidate| {
        validate_candidate(&paths, task_id, sampled, candidate, None)
            .unwrap()
            .explore
    };
    assert!(validate(&sampled, &candidate).summary.contains("分页 URL"));
    explore::complete_pagination(&paths, task_id, &sampled, &mut candidate.book_source).unwrap();
    assert!(validate(&sampled, &candidate)
        .summary
        .contains("尚未实际验证"));
    sampled.pages.push(crate::domain::PageSample {
        role: "explore-next".into(),
        url: "https://books.test/catalog/2".into(),
        html_path: "dom/explore-next.html".into(),
        ..Default::default()
    });
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        first_html,
    )
    .unwrap();
    assert!(validate(&sampled, &candidate).summary.contains("重复首页"));
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        "<p>No books</p>",
    )
    .unwrap();
    assert!(!validate(&sampled, &candidate).passed);
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        r#"<div class="item"><h3><a href="https://external.test/two">Two</a></h3></div>"#,
    )
    .unwrap();
    assert!(validate(&sampled, &candidate).summary.contains("站内链接"));
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        r#"<div class="item"><h3><a href="/two">Two</a></h3></div>"#,
    )
    .unwrap();
    assert!(validate(&sampled, &candidate).passed);
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        r#"<div class="item"><h3><a href="/two">Two</a></h3></div><a href="/catalog/4" rel="next">Next</a>"#,
    )
    .unwrap();
    assert!(validate(&sampled, &candidate)
        .summary
        .contains("第 3 页地址"));
    fs::write(
        paths.task_path(task_id, "dom/explore-next.html"),
        r#"<div class="item"><h3><a href="/two">Two</a></h3></div><a href="/catalog/3" rel="next">Next</a>"#,
    )
    .unwrap();
    assert!(validate(&sampled, &candidate).passed);
    sampled.pages[1].url = "https://books.test/catalog/3".into();
    assert!(validate(&sampled, &candidate).summary.contains("地址不符"));
    sampled.pages.clear();
    assert!(!validate(&sampled, &candidate).passed);
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn inferred_explore_categories_use_navigation_and_pagination_evidence() {
    let paths = diagnostic_test_paths();
    let task_id = "explore-inference";
    paths.ensure_task_layout(task_id).unwrap();
    fs::write(paths.task_path(task_id, "dom/explore.html"), r#"<nav><a href="/catalog">完结</a><a href="/fiction">玄幻</a><a href="https://external.test/other">奇幻</a></nav><div class="item"><h3><a href="/one">都市小说书名</a></h3></div><a href="/catalog/2" rel="next">Next</a>"#).unwrap();
    let sampled = SamplingResult {
        base_url: "https://books.test".into(),
        pages: vec![crate::domain::PageSample {
            role: "explore".into(),
            url: "https://books.test/catalog".into(),
            html_path: "dom/explore.html".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let candidate = infer_candidate_from_sampled(&paths, task_id, &sampled).unwrap();
    let entries: Value =
        serde_json::from_str(candidate.book_source.explore_url.as_ref().unwrap()).unwrap();
    assert_eq!(entries.as_array().unwrap().len(), 2);
    assert_eq!(entries[0]["title"], "完结");
    assert_eq!(
        explore::page_url(entries[1]["url"].as_str().unwrap(), &sampled.base_url, 2).as_deref(),
        Some("https://books.test/fiction/2")
    );
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn current_reader_optional_rule_fields_are_preserved() {
    let mut candidate = good_candidate();
    candidate.book_source.rule_content.as_mut().unwrap().title = Some("h1@text".into());
    candidate.book_source.rule_toc.as_mut().unwrap().update_time = Some(".date@text".into());
    sanitize_candidate(&mut candidate).unwrap();
    let exported = exportable_book_source_value(&candidate.book_source).unwrap();
    assert_eq!(
        exported
            .pointer("/ruleContent/title")
            .and_then(Value::as_str),
        Some("h1@text")
    );
    assert_eq!(
        exported
            .pointer("/ruleToc/updateTime")
            .and_then(Value::as_str),
        Some(".date@text")
    );
    let paths = diagnostic_test_paths();
    assert!(validate_candidate(
        &paths,
        "optional-fields",
        &SamplingResult::default(),
        &candidate,
        None
    )
    .is_ok());
    fs::remove_dir_all(paths.data_dir).unwrap();
}

#[test]
fn task_roundtrip_payload_is_stable() {
    let task = TaskRecord {
        id: "task-1".into(),
        status: TaskStatus::Draft,
        task_mode: TaskMode::Classic,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        target_url: "fixture://novel-world".into(),
        test_keyword: Some("星门".into()),
        error_summary: None,
        artifact_manifest: Some(ArtifactManifest::default()),
        source_candidate: Some(good_candidate()),
        validation_report: None,
        logs: vec![],
    };

    let raw = serde_json::to_string(&task).unwrap();
    let decoded: TaskRecord = serde_json::from_str(&raw).unwrap();
    assert_eq!(decoded.id, "task-1");
    assert_eq!(decoded.target_url, "fixture://novel-world");
    assert_eq!(decoded.test_keyword.as_deref(), Some("星门"));
}

#[test]
fn parses_mock_openai_response_payload() {
    let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": "{\"strategy\":\"mock\",\"confidence\":0.91,\"evidenceRefs\":[],\"repairHistory\":[],\"bookSource\":{\"bookSourceName\":\"Fixture\",\"bookSourceUrl\":\"https://fixture.novel.world\"}}"
                    }
                }
            ]
        }))
        .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(parsed.strategy, "mock");
    assert_eq!(parsed.book_source.book_source_name, "Fixture");
}

#[test]
fn parses_string_evidence_refs_from_model_output() {
    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": r#"{
                          "strategy":"live-model",
                          "confidence":0.74,
                          "evidenceRefs":[
                            "Base URL: https://biquge.xin",
                            "Search entry appears in page header"
                          ],
                          "repairHistory":["first draft"],
                          "bookSource":{
                            "bookSourceName":"Biquge",
                            "bookSourceUrl":"https://biquge.xin"
                          }
                        }"#
                }
            }
        ]
    }))
    .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(parsed.evidence_refs.len(), 2);
    assert_eq!(
        parsed.evidence_refs[0].label,
        "Base URL: https://biquge.xin"
    );
    assert_eq!(parsed.repair_history.len(), 1);
    assert_eq!(parsed.repair_history[0].reason, "first draft");
}

#[test]
fn parses_string_confidence_levels_from_model_output() {
    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": r#"{
                          "strategy":"live-model",
                          "confidence":"medium",
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "bookSourceName":"Cuoceng",
                            "bookSourceUrl":"https://m.cuoceng.com"
                          }
                        }"#
                }
            }
        ]
    }))
    .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(parsed.confidence, 0.65);
}

#[test]
fn parses_numeric_confidence_strings_from_model_output() {
    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": r#"{
                          "strategy":"live-model",
                          "confidence":"0.82",
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "bookSourceName":"Fixture",
                            "bookSourceUrl":"https://fixture.novel.world"
                          }
                        }"#
                }
            }
        ]
    }))
    .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(parsed.confidence, 0.82);
}

#[test]
fn parses_string_book_source_type_from_model_output() {
    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": r#"{
                          "strategy":"live-model",
                          "confidence":"0.72",
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "bookSourceName":"Cuoceng",
                            "bookSourceUrl":"https://m.cuoceng.com",
                            "bookSourceType":"novel",
                            "enabledExplore":"true"
                          }
                        }"#
                }
            }
        ]
    }))
    .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(parsed.book_source.book_source_type, Some(0));
    assert_eq!(parsed.book_source.enabled_explore, Some(true));
}

#[test]
fn parses_stringified_rule_book_info_object_from_model_output() {
    let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": r#"{
                          "strategy":"live-model",
                          "confidence":"0.72",
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "bookSourceName":"Cuoceng",
                            "bookSourceUrl":"https://m.cuoceng.com",
                            "ruleBookInfo":"{\"infoElement\":\".book-info\",\"titleElement\":\".book-title,h1\",\"authorElement\":\".author\",\"introElement\":\".intro,.desc\",\"coverUrlElement\":\".cover img@src\",\"tocUrlElement\":\"a[href*=\\\"/chapter/\\\"]@href\"}"
                          }
                        }"#
                    }
                }
            ]
        }))
        .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    let rule = parsed
        .book_source
        .rule_book_info
        .expect("ruleBookInfo should be parsed");
    assert_eq!(rule.name.as_deref(), Some(".book-title,h1"));
    assert_eq!(rule.author.as_deref(), Some(".author"));
    assert_eq!(rule.intro.as_deref(), Some(".intro,.desc"));
    assert_eq!(rule.cover_url.as_deref(), Some(".cover img@src"));
    assert_eq!(rule.toc_url.as_deref(), Some("a[href*=\"/chapter/\"]@href"));
}

#[test]
fn normalizes_multi_extractor_rule_expression() {
    let body: OpenAiResponse = serde_json::from_value(json!({
            "choices": [
                {
                    "message": {
                        "content": r#"{
                          "strategy":"live-model",
                          "confidence":"0.81",
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "bookSourceName":"m.22biqu.com",
                            "bookSourceUrl":"https://m.22biqu.com",
                            "ruleBookInfo":{"tocUrl":"a.chapter-list@href, .toc-link@href, a[href*=\"index\"]@href"}
                          }
                        }"#
                    }
                }
            ]
        }))
        .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(
        parsed
            .book_source
            .rule_book_info
            .as_ref()
            .and_then(|rule| rule.toc_url.as_deref()),
        Some("a.chapter-list@href")
    );
}

#[test]
fn parses_and_normalizes_model_rule_syntax() {
    let content = r##"{
  "strategy": "live-model",
  "confidence": 0.88,
  "evidenceRefs": [],
  "repairHistory": [],
  "bookSource": {
    "bookSourceName": "Cuoceng",
    "bookSourceUrl": "https://m.cuoceng.com",
    "ruleSearch": {
      "bookList": ".bookbox",
      "name": ".bookname a::text",
      "bookUrl": ".bookname a::attr(href)"
    },
    "ruleContent": {
      "content": "#content::textNodes",
      "nextContentUrl": ".bottem1 a:contains('下一页')::attr(href)"
    }
  }
}"##;
    let payload = r#"{
              "choices": [
                {
                  "message": {
                    "content": "__REPLACE__"
                  }
                }
              ]
            }"#
    .replace(
        "__REPLACE__",
        &content
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n"),
    );
    let body: OpenAiResponse = serde_json::from_str(&payload).unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult::default(),
        &body,
    )
    .unwrap();

    assert_eq!(
        parsed
            .book_source
            .rule_search
            .as_ref()
            .and_then(|rule| rule.name.as_deref()),
        Some(".bookname a@text")
    );
    assert_eq!(
        parsed
            .book_source
            .rule_search
            .as_ref()
            .and_then(|rule| rule.book_url.as_deref()),
        Some(".bookname a@href")
    );
    assert_eq!(
        parsed
            .book_source
            .rule_content
            .as_ref()
            .and_then(|rule| rule.content.as_deref()),
        Some("#content@textNodes")
    );
    assert_eq!(
        parsed
            .book_source
            .rule_content
            .as_ref()
            .and_then(|rule| rule.next_content_url.as_deref()),
        Some("text.下一页@href")
    );
}

#[test]
fn fills_missing_book_source_fields_from_sampled_site() {
    let body: OpenAiResponse = serde_json::from_value(json!({
        "choices": [
            {
                "message": {
                    "content": r#"{
                          "strategy":"live-model",
                          "confidence":0.74,
                          "evidenceRefs":[],
                          "repairHistory":[],
                          "bookSource":{
                            "search_url":"https://biquge.xin/search?q={key}&page={page}"
                          }
                        }"#
                }
            }
        ]
    }))
    .unwrap();

    let parsed = parse_candidate_from_openai(
        &AppPaths {
            data_dir: PathBuf::from("."),
            db_path: PathBuf::from("./db"),
            tasks_dir: PathBuf::from("./tasks"),
            logs_dir: PathBuf::from("./logs"),
            settings_path: PathBuf::from("./settings.json"),
            fixtures_dir: PathBuf::from("./fixtures"),
            worker_script: PathBuf::from("./worker.mjs"),
        },
        &SamplingResult {
            fixture_name: None,
            site_label: "biquge.xin".into(),
            base_url: "https://biquge.xin".into(),
            pages: vec![],
            network_samples: vec![],
            manifest: ArtifactManifest::default(),
        },
        &body,
    )
    .unwrap();

    assert_eq!(parsed.book_source.book_source_name, "biquge.xin");
    assert_eq!(parsed.book_source.book_source_url, "https://biquge.xin");
    assert_eq!(
        parsed.book_source.search_url.as_deref(),
        Some("https://biquge.xin/search?q={key}&page={page}")
    );
}

#[test]
fn keeps_full_openai_endpoint_if_already_provided() {
    let endpoint =
        normalize_openai_endpoint("https://integrate.api.nvidia.com/v1/chat/completions");
    assert_eq!(
        endpoint,
        "https://integrate.api.nvidia.com/v1/chat/completions"
    );
}

#[test]
fn appends_openai_endpoint_when_base_url_is_root() {
    let endpoint = normalize_openai_endpoint("https://api.openai.com/v1");
    assert_eq!(endpoint, "https://api.openai.com/v1/chat/completions");
}

#[test]
fn ai_settings_backward_compatible_with_old_flag_field() {
    let parsed: AiSettings = serde_json::from_str(
        r#"{"baseUrl":"mock://fixture","model":"mock-fixture","apiKeyPresent":true}"#,
    )
    .unwrap();
    assert!(parsed.api_key.is_empty());
}

#[test]
fn inferred_rules_handle_biquge_style_templates() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-biquge";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();

    fs::write(
        task_root.join("dom/search.html"),
        r#"
            <html><body>
              <ul class="librarylist">
                <li>
                  <a class="novelname" href="/book/9058/">剑来</a>
                  <div class="author">烽火戏诸侯</div>
                  <div class="intro">大千世界，无奇不有。</div>
                </li>
              </ul>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/search-mobile.html"),
        r#"
            <html><body>
              <div class="read_book">
                <div class="bookbox">
                  <div class="bookinfo">
                    <h4 class="bookname"><a href="/biqu42106/">星门</a></h4>
                    <div class="author">作者：老鹰吃小鸡</div>
                  </div>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"
            <html><body>
              <h1>剑来</h1>
              <ul class="novelinfo-l"><li><a href="/author/1">烽火戏诸侯</a></li></ul>
              <div class="novelintro">这是一本长篇小说简介。</div>
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/9058/1.html">第一章 小镇</a></li>
                  <li><a href="/book/9058/2.html">第二章 少年</a></li>
                </ul>
              </div>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/toc.html"),
        r#"
            <html><body>
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/9058/1.html">第一章 小镇</a></li>
                  <li><a href="/book/9058/2.html">第二章 少年</a></li>
                </ul>
              </div>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/content.html"),
        r#"
            <html><body>
              <div id="content">
                这里是第一段正文。
                这里是第二段正文。
                这里是第三段正文。
                这里是第四段正文。
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };

    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "biquge.xin".into(),
        base_url: "https://biquge.xin".into(),
        pages: vec![
            crate::domain::PageSample {
                role: "search".into(),
                url: "https://biquge.xin/search/result.html?searchkey=%E5%89%91%E6%9D%A5&page=1"
                    .into(),
                html_path: "dom/search.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "book".into(),
                url: "https://biquge.xin/book/9058/".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "toc".into(),
                url: "https://biquge.xin/book/9058/".into(),
                html_path: "dom/toc.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "content".into(),
                url: "https://biquge.xin/book/9058/1.html".into(),
                html_path: "dom/content.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
        ],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let candidate = infer_candidate_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(
        candidate.book_source.search_url.as_deref(),
        Some("https://biquge.xin/search/result.html?searchkey={key}&page={page}")
    );
    assert!(candidate.book_source.explore_url.is_none());
    assert_eq!(
        candidate
            .book_source
            .rule_search
            .as_ref()
            .and_then(|rule| rule.book_list.as_deref()),
        Some(".librarylist li")
    );
    assert_eq!(
        candidate
            .book_source
            .rule_book_info
            .as_ref()
            .and_then(|rule| rule.intro.as_deref()),
        Some(".novelintro@text")
    );
    assert_eq!(
        candidate
            .book_source
            .rule_toc
            .as_ref()
            .and_then(|rule| rule.chapter_list.as_deref()),
        Some(".fulldir .dirlist li")
    );
    assert_eq!(
        candidate
            .book_source
            .rule_content
            .as_ref()
            .and_then(|rule| rule.content.as_deref()),
        Some("#content@textNodes")
    );

    let report = validate_candidate(&paths, task_id, &sampled, &candidate, Some("剑来")).unwrap();
    assert!(!report.explore.passed);
    assert!(report.search.passed);
    assert!(report.book_info.passed);
    assert!(report.toc.passed);
    assert!(report.content.passed);
    assert!(report.cleanup.passed);
    assert!(report_is_usable(&report));
    assert!(!report_requires_repair(&report, &candidate.book_source));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn infer_search_url_prefers_sampled_path_style_result() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-cuoceng-search";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();

    fs::write(
            task_root.join("dom/landing.html"),
            r#"
            <html><body>
              <form name="articlesearch" method="get" action="/book/so" onsubmit="return handleSearch(this,null)">
                <input name="searchkey" type="text" />
                <button type="submit">搜索</button>
              </form>
            </body></html>
            "#,
        )
        .unwrap();
    fs::write(
        task_root.join("dom/search.html"),
        r#"
            <html><body>
              <div class="bookbox">
                <div class="bookinfo">
                  <h4 class="bookname"><a href="/book/abc.html">星门</a></h4>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.cuoceng.com".into(),
        base_url: "https://m.cuoceng.com".into(),
        pages: vec![
            crate::domain::PageSample {
                role: "landing".into(),
                url: "https://m.cuoceng.com/".into(),
                html_path: "dom/landing.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "search".into(),
                url: "https://m.cuoceng.com/book/so/%E6%98%9F%E9%97%A8.html".into(),
                html_path: "dom/search.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
        ],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let search_url = infer_search_url_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(
        search_url.as_deref(),
        Some("https://m.cuoceng.com/book/so/{key}.html")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inferred_search_rule_handles_mobile_biquge_cards() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-biquge-mobile";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/search.html"),
        r#"
            <html><body>
              <div class="read_book">
                <div class="bookbox">
                  <div class="bookinfo">
                    <h4 class="bookname"><a href="/biqu42106/">星门</a></h4>
                    <div class="author">作者：老鹰吃小鸡</div>
                  </div>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.22biqu.net".into(),
        base_url: "https://m.22biqu.net".into(),
        pages: vec![crate::domain::PageSample {
            role: "search".into(),
            url: "https://m.22biqu.net/ss/?searchkey=%E6%98%9F%E9%97%A8&page=1".into(),
            html_path: "dom/search.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let rule = infer_search_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(rule.book_list.as_deref(), Some(".bookbox"));
    assert_eq!(rule.name.as_deref(), Some(".bookname a@text"));
    assert_eq!(rule.book_url.as_deref(), Some(".bookname a@href"));
    assert_eq!(rule.cover_url.as_deref(), None);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn inferred_book_info_handles_mobile_biquge_detail_page() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-biquge-detail";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"
            <html><body>
              <header class="channelHeader"><span class="title">星门</span></header>
              <div class="synopsisArea_detail">
                <b class="author">作者：老鹰吃小鸡</b>
              </div>
              <div class="review">这是一本简介。</div>
              <div class="btn"><a href="/biqu42106/123.html">开始阅读</a></div>
              <div id="chapterlist"><p><a href="/biqu42106/456.html">最新章节</a></p></div>
              <div class="directoryArea"><p><a href="/biqu42106/123.html">第一章</a></p></div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.22biqu.net".into(),
        base_url: "https://m.22biqu.net".into(),
        pages: vec![crate::domain::PageSample {
            role: "book".into(),
            url: "https://m.22biqu.net/biqu42106/".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let book_rule = infer_book_info_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(
        book_rule.name.as_deref(),
        Some(".channelHeader .title@text")
    );
    assert_eq!(
        book_rule.author.as_deref(),
        Some(".synopsisArea_detail .author@text")
    );
    assert_eq!(book_rule.intro.as_deref(), Some(".review@text"));
    assert_eq!(book_rule.cover_url.as_deref(), None);
    assert_eq!(book_rule.toc_url.as_deref(), None);

    let toc_rule = infer_toc_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(toc_rule.chapter_list.as_deref(), Some(".directoryArea p"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inferred_toc_rule_prefers_toc_page_and_allchapter_dd() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-cuoceng-toc";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"
            <html><body>
              <div class="book chapterlist">
                <div id="allchapter">
                  <dl>
                    <dd><a href="/book/chapter/example.html">查看全部章节 >></a></dd>
                  </dl>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/toc.html"),
        r#"
            <html><body>
              <div class="book chapterlist">
                <div id="allchapter">
                  <dl>
                    <dd data-num="1"><a href="/book/example/1.html">第一章</a></dd>
                    <dd data-num="2"><a href="/book/example/2.html">第二章</a></dd>
                  </dl>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.cuoceng.com".into(),
        base_url: "https://m.cuoceng.com".into(),
        pages: vec![
            crate::domain::PageSample {
                role: "book".into(),
                url: "https://m.cuoceng.com/book/example.html".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "toc".into(),
                url: "https://m.cuoceng.com/book/chapter/example.html".into(),
                html_path: "dom/toc.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
        ],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let toc_rule = infer_toc_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(
        toc_rule.chapter_list.as_deref(),
        Some(".book.chapterlist #allchapter dd")
    );
    assert_eq!(toc_rule.chapter_name.as_deref(), Some("a@text"));
    assert_eq!(toc_rule.chapter_url.as_deref(), Some("a@href"));
    assert_eq!(toc_rule.next_toc_url.as_deref(), None);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inferred_toc_rule_detects_select_based_pagination() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-toc-next-page";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/toc.html"),
        r#"
            <html><body>
              <div class="directoryArea">
                <p><a href="/book/example/1.html">第一章</a></p>
                <p><a href="/book/example/2.html">第二章</a></p>
              </div>
              <div class="index-container">
                <select id="indexselect">
                  <option value="https://m.22biqu.com/biqu42106/" selected="selected">1-50</option>
                  <option value="https://m.22biqu.com/biqu42106/2/">51-100</option>
                </select>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.22biqu.com".into(),
        base_url: "https://m.22biqu.com".into(),
        pages: vec![crate::domain::PageSample {
            role: "toc".into(),
            url: "https://m.22biqu.com/biqu42106/".into(),
            html_path: "dom/toc.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let toc_rule = infer_toc_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(toc_rule.chapter_list.as_deref(), Some(".directoryArea p"));
    assert_eq!(
        toc_rule.next_toc_url.as_deref(),
        Some("#indexselect option@value")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn validate_toc_requires_next_toc_when_page_has_pagination() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-toc-pagination-required";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/toc.html"),
        r#"
            <html><body>
              <div class="directoryArea">
                <p><a href="/book/example/1.html">第一章</a></p>
                <p><a href="/book/example/2.html">第二章</a></p>
              </div>
              <div class="index-container">
                <select id="indexselect">
                  <option value="https://m.22biqu.com/biqu42106/" selected="selected">1-50</option>
                  <option value="https://m.22biqu.com/biqu42106/2/">51-100</option>
                </select>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.22biqu.com".into(),
        base_url: "https://m.22biqu.com".into(),
        pages: vec![crate::domain::PageSample {
            role: "toc".into(),
            url: "https://m.22biqu.com/biqu42106/".into(),
            html_path: "dom/toc.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };
    let candidate = SourceCandidate {
        strategy: "test".into(),
        confidence: 1.0,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "m.22biqu.com".into(),
            book_source_url: "https://m.22biqu.com".into(),
            rule_toc: Some(TocRule {
                chapter_list: Some(".directoryArea p".into()),
                chapter_name: Some("a@text".into()),
                chapter_url: Some("a@href".into()),
                next_toc_url: None,
                ..Default::default()
            }),
            ..Default::default()
        },
    };

    let report = validate_candidate(&paths, task_id, &sampled, &candidate, Some("星门")).unwrap();
    assert!(!report.toc.passed);
    assert_eq!(report.toc.summary, "目录分页规则缺失");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn inferred_content_rule_handles_mobile_reader_page() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-biquge-reader";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/content.html"),
        r#"
            <html><body>
              <div id="chaptercontent">
                <p>第一段正文。</p>
                <p>第二段正文。</p>
                <p>第三段正文。</p>
              </div>
              <p class="Readpage"><a id="pt_next" href="/biqu42106/123_2.html">下一页</a></p>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.22biqu.net".into(),
        base_url: "https://m.22biqu.net".into(),
        pages: vec![crate::domain::PageSample {
            role: "content".into(),
            url: "https://m.22biqu.net/biqu42106/123.html".into(),
            html_path: "dom/content.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let rule = infer_content_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(rule.content.as_deref(), Some("#chaptercontent@textNodes"));
    assert_eq!(rule.next_content_url.as_deref(), Some("#pt_next@href"));

    let candidate = SourceCandidate {
        strategy: "test".into(),
        confidence: 1.0,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "m.22biqu.net".into(),
            book_source_url: "https://m.22biqu.net".into(),
            enabled: Some(true),
            search_url: None,
            rule_search: None,
            rule_book_info: None,
            rule_toc: None,
            rule_content: Some(rule),
            ..Default::default()
        },
    };

    let report = validate_content(&paths, task_id, &sampled, &candidate).unwrap();
    assert!(report.passed);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn export_22biqu_mobile_template_uses_reader_compatible_rules() {
    let source = BookSource {
        book_source_name: "m.22biqu.net".into(),
        book_source_url: "https://m.22biqu.net".into(),
        enabled: Some(true),
        search_url: Some("https://m.22biqu.net/ss/?searchkey={key}&page={page}".into()),
        rule_search: Some(SearchRule {
            book_list: Some(".bookbox".into()),
            name: Some(".bookname a@text".into()),
            book_url: Some(".bookname a@href".into()),
            ..Default::default()
        }),
        rule_book_info: Some(BookInfoRule {
            name: Some(".channelHeader .title@text".into()),
            ..Default::default()
        }),
        rule_toc: Some(TocRule {
            chapter_list: Some(".directoryArea p".into()),
            ..Default::default()
        }),
        rule_content: Some(ContentRule {
            content: Some("#chaptercontent@textNodes".into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let exported = exportable_book_source_value(&source).unwrap();
    assert_eq!(
            exported.get("searchUrl").and_then(Value::as_str),
            Some("{{cookie.removeCookie(source.getKey())}}/ss/,\n{\n  \"body\": \"searchkey={{key}}\",\n  \"method\": \"POST\"\n}")
        );
    assert_eq!(
        exported.pointer("/ruleSearch/name").and_then(Value::as_str),
        Some(".bookname@a@text")
    );
    assert_eq!(
        exported
            .pointer("/ruleToc/chapterList")
            .and_then(Value::as_str),
        Some(".directoryArea p a")
    );
    assert_eq!(
        exported
            .pointer("/ruleContent/content")
            .and_then(Value::as_str),
        Some("#chaptercontent@html")
    );
    assert_eq!(
        exported.get("bookUrlPattern").and_then(Value::as_str),
        Some("https?://m\\.22biqu\\.net/biqu\\d+/")
    );
    assert_eq!(
        exported.get("enabledExplore").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        exported
            .pointer("/ruleExplore/bookList")
            .and_then(Value::as_str),
        Some(".hot_sale")
    );
    assert!(exported.get("exploreUrl").and_then(Value::as_str).is_some());
    assert_eq!(exported.pointer("/ruleContent/title"), Some(&Value::Null));
}

#[test]
fn export_value_normalizes_attr_syntax_for_reader() {
    let source = BookSource {
        book_source_name: "biquge.xin".into(),
        book_source_url: "https://biquge.xin".into(),
        enabled: Some(true),
        search_url: Some(
            "https://biquge.xin/search/result.html?searchkey={key}&page={page}".into(),
        ),
        rule_search: Some(SearchRule {
            book_list: Some(".librarylist li".into()),
            name: Some(".novelname@text".into()),
            author: Some(".info span:nth-child(2) a@text".into()),
            book_url: Some(".novelname@attr[href]".into()),
            cover_url: Some(".pt-ll-l img@attr[src]".into()),
            ..Default::default()
        }),
        rule_book_info: Some(BookInfoRule {
            name: Some("meta[property=\"og:novel:book_name\"]@attr[content]".into()),
            author: Some("meta[property=\"og:novel:author\"]@attr[content]".into()),
            cover_url: Some("meta[property=\"og:image\"]@attr[content]".into()),
            ..Default::default()
        }),
        rule_toc: Some(TocRule {
            chapter_list: Some(".fulldir .dirlist li".into()),
            chapter_name: Some("a@text".into()),
            chapter_url: Some("a@attr[href]".into()),
            ..Default::default()
        }),
        rule_content: Some(ContentRule {
            content: Some("#chaptercontent@textNodes".into()),
            next_content_url: Some("#pt_next@attr[href]".into()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let exported = exportable_book_source_value(&source).unwrap();
    assert_eq!(
        exported
            .pointer("/ruleSearch/bookUrl")
            .and_then(Value::as_str),
        Some(".novelname@href")
    );
    assert_eq!(
        exported
            .pointer("/ruleSearch/coverUrl")
            .and_then(Value::as_str),
        Some(".pt-ll-l img@src")
    );
    assert_eq!(
        exported
            .pointer("/ruleBookInfo/author")
            .and_then(Value::as_str),
        Some("meta[property=\"og:novel:author\"]@content")
    );
    assert_eq!(
        exported
            .pointer("/ruleBookInfo/coverUrl")
            .and_then(Value::as_str),
        Some("meta[property=\"og:image\"]@content")
    );
    assert_eq!(
        exported
            .pointer("/ruleToc/chapterUrl")
            .and_then(Value::as_str),
        Some("a@href")
    );
    assert_eq!(
        exported
            .pointer("/ruleContent/nextContentUrl")
            .and_then(Value::as_str),
        Some("#pt_next@href")
    );
}

#[test]
fn validate_content_supports_text_keyword_selector() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-text-keyword-selector";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/content.html"),
        r#"
            <html><body>
              <div id="content">正文第一页</div>
              <div class="bottem1"><a href="/book/1/2.html">下一页</a></div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.cuoceng.com".into(),
        base_url: "https://m.cuoceng.com".into(),
        pages: vec![crate::domain::PageSample {
            role: "content".into(),
            url: "https://m.cuoceng.com/book/1/1.html".into(),
            html_path: "dom/content.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };
    let candidate = SourceCandidate {
        strategy: "test".into(),
        confidence: 0.9,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "m.cuoceng.com".into(),
            book_source_url: "https://m.cuoceng.com".into(),
            enabled: Some(true),
            rule_content: Some(ContentRule {
                content: Some("#content@textNodes".into()),
                next_content_url: Some("text.下一页@href".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    };

    let phase = validate_content(&paths, task_id, &sampled, &candidate).unwrap();
    assert!(phase.passed);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn export_task_source_also_writes_to_selected_path() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };

    fs::create_dir_all(&paths.tasks_dir).unwrap();
    fs::create_dir_all(&paths.logs_dir).unwrap();
    init_database(&paths).unwrap();

    let mut task = TaskRecord::new(crate::domain::TaskDraft {
        target_url: "https://m.22biqu.net".into(),
        test_keyword: Some("星门".into()),
        task_mode: TaskMode::Classic,
    });
    task.source_candidate = Some(SourceCandidate {
        strategy: "test".into(),
        confidence: 1.0,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: good_candidate().book_source,
    });
    task.artifact_manifest = Some(ArtifactManifest::default());

    paths.ensure_task_layout(&task.id).unwrap();
    persist_task(&paths, &task).unwrap();

    let selected_path = root.join("exports").join("book-source.json");
    let exported = export_task_source(
        &paths,
        &task.id,
        Some(selected_path.to_string_lossy().as_ref()),
    )
    .unwrap();

    assert_eq!(fs::read_to_string(&selected_path).unwrap(), exported);
    assert!(paths
        .task_path(&task.id, "output/book-source.json")
        .exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_task_removes_database_row_and_task_directory() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };

    fs::create_dir_all(&paths.tasks_dir).unwrap();
    fs::create_dir_all(&paths.logs_dir).unwrap();
    init_database(&paths).unwrap();

    let task = create_task(
        &paths,
        crate::domain::TaskDraft {
            target_url: "fixture://novel-world".into(),
            test_keyword: Some("星门".into()),
            task_mode: TaskMode::Classic,
        },
    )
    .unwrap();
    fs::write(paths.task_path(&task.id, "output/temp.json"), "{}").unwrap();

    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::new(TaskRuntime::default()),
    };

    delete_task(&state, &task.id).unwrap();

    assert!(get_task(&paths, &task.id).is_err());
    assert!(!paths.task_dir(&task.id).exists());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_running_task_removes_list_entry_and_marks_pending_cleanup() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };

    fs::create_dir_all(&paths.tasks_dir).unwrap();
    fs::create_dir_all(&paths.logs_dir).unwrap();
    init_database(&paths).unwrap();

    let task = create_task(
        &paths,
        crate::domain::TaskDraft {
            target_url: "fixture://novel-world".into(),
            test_keyword: Some("星门".into()),
            task_mode: TaskMode::Classic,
        },
    )
    .unwrap();

    let runtime = Arc::new(TaskRuntime::default());
    assert!(runtime.register(&task.id));

    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::clone(&runtime),
    };

    delete_task(&state, &task.id).unwrap();

    assert!(get_task(&paths, &task.id).is_err());
    assert!(paths.task_dir(&task.id).exists());
    assert!(runtime.is_cancel_requested(&task.id));
    assert!(runtime.is_delete_requested(&task.id));

    runtime.unregister(&task.id);
    purge_task_storage(&paths, &task.id).unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_task_succeeds_when_only_task_directory_remains() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };

    fs::create_dir_all(&paths.tasks_dir).unwrap();
    fs::create_dir_all(&paths.logs_dir).unwrap();
    init_database(&paths).unwrap();

    let task_id = "stale-task";
    paths.ensure_task_layout(task_id).unwrap();
    fs::write(paths.task_path(task_id, "output/temp.json"), "{}").unwrap();

    let state = AppState {
        paths: paths.clone(),
        runtime: Arc::new(TaskRuntime::default()),
    };

    delete_task(&state, task_id).unwrap();

    assert!(!paths.task_dir(task_id).exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn report_is_usable_when_only_one_discovery_path_passes() {
    let mut report = ValidationReport {
        explore: passed_phase("explore ok".into(), vec![], vec![]),
        search: failed_phase("search missing", vec![], vec![]),
        book_info: passed_phase("book ok".into(), vec![], vec![]),
        toc: passed_phase("toc ok".into(), vec![], vec![]),
        content: passed_phase("content ok".into(), vec![], vec![]),
        cleanup: passed_phase("cleanup ok".into(), vec![], vec![]),
    };

    assert!(report_is_usable(&report));
    let mut source = good_candidate().book_source;
    assert!(!report_requires_repair(&report, &source));
    report.explore = failed_phase("pagination failed", vec![], vec![]);
    report.search = passed_phase("search ok".into(), vec![], vec![]);
    source.explore_url = Some("All::https://books.test/catalog".into());
    assert!(report_is_usable(&report));
    assert!(report_requires_repair(&report, &source));
    source.enabled_explore = Some(false);
    assert!(!report_requires_repair(&report, &source));
    report.search = failed_phase("search failed", vec![], vec![]);
    assert!(!report_is_usable(&report));
    assert!(report_requires_repair(&report, &source));
}

#[test]
fn site_scope_accepts_related_hosts_and_rejects_foreign_destinations() {
    let target = "https://www.books.example/";
    for url in [
        "https://www.books.example/novel",
        "https://books.example/novel",
        "https://m.books.example/novel",
        "http://books.example/novel",
    ] {
        assert!(is_site_url(target, url), "{url}");
    }
    for url in [
        "https://search.example/",
        "https://books.example.evil.test/",
        "https://notbooks.example/",
        "https://books.example@search.example/",
        "javascript:history.back()",
    ] {
        assert!(!is_site_url(target, url), "{url}");
    }
    assert!(is_site_url(
        "http://127.0.0.1:8123/",
        "http://127.0.0.1:8123/book"
    ));
    assert!(!is_site_url(
        "http://127.0.0.1:8123/",
        "http://127.0.0.1:8124/book"
    ));
    assert!(!is_site_url(
        "http://127.0.0.1:8123/",
        "http://127.0.0.2:8123/book"
    ));
}

#[test]
fn rejects_foreign_observed_links_for_reading_roles() {
    let target = "https://books.example/";
    let inspections = vec![PageInspection {
        page: crate::domain::PageSample {
            url: target.into(),
            ..Default::default()
        },
        links: vec![crate::domain::PageLink {
            label: "图片".into(),
            url: "https://search.example/images".into(),
        }],
        ..Default::default()
    }];
    let observed = build_observed_action_inputs(&inspections);
    for role in ["search", "explore", "book", "toc", "content"] {
        let action = AgentAction {
            kind: "open".into(),
            url: "https://search.example/images".into(),
            role: role.into(),
            reason: None,
            form_action: None,
            form_method: None,
            query_param: None,
            wait_for_selector: None,
        };
        assert!(validate_agent_action(&action, &observed, target)
            .unwrap()
            .contains("outside the target site"));
    }
}

#[test]
fn search_requires_an_explicit_observed_internal_form() {
    let target = "https://books.example/";
    let inspections = vec![PageInspection {
        page: crate::domain::PageSample {
            url: target.into(),
            ..Default::default()
        },
        forms: vec![
            crate::domain::PageForm {
                action: "https://search.example/search".into(),
                method: "get".into(),
                input_names: vec!["q".into()],
            },
            crate::domain::PageForm {
                action: "https://books.example/search".into(),
                method: "get".into(),
                input_names: vec!["q".into()],
            },
        ],
        ..Default::default()
    }];
    let observed = build_observed_action_inputs(&inspections);
    let mut action = AgentAction {
        kind: "search".into(),
        url: target.into(),
        role: "search".into(),
        reason: None,
        form_action: Some("https://search.example/search".into()),
        form_method: Some("get".into()),
        query_param: Some("q".into()),
        wait_for_selector: None,
    };
    assert!(validate_agent_action(&action, &observed, target)
        .unwrap()
        .contains("outside the target site"));
    action.form_action = None;
    assert!(validate_agent_action(&action, &observed, target)
        .unwrap()
        .contains("requires an observed formAction"));
    action.form_action = Some("https://books.example/search".into());
    assert_eq!(validate_agent_action(&action, &observed, target), None);
    action.query_param = Some("guessed".into());
    assert!(validate_agent_action(&action, &observed, target).is_some());
}

#[test]
fn cached_foreign_pages_do_not_complete_coverage_but_keep_artifacts() {
    let paths = diagnostic_test_paths();
    let task = TaskRecord::new(crate::domain::TaskDraft {
        target_url: "https://books.example/".into(),
        test_keyword: None,
        task_mode: TaskMode::Agent,
    });
    paths.ensure_task_layout(&task.id).unwrap();
    let mut worker_log = String::new();
    for role in ["landing", "explore", "search", "book", "toc", "content"] {
        fs::write(
            paths.task_path(&task.id, &format!("dom/{role}.html")),
            "<html><body>snapshot</body></html>",
        )
        .unwrap();
        fs::write(
            paths.task_path(&task.id, &format!("screenshots/{role}.png")),
            [],
        )
        .unwrap();
        let url = if matches!(role, "landing" | "explore") {
            "https://books.example/"
        } else {
            "https://search.example/"
        };
        worker_log.push_str(&format!("capture role={role} url={url}\n"));
    }
    fs::write(paths.task_path(&task.id, "network/external.json"), "{}").unwrap();
    worker_log
        .push_str("network-sample label=external method=GET url=https://search.example/api\n");
    fs::write(paths.task_path(&task.id, "logs/worker.log"), worker_log).unwrap();
    let sampled = load_sampled_from_task(&paths, &task).unwrap().unwrap();
    assert_eq!(sampled.pages.len(), 2);
    assert!(!agent_has_minimum_coverage(&sampled.pages));
    assert!(sampled.network_samples.is_empty());
    assert_eq!(sampled.manifest.dom.len(), 6);
    assert_eq!(sampled.manifest.screenshots.len(), 6);
    assert_eq!(sampled.manifest.network, vec!["network/external.json"]);
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn uncaptured_files_remain_diagnostic_evidence_without_assumed_urls() {
    let paths = diagnostic_test_paths();
    let task = TaskRecord::new(crate::domain::TaskDraft {
        target_url: "https://books.example/".into(),
        test_keyword: None,
        task_mode: TaskMode::Agent,
    });
    paths.ensure_task_layout(&task.id).unwrap();
    fs::write(
        paths.task_path(&task.id, "dom/book.html"),
        "<html><body>外站或未完成的页面</body></html>",
    )
    .unwrap();
    fs::write(paths.task_path(&task.id, "network/unlogged.json"), "{}").unwrap();
    let sampled = load_sampled_from_task(&paths, &task).unwrap().unwrap();
    assert!(sampled.pages.is_empty());
    assert!(sampled.network_samples.is_empty());
    assert_eq!(sampled.manifest.dom, vec!["dom/book.html"]);
    assert_eq!(sampled.manifest.network, vec!["network/unlogged.json"]);
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn navigation_links_do_not_become_reading_pages() {
    let paths = diagnostic_test_paths();
    let task_id = "navigation";
    paths.ensure_task_layout(task_id).unwrap();
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            url: "https://books.example/".into(),
            html_path: "dom/explore.html".into(),
            ..Default::default()
        },
        ..Default::default()
    };
    let navigation = (0..120)
        .map(|index| format!("<a href='/nav/{index}'>首页 图片 隐私</a>"))
        .collect::<String>();
    fs::write(paths.task_path(task_id, "dom/explore.html"), &navigation).unwrap();
    assert_eq!(
        infer_followup_book_url_from_dom(&paths, task_id, &inspection, Some("星门")),
        None
    );
    assert_eq!(
        infer_followup_toc_url_from_dom(&paths, task_id, &inspection),
        None
    );
    assert_eq!(
        infer_followup_content_url_from_dom(&paths, task_id, &inspection),
        None
    );

    fs::write(
        paths.task_path(task_id, "dom/explore.html"),
        format!("{navigation}<h3><a href='/novels/42'>星门</a></h3>"),
    )
    .unwrap();
    assert_eq!(
        infer_followup_book_url_from_dom(&paths, task_id, &inspection, Some("星门")).as_deref(),
        Some("https://books.example/novels/42")
    );
    fs::write(
        paths.task_path(task_id, "dom/explore.html"),
        format!("{navigation}<a href='/novels/42/list'>章节目录</a>"),
    )
    .unwrap();
    assert_eq!(
        infer_followup_toc_url_from_dom(&paths, task_id, &inspection).as_deref(),
        Some("https://books.example/novels/42/list")
    );
    fs::write(
        paths.task_path(task_id, "dom/explore.html"),
        format!("{navigation}<a href='/novels/42/read/1'>第一章 初始</a>"),
    )
    .unwrap();
    assert_eq!(
        infer_followup_content_url_from_dom(&paths, task_id, &inspection).as_deref(),
        Some("https://books.example/novels/42/read/1")
    );
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn foreign_redirect_keeps_diagnostics_without_marking_role_sampled() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "book".into(),
            url: "https://search.example/".into(),
            ..Default::default()
        },
        network_samples: vec![crate::domain::NetworkSample {
            request_url: "https://search.example/api".into(),
            response_path: "network/redirect.json".into(),
            ..Default::default()
        }],
        manifest: ArtifactManifest {
            screenshots: vec!["screenshots/book.png".into()],
            network: vec!["network/redirect.json".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let (mut pages, mut network, mut manifest, mut inspections) = (
        Vec::new(),
        Vec::new(),
        ArtifactManifest::default(),
        Vec::new(),
    );
    assert!(!merge_inspection(
        "https://books.example/",
        &mut pages,
        &mut network,
        &mut manifest,
        &mut inspections,
        inspection
    ));
    assert!(pages.is_empty());
    assert!(inspections.is_empty());
    assert!(network.is_empty());
    assert_eq!(manifest.screenshots, vec!["screenshots/book.png"]);
    assert_eq!(manifest.network, vec!["network/redirect.json"]);
}

#[test]
fn inline_chapter_list_is_toc_instead_of_a_chapter_title_containing_chapters() {
    let paths = diagnostic_test_paths();
    let task_id = "inline-toc";
    paths.ensure_task_layout(task_id).unwrap();
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "book".into(),
            url: "https://books.example/42".into(),
            html_path: "dom/book.html".into(),
            ..Default::default()
        },
        links: vec![crate::domain::PageLink {
            label: "年终总结（章节已更）".into(),
            url: "https://books.example/42/note".into(),
        }],
        ..Default::default()
    };
    let html = "<h1>星门</h1><div id='list'><ul><li><a href='/42/note'>年终总结（章节已更）</a></li><li><a href='/42/1'>第一章</a></li><li><a href='/42/2'>第二章</a></li><li><a href='/42/3'>第三章</a></li></ul></div>";
    fs::write(paths.task_path(task_id, "dom/book.html"), html).unwrap();
    assert_eq!(infer_followup_toc_url(&inspection), None);
    assert_eq!(
        infer_followup_toc_url_from_dom(&paths, task_id, &inspection).as_deref(),
        Some("https://books.example/42")
    );
    let sampled = SamplingResult {
        pages: vec![inspection.page.clone()],
        ..Default::default()
    };
    assert_eq!(
        infer_toc_rule_from_sampled(&paths, task_id, &sampled)
            .unwrap()
            .chapter_list
            .as_deref(),
        Some("#list li")
    );
    fs::write(
        paths.task_path(task_id, "dom/book.html"),
        format!("{html}<a href='/toc/42'>查看全部章节</a>"),
    )
    .unwrap();
    assert_eq!(
        infer_followup_toc_url_from_dom(&paths, task_id, &inspection).as_deref(),
        Some("https://books.example/toc/42")
    );
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn repeated_captures_replace_the_url_for_the_same_artifact() {
    let (mut pages, mut network, mut manifest, mut inspections) = (
        Vec::new(),
        Vec::new(),
        ArtifactManifest::default(),
        Vec::new(),
    );
    for url in [
        "https://books.example/chapter/1",
        "https://books.example/chapter/2",
        "https://search.example/",
    ] {
        let inspection = PageInspection {
            page: crate::domain::PageSample {
                role: "content".into(),
                url: url.into(),
                html_path: "dom/content.html".into(),
                ..Default::default()
            },
            network_samples: vec![crate::domain::NetworkSample {
                request_url: url.into(),
                response_path: "network/content-1.html".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let accepted = merge_inspection(
            "https://books.example/",
            &mut pages,
            &mut network,
            &mut manifest,
            &mut inspections,
            inspection,
        );
        if url.starts_with("https://books.example/") {
            assert!(accepted);
            assert_eq!(pages.len(), 1);
            assert_eq!(pages[0].url, url);
            assert_eq!(inspections.len(), 1);
            assert_eq!(inspections[0].page.url, url);
            assert_eq!(network.len(), 1);
            assert_eq!(network[0].request_url, url);
        } else {
            assert!(!accepted);
            assert!(pages.is_empty());
            assert!(inspections.is_empty());
            assert!(network.is_empty());
        }
    }
}

#[test]
fn agent_planner_receives_dom_and_search_failure_context() {
    let paths = diagnostic_test_paths();
    let task_id = "plan-evidence";
    paths.ensure_task_layout(task_id).unwrap();
    fs::write(
        paths.task_path(task_id, "dom/landing.html"),
        "<html><body><h3><a href='/novels/42'>星门</a></h3></body></html>",
    )
    .unwrap();
    fs::write(
        paths.task_path(task_id, "logs/task.log"),
        "站内搜索不可用，继续通过 explore 采样\n",
    )
    .unwrap();
    let inspections = vec![PageInspection {
        page: crate::domain::PageSample {
            role: "landing".into(),
            url: "https://books.example/".into(),
            html_path: "dom/landing.html".into(),
            ..Default::default()
        },
        ..Default::default()
    }];
    let body = json!({"choices":[{"message":{"content":"{\"completed\":false,\"actions\":[]}"}}]})
        .to_string();
    let (base_url, server) = mock_model_server(200, body);
    let plan = tauri::async_runtime::block_on(request_agent_plan(
        &paths,
        task_id,
        &StartAiSettings {
            base_url,
            model: "test-model".into(),
        },
        "test-key",
        "https://books.example/",
        Some("星门"),
        &inspections,
    ))
    .unwrap();
    assert!(!plan.completed);
    let request = server.join().unwrap();
    let body_start = request
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap()
        + 4;
    let payload: Value = serde_json::from_slice(&request[body_start..]).unwrap();
    let prompt = payload["messages"][1]["content"].as_str().unwrap();
    assert!(prompt.contains("<h3><a href=\"/novels/42\">星门</a></h3>"));
    assert!(prompt.contains("站内搜索不可用，继续通过 explore 采样"));
    assert!(prompt.contains("target site"));
    fs::remove_dir_all(&paths.data_dir).unwrap();
}

#[test]
fn rejects_agent_search_action_with_unobserved_url() {
    let inspections = vec![PageInspection {
        page: crate::domain::PageSample {
            role: "landing".into(),
            url: "https://m.cuoceng.com/".into(),
            html_path: "dom/landing.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "landing".into(),
        excerpt: String::new(),
        links: vec![crate::domain::PageLink {
            label: "搜索".into(),
            url: "https://m.cuoceng.com/book/so.html".into(),
        }],
        forms: vec![crate::domain::PageForm {
            action: "https://m.cuoceng.com/search".into(),
            method: "get".into(),
            input_names: vec!["searchkey".into()],
        }],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    }];

    let observed = build_observed_action_inputs(&inspections);
    let action = AgentAction {
        kind: "search".into(),
        url: "https://m.cuoceng.com/book/so".into(),
        role: "search".into(),
        reason: None,
        form_action: Some("https://m.cuoceng.com/book/so".into()),
        form_method: Some("get".into()),
        query_param: Some("searchkey".into()),
        wait_for_selector: None,
    };

    assert!(validate_agent_action(&action, &observed, "https://m.cuoceng.com/").is_some());
}

#[test]
fn rejects_agent_action_with_javascript_scheme() {
    let inspections = vec![PageInspection {
        page: crate::domain::PageSample {
            role: "landing".into(),
            url: "https://m.22biqu.com/".into(),
            html_path: "dom/landing.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "landing".into(),
        excerpt: String::new(),
        links: vec![crate::domain::PageLink {
            label: "返回".into(),
            url: "javascript:history.go(-1);".into(),
        }],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    }];

    let observed = build_observed_action_inputs(&inspections);
    let action = AgentAction {
        kind: "open".into(),
        url: "javascript:history.go(-1);".into(),
        role: "book".into(),
        reason: None,
        form_action: None,
        form_method: None,
        query_param: None,
        wait_for_selector: None,
    };

    assert_eq!(
        validate_agent_action(&action, &observed, "https://m.22biqu.com/"),
        Some("action url must use http/https scheme".into())
    );
}

#[test]
fn parses_agent_plan_with_trailing_text() {
    let content = r#"{
          "completed": false,
          "summary": "continue sampling",
          "actions": [
            {
              "kind": "open",
              "url": "https://m.cuoceng.com/book/ranking.html",
              "role": "explore"
            }
          ]
        }
        note: this is my reasoning"#;

    let plan = parse_agent_plan_response(content).unwrap();
    assert!(!plan.completed);
    assert_eq!(plan.actions.len(), 1);
    assert_eq!(
        plan.actions[0].url,
        "https://m.cuoceng.com/book/ranking.html"
    );
}

#[test]
fn infers_followup_toc_url_from_book_links() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "book".into(),
            url: "https://m.cuoceng.com/book/05c99514.html".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "book".into(),
        excerpt: String::new(),
        links: vec![
            crate::domain::PageLink {
                label: "首页".into(),
                url: "https://m.cuoceng.com/".into(),
            },
            crate::domain::PageLink {
                label: "查看全部章节 >>".into(),
                url: "https://m.cuoceng.com/book/chapter/05c99514.html".into(),
            },
        ],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_toc_url(&inspection).as_deref(),
        Some("https://m.cuoceng.com/book/chapter/05c99514.html")
    );
}

#[test]
fn followup_toc_url_prefers_chapter_link_over_category_catalog() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "book".into(),
            url: "https://m.cuoceng.com/book/05c99514.html".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "book".into(),
        excerpt: String::new(),
        links: vec![
            crate::domain::PageLink {
                label: "书库".into(),
                url: "https://m.cuoceng.com/book/category/catalog.html".into(),
            },
            crate::domain::PageLink {
                label: "查看全部章节 >>".into(),
                url: "https://m.cuoceng.com/book/chapter/05c99514-3e63-44d3-97c9-34de129280db.html"
                    .into(),
            },
        ],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_toc_url(&inspection).as_deref(),
        Some("https://m.cuoceng.com/book/chapter/05c99514-3e63-44d3-97c9-34de129280db.html")
    );
}

#[test]
fn infers_followup_content_url_from_toc_chapter_links() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "toc".into(),
            url: "https://m.cuoceng.com/book/chapter/05c99514.html".into(),
            html_path: "dom/toc.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "toc".into(),
        excerpt: String::new(),
        links: vec![
            crate::domain::PageLink {
                label: "首页".into(),
                url: "https://m.cuoceng.com/".into(),
            },
            crate::domain::PageLink {
                label: "第1章 长生万古 开始加点".into(),
                url: "https://m.cuoceng.com/book/05c99514/728eae05.html".into(),
            },
            crate::domain::PageLink {
                label: "第2章 修仙者降临".into(),
                url: "https://m.cuoceng.com/book/05c99514/abfb504f.html".into(),
            },
        ],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_content_url(&inspection).as_deref(),
        Some("https://m.cuoceng.com/book/05c99514/728eae05.html")
    );
}

#[test]
fn infers_followup_book_url_from_explore_cards() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "explore".into(),
            url: "https://m.cuoceng.com/book/category/catalog.html".into(),
            html_path: "dom/explore.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "explore".into(),
        excerpt: String::new(),
        links: vec![
            crate::domain::PageLink {
                label: "星门".into(),
                url: "https://m.cuoceng.com/book/44021.html".into(),
            },
            crate::domain::PageLink {
                label: "第1章 星门开启".into(),
                url: "https://m.cuoceng.com/book/44021/1001.html".into(),
            },
            crate::domain::PageLink {
                label: "书库".into(),
                url: "https://m.cuoceng.com/book/category/catalog.html".into(),
            },
        ],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_book_url(&inspection, Some("星门")).as_deref(),
        Some("https://m.cuoceng.com/book/44021.html")
    );
}

#[test]
fn infers_followup_book_url_from_dom_without_picking_history_page() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-explore-followup-book";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/explore.html"),
        r#"
            <html><body>
              <a href="/history.html">阅读历史</a>
              <div class="bookbox">
                <div class="bookinfo">
                  <h2 class="bookname"><a href="/book/44021.html">星门</a></h2>
                </div>
                <div class="delbutton"><a class="del_but" href="/book/44021.html">阅读</a></div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "explore".into(),
            url: "https://m.cuoceng.com/book/category/catalog.html".into(),
            html_path: "dom/explore.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "explore".into(),
        excerpt: String::new(),
        links: vec![],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_book_url_from_dom(&paths, task_id, &inspection, Some("星门")).as_deref(),
        Some("https://m.cuoceng.com/book/44021.html")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn infers_followup_content_url_from_explore_dom() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-explore-followup-content";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/explore.html"),
        r#"
            <html><body>
              <div class="bookbox">
                <div class="bookinfo">
                  <h2 class="bookname"><a href="/book/44021.html">星门</a></h2>
                  <a href="/book/44021/1001.html">第1章 星门开启</a>
                </div>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "explore".into(),
            url: "https://m.cuoceng.com/book/category/catalog.html".into(),
            html_path: "dom/explore.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "explore".into(),
        excerpt: String::new(),
        links: vec![],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_content_url_from_dom(&paths, task_id, &inspection).as_deref(),
        Some("https://m.cuoceng.com/book/44021/1001.html")
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn followup_content_url_prefers_start_read_over_catalog_links() {
    let inspection = PageInspection {
        page: crate::domain::PageSample {
            role: "book".into(),
            url: "https://m.cuoceng.com/book/05c99514.html".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        },
        title: "book".into(),
        excerpt: String::new(),
        links: vec![
            crate::domain::PageLink {
                label: "查看全部章节 >>".into(),
                url: "https://m.cuoceng.com/book/chapter/05c99514.html".into(),
            },
            crate::domain::PageLink {
                label: "开始阅读".into(),
                url: "https://m.cuoceng.com/book/05c99514/728eae05.html".into(),
            },
        ],
        forms: vec![],
        hints: crate::domain::PageHints::default(),
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    assert_eq!(
        infer_followup_content_url(&inspection).as_deref(),
        Some("https://m.cuoceng.com/book/05c99514/728eae05.html")
    );
}

#[test]
fn validate_search_rejects_when_sampled_book_is_not_best_keyword_match() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-search-mismatch";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();

    fs::write(
        task_root.join("dom/search.html"),
        r#"
            <html><body>
              <ul class="librarylist">
                <li><a class="novelname" href="/book/9058/">拉格朗日星门</a></li>
                <li><a class="novelname" href="/book/44021/">星门</a></li>
              </ul>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"<html><body><h1>拉格朗日星门</h1></body></html>"#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "biquge.xin".into(),
        base_url: "https://biquge.xin".into(),
        pages: vec![
            crate::domain::PageSample {
                role: "search".into(),
                url: "https://biquge.xin/search/result.html?searchkey=%E6%98%9F%E9%97%A8&page=1"
                    .into(),
                html_path: "dom/search.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "book".into(),
                url: "https://biquge.xin/book/9058/".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
        ],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };
    let candidate = SourceCandidate {
        strategy: "test".into(),
        confidence: 0.9,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "biquge.xin".into(),
            book_source_url: "https://biquge.xin".into(),
            enabled: Some(true),
            search_url: Some(
                "https://biquge.xin/search/result.html?searchkey={key}&page={page}".into(),
            ),
            rule_search: Some(SearchRule {
                book_list: Some(".librarylist li".into()),
                name: Some(".novelname@text".into()),
                book_url: Some(".novelname@attr[href]".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    };

    let phase = validate_search(&paths, task_id, &sampled, &candidate, Some("星门")).unwrap();
    assert!(!phase.passed);
    assert!(phase.summary.contains("采样详情页与搜索最佳匹配不一致"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn validate_search_allows_path_style_without_page_placeholder() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-search-path-style";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();

    fs::write(
        task_root.join("dom/search.html"),
        r#"
            <html><body>
              <ul class="result-list">
                <li><a class="book-link" href="/book/44021.html">星门</a></li>
              </ul>
            </body></html>
            "#,
    )
    .unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"<html><body><h1>星门</h1></body></html>"#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.cuoceng.com".into(),
        base_url: "https://m.cuoceng.com".into(),
        pages: vec![
            crate::domain::PageSample {
                role: "search".into(),
                url: "https://m.cuoceng.com/book/so/%E6%98%9F%E9%97%A8.html".into(),
                html_path: "dom/search.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
            crate::domain::PageSample {
                role: "book".into(),
                url: "https://m.cuoceng.com/book/44021.html".into(),
                html_path: "dom/book.html".into(),
                screenshot_path: None,
                snippet_paths: vec![],
            },
        ],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };
    let candidate = SourceCandidate {
        strategy: "test".into(),
        confidence: 0.9,
        evidence_refs: vec![],
        repair_history: vec![],
        book_source: BookSource {
            book_source_name: "m.cuoceng.com".into(),
            book_source_url: "https://m.cuoceng.com".into(),
            enabled: Some(true),
            search_url: Some("https://m.cuoceng.com/book/so/{key}.html".into()),
            rule_search: Some(SearchRule {
                book_list: Some(".result-list li".into()),
                name: Some(".book-link@text".into()),
                book_url: Some(".book-link@attr[href]".into()),
                ..Default::default()
            }),
            ..Default::default()
        },
    };

    let phase = validate_search(&paths, task_id, &sampled, &candidate, Some("星门")).unwrap();
    assert!(phase.passed);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn infer_book_info_rule_omits_toc_url_when_book_page_contains_inline_toc() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-inline-toc";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
        task_root.join("dom/book.html"),
        r#"
            <html><body>
              <meta property="og:novel:author" content="老鹰吃小鸡" />
              <meta property="og:novel:book_name" content="星门" />
              <meta property="og:description" content="简介" />
              <div class="card mt20 fulldir">
                <ul class="dirlist three clearfix">
                  <li><a href="/book/44021/1.html">第一章</a></li>
                  <li><a href="/book/44021/2.html">第二章</a></li>
                </ul>
              </div>
            </body></html>
            "#,
    )
    .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "biquge.xin".into(),
        base_url: "https://biquge.xin".into(),
        pages: vec![crate::domain::PageSample {
            role: "book".into(),
            url: "https://biquge.xin/book/44021/".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let rule = infer_book_info_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(rule.toc_url.as_deref(), None);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn infer_book_info_rule_prefers_real_toc_link_over_catalog_nav() {
    let root = std::env::temp_dir().join(format!("asd-test-{}", uuid::Uuid::new_v4()));
    let task_id = "task-book-info-real-toc";
    let task_root = root.join("tasks").join(task_id);
    fs::create_dir_all(task_root.join("dom")).unwrap();
    fs::write(
            task_root.join("dom/book.html"),
            r#"
            <html><body>
              <nav><a href="/book/category/catalog.html">书库</a></nav>
              <div class="bookinfo"><h1>天渊</h1></div>
              <div class="bookintro">简介</div>
              <div class="book chapterlist">
                <div id="allchapter">
                  <dl>
                    <dd><a href="/book/chapter/633122f9-be7e-411f-981e-0bd4666d7a8b.html">查看全部章节 >></a></dd>
                  </dl>
                </div>
              </div>
            </body></html>
            "#,
        )
        .unwrap();

    let paths = AppPaths {
        data_dir: root.clone(),
        db_path: root.join("app.sqlite"),
        tasks_dir: root.join("tasks"),
        logs_dir: root.join("logs"),
        settings_path: root.join("settings.json"),
        fixtures_dir: root.join("fixtures"),
        worker_script: root.join("worker.mjs"),
    };
    let sampled = SamplingResult {
        fixture_name: None,
        site_label: "m.cuoceng.com".into(),
        base_url: "https://m.cuoceng.com".into(),
        pages: vec![crate::domain::PageSample {
            role: "book".into(),
            url: "https://m.cuoceng.com/book/633122f9-be7e-411f-981e-0bd4666d7a8b.html".into(),
            html_path: "dom/book.html".into(),
            screenshot_path: None,
            snippet_paths: vec![],
        }],
        network_samples: vec![],
        manifest: ArtifactManifest::default(),
    };

    let rule = infer_book_info_rule_from_sampled(&paths, task_id, &sampled).unwrap();
    assert_eq!(
        rule.toc_url.as_deref(),
        Some("a[href*=\"/book/chapter/\"]@href")
    );

    let _ = fs::remove_dir_all(root);
}
