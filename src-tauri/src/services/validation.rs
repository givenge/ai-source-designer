use super::*;

pub(super) fn validate_candidate(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
    expected_keyword: Option<&str>,
) -> Result<ValidationReport> {
    ensure_compatibility(&candidate.book_source)?;

    let explore = phase_or_error(
        "explore",
        validate_explore(paths, task_id, sampled, candidate),
        sampled_page_evidence("explore", sampled),
    );
    let search = phase_or_error(
        "search",
        validate_search(paths, task_id, sampled, candidate, expected_keyword),
        sampled_page_evidence("search", sampled),
    );
    let book_info = phase_or_error(
        "bookInfo",
        validate_book_info(paths, task_id, sampled, candidate),
        sampled_page_evidence("book", sampled),
    );
    let toc = phase_or_error(
        "toc",
        validate_toc(paths, task_id, sampled, candidate),
        sampled_page_evidence("toc", sampled),
    );
    let content = phase_or_error(
        "content",
        validate_content(paths, task_id, sampled, candidate),
        sampled_page_evidence("content", sampled),
    );
    let cleanup = phase_or_error(
        "cleanup",
        validate_cleanup(paths, task_id, sampled, candidate),
        sampled_page_evidence("content", sampled),
    );

    Ok(ValidationReport {
        explore,
        search,
        book_info,
        toc,
        content,
        cleanup,
    })
}

fn phase_or_error(
    phase: &str,
    result: Result<ValidationPhase>,
    evidence_refs: Vec<EvidenceRef>,
) -> ValidationPhase {
    match result {
        Ok(value) => value,
        Err(error) => failed_phase(
            "验证阶段异常",
            vec![format!("{phase} validator error: {error}")],
            evidence_refs,
        ),
    }
}

fn has_discovery_coverage(report: &ValidationReport) -> bool {
    report.explore.passed || report.search.passed
}

pub(super) fn report_is_usable(report: &ValidationReport) -> bool {
    has_discovery_coverage(report)
        && report.book_info.passed
        && report.toc.passed
        && report.content.passed
        && report.cleanup.passed
}

pub(super) fn report_requires_repair(report: &ValidationReport) -> bool {
    !report_is_usable(report)
}

fn validate_explore(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Ok(page) = sampled_page(sampled, "explore") else {
        return Ok(passed_phase(
            "未采样探索页，暂时跳过探索验证。".into(),
            vec!["当前任务没有单独的书海探索页样本。".into()],
            Vec::new(),
        ));
    };

    let Some(explore_url) = candidate.book_source.explore_url.as_ref() else {
        return Ok(failed_phase(
            "缺少 exploreUrl",
            vec!["存在探索页样本，但候选书源没有输出 exploreUrl。".into()],
            sampled_page_evidence("explore", sampled),
        ));
    };
    let Some(rule) = candidate.book_source.rule_explore.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleExplore",
            vec!["存在探索页样本，但候选书源没有输出 ruleExplore。".into()],
            sampled_page_evidence("explore", sampled),
        ));
    };
    if rule
        .book_list
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Ok(failed_phase(
            "ruleExplore.bookList missing",
            vec!["ruleExplore.bookList 为空。".into()],
            sampled_page_evidence("explore", sampled),
        ));
    }

    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let item_selector = selector(rule.book_list.as_deref().unwrap_or_default())?;
    let items: Vec<_> = document.select(&item_selector).collect();
    if items.is_empty() {
        return Ok(failed_phase(
            "探索列表为空",
            vec!["ruleExplore.bookList 没有命中任何候选书籍。".into()],
            sampled_page_evidence("explore", sampled),
        ));
    }

    let first = items[0];
    let name = evaluate_on_element_safe(&first, rule.name.as_deref().unwrap_or_default())?;
    let book_url = normalize_url(
        &page.url,
        &evaluate_on_element_safe(&first, rule.book_url.as_deref().unwrap_or_default())?,
    )?;

    if name.trim().is_empty() || book_url.trim().is_empty() {
        return Ok(failed_phase(
            "探索规则缺少书名或链接",
            vec!["ruleExplore.name 或 ruleExplore.bookUrl 没有提取到有效值。".into()],
            sampled_page_evidence("explore", sampled),
        ));
    }

    Ok(passed_phase(
        "探索规则通过。".into(),
        vec![
            format!("探索入口：{explore_url}"),
            format!("首个结果：{name}"),
            format!("详情链接：{book_url}"),
        ],
        sampled_page_evidence("explore", sampled),
    ))
}

fn ensure_compatibility(source: &BookSource) -> Result<()> {
    if source.book_source_name.trim().is_empty() || source.book_source_url.trim().is_empty() {
        bail!("bookSourceName and bookSourceUrl are required");
    }

    let value = serde_json::to_value(source)?;
    if value
        .pointer("/ruleContent/title")
        .is_some_and(|field| !field.is_null())
    {
        bail!("ruleContent.title is not supported by Reader-Rust model");
    }
    if value
        .pointer("/ruleToc/updateTime")
        .is_some_and(|field| !field.is_null())
    {
        bail!("ruleToc.updateTime is not supported by Reader-Rust model");
    }

    Ok(())
}

pub(super) fn validate_search(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
    expected_keyword: Option<&str>,
) -> Result<ValidationPhase> {
    let Some(search_url) = candidate.book_source.search_url.as_ref() else {
        return Ok(failed_phase(
            "缺少 searchUrl",
            vec!["模型未输出 searchUrl，无法执行搜索验证。".into()],
            sampled_page_evidence("landing", sampled),
        ));
    };
    let Some(rule) = candidate.book_source.rule_search.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleSearch",
            vec!["模型未输出 ruleSearch，无法执行搜索结果提取验证。".into()],
            sampled_page_evidence("search", sampled),
        ));
    };
    if !search_url.contains("{key}") {
        return Ok(failed_phase(
            "searchUrl 必须包含 {key}",
            vec!["当前 searchUrl 缺少关键词替换占位符。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    if rule
        .book_list
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Ok(failed_phase(
            "ruleSearch.bookList missing",
            vec!["bookList selector is empty".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let page = match sampled_page(sampled, "search") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少搜索结果页样本",
                vec!["当前任务只采样了落地页，尚未拿到真实搜索结果页。".into()],
                sampled_page_evidence("landing", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let book_list_selector = selector(rule.book_list.as_deref().unwrap_or_default())?;
    let items: Vec<_> = document.select(&book_list_selector).collect();

    if items.is_empty() {
        return Ok(failed_phase(
            "搜索列表为空",
            vec!["bookList 选择器没有命中任何结果。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let mut extracted = Vec::new();
    for item in &items {
        let name = evaluate_on_element_safe(item, rule.name.as_deref().unwrap_or_default())?;
        let book_url =
            evaluate_on_element_safe(item, rule.book_url.as_deref().unwrap_or_default())?;
        let resolved = normalize_url(&page.url, &book_url)?;
        if !name.trim().is_empty() && !resolved.trim().is_empty() {
            extracted.push((name, resolved));
        }
    }

    if extracted.is_empty() {
        return Ok(failed_phase(
            "搜索结果缺少书名或详情链接",
            vec!["name 或 bookUrl 规则没有提取到有效结果。".into()],
            sampled_page_evidence("search", sampled),
        ));
    }

    let expected_keyword = expected_keyword.filter(|value| !value.trim().is_empty());
    let best = if let Some(keyword) = expected_keyword {
        extracted
            .iter()
            .enumerate()
            .map(|(index, (name, resolved))| {
                (index, name, resolved, keyword_match_score(name, keyword))
            })
            .max_by(|left, right| {
                left.3
                    .cmp(&right.3)
                    .then_with(|| right.1.len().cmp(&left.1.len()))
            })
    } else {
        extracted
            .iter()
            .enumerate()
            .map(|(index, (name, resolved))| (index, name, resolved, 1))
            .next()
    };

    let Some((_, name, resolved, score)) = best else {
        return Ok(failed_phase(
            "搜索结果缺少有效候选",
            vec!["无法从搜索结果中确定可用的候选书籍。".into()],
            sampled_page_evidence("search", sampled),
        ));
    };

    if expected_keyword.is_some() && score <= 0 {
        return Ok(failed_phase(
            "搜索结果未命中测试关键词",
            vec![format!(
                "没有找到书名匹配测试关键词的结果：{}",
                expected_keyword.unwrap_or_default()
            )],
            sampled_page_evidence("search", sampled),
        ));
    }

    if let Ok(book_page) = sampled_page(sampled, "book") {
        let sampled_book_url = canonical_compare_url(&book_page.url)?;
        let expected_book_url = canonical_compare_url(resolved)?;
        if sampled_book_url != expected_book_url {
            return Ok(failed_phase(
                "采样详情页与搜索最佳匹配不一致",
                vec![
                    format!("测试关键词最佳匹配结果：{name}"),
                    format!("最佳匹配链接：{resolved}"),
                    format!("实际采样详情页：{}", book_page.url),
                ],
                sampled_page_evidence("search", sampled),
            ));
        }
    }

    Ok(passed_phase(
        format!("搜索规则通过，共命中 {} 条结果。", extracted.len()),
        vec![
            format!("最佳匹配书名：{name}"),
            format!("最佳匹配链接：{resolved}"),
        ],
        sampled_page_evidence("search", sampled),
    ))
}

fn validate_book_info(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_book_info.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleBookInfo",
            vec!["模型未输出详情页规则。".into()],
            sampled_page_evidence("book", sampled),
        ));
    };
    let page = match sampled_page(sampled, "book") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少详情页样本",
                vec!["当前任务未采样到书籍详情页。".into()],
                sampled_page_evidence("search", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);

    let name = evaluate_on_document_safe(&document, rule.name.as_deref().unwrap_or_default())?;
    let author = evaluate_on_document_safe(&document, rule.author.as_deref().unwrap_or_default())?;
    let intro = evaluate_on_document_safe(&document, rule.intro.as_deref().unwrap_or_default())?;
    let toc_url =
        evaluate_on_document_safe(&document, rule.toc_url.as_deref().unwrap_or_default())?;
    let resolved_toc = normalize_url(&page.url, &toc_url)?;

    if name.trim().is_empty() || (author.trim().is_empty() && intro.trim().is_empty()) {
        return Ok(failed_phase(
            "详情规则缺少必要字段",
            vec!["ruleBookInfo 至少应提取 name，以及 author 或 intro。".into()],
            sampled_page_evidence("book", sampled),
        ));
    }

    Ok(passed_phase(
        "详情页规则通过。".into(),
        vec![
            format!("书名：{name}"),
            format!("作者：{author}"),
            format!("目录链接：{resolved_toc}"),
        ],
        sampled_page_evidence("book", sampled),
    ))
}

fn validate_toc(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_toc.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleToc",
            vec!["模型未输出目录页规则。".into()],
            sampled_page_evidence("toc", sampled),
        ));
    };
    let page = match sampled_page(sampled, "toc") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少目录页样本",
                vec!["当前任务未采样到目录页。".into()],
                sampled_page_evidence("book", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    if rule
        .chapter_list
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Ok(failed_phase(
            "ruleToc.chapterList missing",
            vec!["chapterList selector is empty".into()],
            sampled_page_evidence("toc", sampled),
        ));
    }
    let chapter_selector = selector(rule.chapter_list.as_deref().unwrap_or_default())?;
    let chapters: Vec<_> = document.select(&chapter_selector).collect();

    if chapters.len() < 2 {
        return Ok(failed_phase(
            "目录章节不足",
            vec!["ruleToc.chapterList 没有命中足够的章节项。".into()],
            sampled_page_evidence("toc", sampled),
        ));
    }

    if toc_page_has_pagination(&document)
        && rule
            .next_toc_url
            .as_deref()
            .unwrap_or_default()
            .trim()
            .is_empty()
    {
        return Ok(failed_phase(
            "目录分页规则缺失",
            vec!["目录页存在明显分页结构，但 ruleToc.nextTocUrl 为空。".into()],
            sampled_page_evidence("toc", sampled),
        ));
    }

    let first = chapters[0];
    let chapter_name =
        evaluate_on_element_safe(&first, rule.chapter_name.as_deref().unwrap_or_default())?;
    let chapter_url = normalize_url(
        &page.url,
        &evaluate_on_element_safe(&first, rule.chapter_url.as_deref().unwrap_or_default())?,
    )?;

    Ok(passed_phase(
        format!("目录规则通过，共提取 {} 章。", chapters.len()),
        vec![
            format!("首章标题：{chapter_name}"),
            format!("首章链接：{chapter_url}"),
        ],
        sampled_page_evidence("toc", sampled),
    ))
}

fn toc_page_has_pagination(document: &Html) -> bool {
    let selectors = [
        "#indexselect option",
        ".index-container select option",
        "select#page_select option",
        ".pagination .next",
        "a.next",
        "a[rel=\"next\"]",
    ];

    for candidate in selectors {
        let Ok(selector) = Selector::parse(candidate) else {
            continue;
        };
        let count = document.select(&selector).count();
        if candidate.ends_with("option") {
            if count >= 2 {
                return true;
            }
        } else if count >= 1 {
            return true;
        }
    }

    false
}

pub(super) fn validate_content(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_content.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleContent",
            vec!["模型未输出正文规则。".into()],
            sampled_page_evidence("content", sampled),
        ));
    };
    let page = match sampled_page(sampled, "content") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少正文页样本",
                vec!["当前任务未采样到正文页。".into()],
                sampled_page_evidence("toc", sampled),
            ));
        }
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let content =
        evaluate_on_document_safe(&document, rule.content.as_deref().unwrap_or_default())?;

    if content.trim().is_empty() {
        return Ok(failed_phase(
            "正文为空",
            vec!["ruleContent.content 没有提取到正文。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    let content_noise_tokens = ["首页", "目录", "书签", "报错", "关灯", "字体+", "字体-"];
    let noisy_hits = content_noise_tokens
        .iter()
        .map(|token| content.matches(token).count())
        .sum::<usize>();
    if noisy_hits >= 4 {
        return Ok(failed_phase(
            "正文提取包含导航按钮文本",
            vec![
                "ruleContent.content 命中了页面导航/操作区，需收窄 selector 或增加 replaceRegex。"
                    .into(),
            ],
            sampled_page_evidence("content", sampled),
        ));
    }

    let next_url = evaluate_on_document_safe(
        &document,
        rule.next_content_url.as_deref().unwrap_or_default(),
    )
    .unwrap_or_default();
    let resolved_next = if next_url.trim().is_empty() {
        String::new()
    } else {
        normalize_url(&page.url, &next_url)?
    };

    Ok(passed_phase(
        "正文规则通过。".into(),
        vec![
            format!("正文长度：{}", content.chars().count()),
            format!(
                "下一页：{}",
                if resolved_next.is_empty() {
                    "无".into()
                } else {
                    resolved_next
                }
            ),
        ],
        sampled_page_evidence("content", sampled),
    ))
}

fn validate_cleanup(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    candidate: &SourceCandidate,
) -> Result<ValidationPhase> {
    let Some(rule) = candidate.book_source.rule_content.as_ref() else {
        return Ok(failed_phase(
            "缺少 ruleContent",
            vec!["当前候选缺少正文规则，无法执行清洗验证。".into()],
            sampled_page_evidence("content", sampled),
        ));
    };
    let first_page = match sampled_page(sampled, "content") {
        Ok(page) => page,
        Err(_) => {
            return Ok(failed_phase(
                "缺少正文页样本",
                vec!["当前任务未采样到正文页，暂时无法执行清洗验证。".into()],
                sampled_page_evidence("toc", sampled),
            ));
        }
    };
    let first_html = fs::read_to_string(paths.task_path(task_id, &first_page.html_path))?;
    let first_doc = Html::parse_document(&first_html);
    let mut combined =
        evaluate_on_document_safe(&first_doc, rule.content.as_deref().unwrap_or_default())?;

    if let Some(next_page) = sampled
        .pages
        .iter()
        .find(|page| page.role == "content-next")
    {
        let next_html = fs::read_to_string(paths.task_path(task_id, &next_page.html_path))?;
        let next_doc = Html::parse_document(&next_html);
        let next_content =
            evaluate_on_document_safe(&next_doc, rule.content.as_deref().unwrap_or_default())?;
        combined.push('\n');
        combined.push_str(&next_content);
    }

    let cleaned = apply_replace_regex(&combined, rule.replace_regex.as_deref())?;
    let contains_ad = cleaned.contains("Sponsored by AI Source Designer");
    if contains_ad {
        return Ok(failed_phase(
            "广告清洗失败",
            vec!["replaceRegex 未能移除 fixture 中的广告文本。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    if cleaned.chars().count() < 50 {
        return Ok(failed_phase(
            "清洗后正文过短",
            vec!["内容清洗后没有保留足够的正文文本。".into()],
            sampled_page_evidence("content", sampled),
        ));
    }

    Ok(passed_phase(
        "清洗规则通过。".into(),
        vec![format!("清洗后正文长度：{}", cleaned.chars().count())],
        sampled_page_evidence("content", sampled),
    ))
}

pub(super) fn first_failure_summary(report: &ValidationReport) -> String {
    if !has_discovery_coverage(report) {
        return "search 与 explore 至少需要一个可用".into();
    }

    for phase in [
        &report.book_info,
        &report.toc,
        &report.content,
        &report.cleanup,
    ] {
        if !phase.passed {
            return phase.summary.clone();
        }
    }
    "任务失败".into()
}

pub(super) fn apply_replace_regex(content: &str, replace_regex: Option<&str>) -> Result<String> {
    let Some(value) = replace_regex else {
        return Ok(content.to_string());
    };

    if value.trim().is_empty() {
        return Ok(content.to_string());
    }

    let parts: Vec<_> = value.split("##").collect();
    let mut current = content.to_string();
    let mut index = 0;
    while index < parts.len() {
        let pattern = parts[index];
        let replacement = parts.get(index + 1).copied().unwrap_or("");
        let regex = Regex::new(pattern)?;
        current = regex.replace_all(&current, replacement).to_string();
        index += 2;
    }
    Ok(current)
}

pub(super) fn sampled_page_evidence(role: &str, sampled: &SamplingResult) -> Vec<EvidenceRef> {
    sampled
        .pages
        .iter()
        .find(|page| page.role == role)
        .map(|page| {
            let mut refs = Vec::new();
            if let Some(screenshot) = &page.screenshot_path {
                refs.push(EvidenceRef {
                    label: format!("{role} screenshot"),
                    artifact_path: screenshot.clone(),
                    detail: Some(page.url.clone()),
                });
            }
            for snippet in &page.snippet_paths {
                refs.push(EvidenceRef {
                    label: format!("{role} dom"),
                    artifact_path: snippet.clone(),
                    detail: Some(page.url.clone()),
                });
            }
            refs
        })
        .unwrap_or_default()
}

pub(super) fn passed_phase(
    summary: String,
    details: Vec<String>,
    evidence_refs: Vec<EvidenceRef>,
) -> ValidationPhase {
    ValidationPhase {
        passed: true,
        summary,
        details,
        evidence_refs,
    }
}

pub(super) fn failed_phase(
    summary: &str,
    details: Vec<String>,
    evidence_refs: Vec<EvidenceRef>,
) -> ValidationPhase {
    ValidationPhase {
        passed: false,
        summary: summary.into(),
        details,
        evidence_refs,
    }
}
