
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
fn serialized_book_source_does_not_emit_doc_only_fields() {
    let source = serde_json::to_value(good_candidate().book_source).unwrap();
    assert!(source.pointer("/ruleContent/title").is_none());
    assert!(source.pointer("/ruleToc/updateTime").is_none());
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
    assert!(report.explore.passed);
    assert!(report.search.passed);
    assert!(report.book_info.passed);
    assert!(report.toc.passed);
    assert!(report.content.passed);
    assert!(report.cleanup.passed);

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
    let report = ValidationReport {
        explore: passed_phase("explore ok".into(), vec![], vec![]),
        search: failed_phase("search missing", vec![], vec![]),
        book_info: passed_phase("book ok".into(), vec![], vec![]),
        toc: passed_phase("toc ok".into(), vec![], vec![]),
        content: passed_phase("content ok".into(), vec![], vec![]),
        cleanup: passed_phase("cleanup ok".into(), vec![], vec![]),
    };

    assert!(report_is_usable(&report));
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

    assert!(validate_agent_action(&action, &observed).is_some());
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
        validate_agent_action(&action, &observed),
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
