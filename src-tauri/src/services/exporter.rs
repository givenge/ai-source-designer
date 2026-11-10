use super::*;

pub(super) fn append_output(
    paths: &AppPaths,
    task_id: &str,
    task: &mut TaskRecord,
    relative_path: String,
) {
    let manifest = task
        .artifact_manifest
        .get_or_insert_with(ArtifactManifest::default);
    if !manifest.output.iter().any(|entry| entry == &relative_path) {
        manifest.output.push(relative_path);
    }
    let _ = write_logs(paths, task);
    let _ = task_id;
}

pub(super) fn export_candidate(
    paths: &AppPaths,
    task_id: &str,
    candidate: &SourceCandidate,
) -> Result<(String, String)> {
    let exported = exportable_book_source_value(&candidate.book_source)?;
    let raw = serde_json::to_string_pretty(&exported)?;
    let output_path = paths.task_path(task_id, "output/book-source.json");
    write_export_file(&output_path, &raw)?;
    Ok((paths.relative_to_task(task_id, &output_path), raw))
}

pub(super) fn write_export_file(path: &Path, raw: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, raw)?;
    Ok(())
}

pub(super) fn exportable_book_source_value(book_source: &BookSource) -> Result<Value> {
    let mut normalized = book_source.clone();
    super::candidate_parser::normalize_book_source_rules(&mut normalized);
    let book_source = &normalized;
    let mut value = serde_json::to_value(book_source)?;

    if is_22biqu_mobile_template(book_source) {
        apply_22biqu_mobile_export_template(&mut value, book_source);
    }

    normalize_export_value_for_reader(&mut value);

    Ok(value)
}

fn is_22biqu_mobile_template(book_source: &BookSource) -> bool {
    let url = book_source.book_source_url.trim().to_ascii_lowercase();
    let Some(search) = book_source.rule_search.as_ref() else {
        return false;
    };
    let Some(book_info) = book_source.rule_book_info.as_ref() else {
        return false;
    };
    let Some(content) = book_source.rule_content.as_ref() else {
        return false;
    };

    (url.contains("22biqu") || url.contains("biqu"))
        && search.book_list.as_deref() == Some(".bookbox")
        && search.name.as_deref() == Some(".bookname a@text")
        && book_info.name.as_deref() == Some(".channelHeader .title@text")
        && content.content.as_deref() == Some("#chaptercontent@textNodes")
}

fn apply_22biqu_mobile_export_template(value: &mut Value, book_source: &BookSource) {
    let base_url = ensure_trailing_slash(&book_source.book_source_url);
    let host = url::Url::parse(&base_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_string))
        .unwrap_or_else(|| "m.22biqu.net".into());
    let host_pattern = regex::escape(&host);

    if let Some(map) = value.as_object_mut() {
        map.insert("bookSourceName".into(), Value::String("笔趣阁".into()));
        map.insert("bookSourceGroup".into(), Value::String("小说".into()));
        map.insert("bookSourceUrl".into(), Value::String(base_url.clone()));
        map.insert("bookSourceType".into(), Value::from(0));
        map.insert(
            "bookUrlPattern".into(),
            Value::String(format!("https?://{host_pattern}/biqu\\d+/")),
        );
        map.insert("enabledExplore".into(), Value::Bool(true));
        map.insert("enabledCookieJar".into(), Value::Null);
        map.insert("jsLib".into(), Value::Null);
        map.insert("concurrentRate".into(), Value::Null);
        map.insert("header".into(), Value::Null);
        map.insert("loginUrl".into(), Value::Null);
        map.insert("loginUi".into(), Value::Null);
        map.insert("loginCheckJs".into(), Value::Null);
        map.insert("coverDecodeJs".into(), Value::Null);
        map.insert(
            "lastUpdateTime".into(),
            Value::from(
                book_source
                    .last_update_time
                    .unwrap_or_else(|| Utc::now().timestamp_millis()),
            ),
        );
        map.insert("customOrder".into(), Value::from(-99880));
        map.insert("weight".into(), Value::from(0));
        map.insert(
            "respondTime".into(),
            Value::from(book_source.respond_time.unwrap_or(0)),
        );
        map.insert("loadWithBaseUrl".into(), Value::Null);
        map.insert("singleUrl".into(), Value::Null);
        map.insert("exploreScreen".into(), Value::Null);
        map.insert("ruleReview".into(), Value::Null);
        map.insert("bookSourceComment".into(), Value::Null);
        map.insert("variableComment".into(), Value::Null);
        map.insert(
            "exploreUrl".into(),
            Value::String(
                "[{\"title\":\"排行🏷榜单\",\"url\":\"\",\"style\":{\"layout_flexBasisPercent\":1,\"layout_flexGrow\":1}},{\"title\":\"总排行榜\",\"url\":\"/rank/\",\"style\":{\"layout_flexBasisPercent\":0.4,\"layout_flexGrow\":1}},{\"title\":\"月排行榜\",\"url\":\"/rank/monthvisit/\",\"style\":{\"layout_flexBasisPercent\":0.4,\"layout_flexGrow\":1}},{\"title\":\"周排行榜\",\"url\":\"/rank/weekvisit/\",\"style\":{\"layout_flexBasisPercent\":0.4,\"layout_flexGrow\":1}},{\"title\":\"收藏榜\",\"url\":\"/rank/goodnum/\",\"style\":{\"layout_flexBasisPercent\":0.4,\"layout_flexGrow\":1}},{\"title\":\"标签🏷分类\",\"url\":\"\",\"style\":{\"layout_flexBasisPercent\":1,\"layout_flexGrow\":1}},{\"title\":\"全部分类\",\"url\":\"/fenlei/{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"玄幻奇幻\",\"url\":\"/fenlei/1_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"武侠修真\",\"url\":\"/fenlei/2_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"都市言情\",\"url\":\"/fenlei/3_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"历史军事\",\"url\":\"/fenlei/4_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"科幻灵异\",\"url\":\"/fenlei/6_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"游戏竞技\",\"url\":\"/fenlei/5_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"女生耽美\",\"url\":\"/fenlei/7_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}},{\"title\":\"其他类型\",\"url\":\"/fenlei/8_{{page}}.html\",\"style\":{\"layout_flexBasisPercent\":0.25,\"layout_flexGrow\":1}}]".into(),
            ),
        );
        map.insert(
            "ruleExplore".into(),
            json!({
                "checkKeyWord": Value::Null,
                "bookList": ".hot_sale",
                "name": ".title@text##\\d+\\.",
                "author": ".author@text##（.*）",
                "intro": ".review@text##简介：",
                "kind": "",
                "lastChapter": Value::Null,
                "updateTime": Value::Null,
                "bookUrl": "a.0@href",
                "coverUrl": "a.0@href##.+\\D((\\d+)\\d{3})\\D##https://img.22biqu.com/$2/$1/$1s.jpg###",
                "wordCount": Value::Null
            }),
        );
        map.insert(
            "searchUrl".into(),
            Value::String(
                "{{cookie.removeCookie(source.getKey())}}/ss/,\n{\n  \"body\": \"searchkey={{key}}\",\n  \"method\": \"POST\"\n}"
                    .into(),
            ),
        );
        map.insert(
            "ruleSearch".into(),
            json!({
                "checkKeyWord": Value::Null,
                "bookList": ".bookbox",
                "name": ".bookname@a@text",
                "author": ".author.0@text##作者：",
                "intro": Value::Null,
                "kind": ".author.1@text##类型：",
                "lastChapter": ".update@a@text##最后章节：",
                "updateTime": Value::Null,
                "bookUrl": ".bookname@a@href",
                "coverUrl": "img@src",
                "wordCount": Value::Null
            }),
        );
        map.insert(
            "ruleBookInfo".into(),
            json!({
                "init": "@put:{n:\"[property$=book_name]@content\",a:\"[property$=author]@content\",k:\"[property~=category|status|update_time]@content\",l:\"[property~=las?test_chapter_name]@content\",i:\"[property$=description]@content\",c:\"[property$=image]@content\"}",
                "name": "@get:{n}",
                "author": "@get:{a}",
                "intro": "@get:{i}",
                "kind": "@get:{k}",
                "lastChapter": "@get:{l}",
                "updateTime": Value::Null,
                "coverUrl": "@get:{c}",
                "wordCount": "",
                "tocUrl": Value::Null,
                "canReName": Value::Null,
                "downloadUrls": Value::Null
            }),
        );
        map.insert(
            "ruleToc".into(),
            json!({
                "preUpdateJs": Value::Null,
                "init": Value::Null,
                "chapterList": ".directoryArea p a",
                "chapterName": "text",
                "chapterUrl": "href",
                "formatJs": Value::Null,
                "isVolume": Value::Null,
                "isVip": Value::Null,
                "isPay": Value::Null,
                "updateTime": Value::Null,
                "nextTocUrl": "option@value"
            }),
        );
        map.insert(
            "ruleContent".into(),
            json!({
                "content": "#chaptercontent@html",
                "title": Value::Null,
                "nextContentUrl": "text.下一页@href",
                "webJs": Value::Null,
                "sourceRegex": Value::Null,
                "replaceRegex": "##\\\\s*({{ book.durChapterTitle }}|.*作者：.*|PS：.*求推荐！|PS：.*求收藏！|感谢.*打赏.*|感谢.*推荐票.*|感谢.*月票.*|（.*月票.*）|（为大家的.*票加更.*）|第二更在.*|为防止采集.*支持！|网址：sudugu\\\\.com|更多.*書吧看！|无错.*小说。|必应.*速读谷|loadAdv.*)\\\\s*",
                "imageStyle": Value::Null,
                "imageDecode": Value::Null,
                "payAction": Value::Null
            }),
        );
    }
}

fn ensure_trailing_slash(url: &str) -> String {
    let trimmed = url.trim();
    if trimmed.ends_with('/') {
        trimmed.to_string()
    } else {
        format!("{trimmed}/")
    }
}
