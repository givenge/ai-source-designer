use super::*;

pub(super) fn merge_inspection(
    pages: &mut Vec<crate::domain::PageSample>,
    network_samples: &mut Vec<crate::domain::NetworkSample>,
    manifest: &mut ArtifactManifest,
    inspections: &mut Vec<PageInspection>,
    inspection: PageInspection,
) {
    merge_manifest(manifest, &inspection.manifest);
    for sample in &inspection.network_samples {
        if !network_samples
            .iter()
            .any(|existing| existing.response_path == sample.response_path)
        {
            network_samples.push(sample.clone());
        }
    }
    if !pages.iter().any(|existing| {
        existing.role == inspection.page.role && existing.url == inspection.page.url
    }) {
        pages.push(inspection.page.clone());
    }
    inspections.push(inspection);
}

fn collect_task_entries(paths: &AppPaths, task_id: &str, dir: &str) -> Vec<String> {
    let root = paths.task_path(task_id, dir);
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };

    let mut items = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_file() {
                return None;
            }
            Some(paths.relative_to_task(task_id, &path))
        })
        .collect::<Vec<_>>();
    items.sort();
    items
}

fn merge_manifest_entries(target: &mut Vec<String>, incoming: Vec<String>) {
    for entry in incoming {
        if !target.iter().any(|existing| existing == &entry) {
            target.push(entry);
        }
    }
    target.sort();
}

pub(super) fn load_sampled_from_task(
    paths: &AppPaths,
    task: &TaskRecord,
) -> Result<Option<SamplingResult>> {
    let mut manifest = task.artifact_manifest.clone().unwrap_or_default();
    merge_manifest_entries(
        &mut manifest.screenshots,
        collect_task_entries(paths, &task.id, "screenshots"),
    );
    merge_manifest_entries(
        &mut manifest.dom,
        collect_task_entries(paths, &task.id, "dom"),
    );
    merge_manifest_entries(
        &mut manifest.network,
        collect_task_entries(paths, &task.id, "network"),
    );
    merge_manifest_entries(
        &mut manifest.logs,
        collect_task_entries(paths, &task.id, "logs"),
    );
    merge_manifest_entries(
        &mut manifest.output,
        collect_task_entries(paths, &task.id, "output"),
    );
    if manifest.dom.is_empty() && manifest.screenshots.is_empty() && manifest.network.is_empty() {
        return Ok(None);
    }

    let worker_log_path = paths.task_path(&task.id, "logs/worker.log");
    let worker_log = fs::read_to_string(&worker_log_path).unwrap_or_default();
    let capture_pattern = Regex::new(r"capture role=([a-z-]+) url=(\S+)")?;
    let network_pattern = Regex::new(r"network-sample label=([^\s]+).+? url=(\S+)")?;

    let mut page_urls = std::collections::HashMap::new();
    for captures in capture_pattern.captures_iter(&worker_log) {
        let role = captures
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or_default();
        let url = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or_default();
        if !role.is_empty() && !url.is_empty() {
            page_urls.insert(role.to_string(), url.to_string());
        }
    }

    let mut pages = Vec::new();
    for dom_path in &manifest.dom {
        let path = Path::new(dom_path);
        let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".html") {
            continue;
        }
        let role = file_name.trim_end_matches(".html").to_string();
        let url = page_urls
            .get(&role)
            .cloned()
            .unwrap_or_else(|| task.target_url.clone());
        let screenshot_path = manifest
            .screenshots
            .iter()
            .find(|entry| {
                Path::new(entry)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .is_some_and(|stem| stem == role)
            })
            .cloned();
        pages.push(crate::domain::PageSample {
            role,
            url,
            html_path: dom_path.clone(),
            screenshot_path,
            snippet_paths: Vec::new(),
        });
    }

    let mut network_urls = std::collections::HashMap::new();
    for captures in network_pattern.captures_iter(&worker_log) {
        let label = captures
            .get(1)
            .map(|value| value.as_str())
            .unwrap_or_default();
        let url = captures
            .get(2)
            .map(|value| value.as_str())
            .unwrap_or_default();
        if !label.is_empty() && !url.is_empty() {
            network_urls.insert(label.to_string(), url.to_string());
        }
    }

    let network_samples = manifest
        .network
        .iter()
        .map(|relative| {
            let label = Path::new(relative)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_string();
            crate::domain::NetworkSample {
                label: label.clone(),
                request_url: network_urls
                    .get(&label)
                    .cloned()
                    .unwrap_or_else(|| task.target_url.clone()),
                method: "GET".into(),
                response_path: relative.clone(),
                notes: None,
            }
        })
        .collect::<Vec<_>>();

    Ok(Some(SamplingResult {
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
    }))
}

pub(super) fn load_inspections_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<Vec<PageInspection>> {
    sampled
        .pages
        .iter()
        .map(|page| load_inspection_from_page(paths, task_id, page))
        .collect()
}

fn load_inspection_from_page(
    paths: &AppPaths,
    task_id: &str,
    page: &crate::domain::PageSample,
) -> Result<PageInspection> {
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let title = first_matching_document_rule(&document, &["title@text"]).unwrap_or_default();
    let excerpt = document
        .root_element()
        .text()
        .map(str::trim)
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1600)
        .collect::<String>();

    let link_selector = Selector::parse("a[href]").map_err(|_| anyhow!("invalid selector"))?;
    let links = document
        .select(&link_selector)
        .filter_map(|anchor| {
            let href = anchor.value().attr("href")?.trim();
            if href.is_empty() {
                return None;
            }
            let label = anchor
                .text()
                .map(str::trim)
                .filter(|chunk| !chunk.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if label.is_empty() {
                return None;
            }
            Some(crate::domain::PageLink {
                label,
                url: normalize_url(&page.url, href).unwrap_or_else(|_| href.to_string()),
            })
        })
        .take(24)
        .collect::<Vec<_>>();

    let form_selector = Selector::parse("form").map_err(|_| anyhow!("invalid selector"))?;
    let input_selector = Selector::parse("input[name], textarea[name], select[name]")
        .map_err(|_| anyhow!("invalid selector"))?;
    let forms = document
        .select(&form_selector)
        .map(|form| crate::domain::PageForm {
            action: normalize_url(&page.url, form.value().attr("action").unwrap_or(&page.url))
                .unwrap_or_else(|_| page.url.clone()),
            method: form
                .value()
                .attr("method")
                .unwrap_or("get")
                .to_ascii_lowercase(),
            input_names: form
                .select(&input_selector)
                .filter_map(|input| input.value().attr("name").map(str::to_string))
                .collect(),
        })
        .collect::<Vec<_>>();

    let hints = crate::domain::PageHints {
        book_list_candidates: vec![
            ".bookbox".into(),
            ".librarylist li".into(),
            ".result-list .result-item".into(),
        ],
        title_link_candidates: vec![
            ".bookname a[href]".into(),
            "h2 a[href]".into(),
            "h3 a[href]".into(),
        ],
        toc_link_candidates: vec![
            "#allchapter a[href]".into(),
            ".chapterlist a[href]".into(),
            ".directoryArea a[href]".into(),
        ],
        content_candidates: vec![
            "#chaptercontent".into(),
            "#content".into(),
            ".content".into(),
        ],
        next_page_candidates: vec!["a[href*=\"next\"]".into(), "a[href*=\"_2.html\"]".into()],
    };

    Ok(PageInspection {
        page: page.clone(),
        title,
        excerpt,
        links,
        forms,
        hints,
        network_samples: Vec::new(),
        manifest: ArtifactManifest::default(),
    })
}

fn merge_manifest(target: &mut ArtifactManifest, source: &ArtifactManifest) {
    merge_manifest_vec(&mut target.screenshots, &source.screenshots);
    merge_manifest_vec(&mut target.dom, &source.dom);
    merge_manifest_vec(&mut target.network, &source.network);
    merge_manifest_vec(&mut target.logs, &source.logs);
    merge_manifest_vec(&mut target.output, &source.output);
}

fn merge_manifest_vec(target: &mut Vec<String>, source: &[String]) {
    for item in source {
        if !target.contains(item) {
            target.push(item.clone());
        }
    }
}

pub(super) fn build_observed_action_inputs(inspections: &[PageInspection]) -> ObservedActionInputs {
    let mut page_urls = std::collections::HashSet::new();
    let mut link_urls = std::collections::HashSet::new();
    let mut search_forms_by_page = std::collections::HashMap::new();

    for inspection in inspections {
        if let Ok(url) = canonical_compare_url(&inspection.page.url) {
            page_urls.insert(url.clone());
            if !inspection.forms.is_empty() {
                search_forms_by_page.insert(url, inspection.forms.clone());
            }
        }

        for link in &inspection.links {
            if !is_supported_action_url(&link.url) {
                continue;
            }
            if let Ok(url) = canonical_compare_url(&link.url) {
                link_urls.insert(url);
            }
        }
    }

    ObservedActionInputs {
        page_urls,
        link_urls,
        search_forms_by_page,
    }
}

pub(super) fn validate_agent_action(
    action: &AgentAction,
    observed: &ObservedActionInputs,
) -> Option<String> {
    if !is_supported_action_url(action.url.trim()) {
        return Some("action url must use http/https scheme".into());
    }
    let canonical_url = canonical_compare_url(action.url.trim()).ok()?;
    let kind = action.kind.trim().to_ascii_lowercase();

    match kind.as_str() {
        "search" => {
            if !observed.page_urls.contains(&canonical_url) {
                return Some("search url must be an observed page url".into());
            }
            let Some(forms) = observed.search_forms_by_page.get(&canonical_url) else {
                return Some("search action page has no observed forms".into());
            };
            if forms.is_empty() {
                return Some("search action page has no observed forms".into());
            }

            if let Some(form_action) = action
                .form_action
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                if !is_supported_action_url(form_action) {
                    return Some("search formAction must use http/https scheme".into());
                }
                let canonical_form_action = canonical_compare_url(form_action).ok();
                let matched = forms.iter().any(|form| {
                    canonical_form_action.as_ref()
                        == canonical_compare_url(&form.action).ok().as_ref()
                        && action
                            .form_method
                            .as_deref()
                            .unwrap_or("get")
                            .eq_ignore_ascii_case(&form.method)
                        && form
                            .input_names
                            .iter()
                            .any(|name| name == action.query_param.as_deref().unwrap_or_default())
                });
                if !matched {
                    return Some("search form parameters do not match any observed form".into());
                }
            }
            None
        }
        _ => {
            if observed.page_urls.contains(&canonical_url)
                || observed.link_urls.contains(&canonical_url)
            {
                None
            } else {
                Some("action url must come from observed page urls or links".into())
            }
        }
    }
}

fn is_supported_action_url(url: &str) -> bool {
    let Ok(parsed) = Url::parse(url.trim()) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
}

pub(super) fn canonical_agent_key(url: &str) -> String {
    canonical_compare_url(url).unwrap_or_else(|_| url.to_string())
}

pub(super) fn normalize_agent_role(role: &str) -> String {
    match role.trim().to_ascii_lowercase().as_str() {
        "explore" => "explore".into(),
        "search" => "search".into(),
        "book" => "book".into(),
        "toc" => "toc".into(),
        "content" => "content".into(),
        "landing" => "landing".into(),
        _ => "inspect".into(),
    }
}

pub(super) fn infer_followup_toc_url(inspection: &PageInspection) -> Option<String> {
    inspection
        .links
        .iter()
        .filter_map(|link| {
            let label = link.label.trim();
            let url = link.url.trim();
            if url.is_empty() {
                return None;
            }
            let lower_url = url.to_ascii_lowercase();

            let mut score = 0i32;
            if lower_url.contains("/book/chapter/") {
                score += 120;
            }
            if lower_url.contains("/chapter/") {
                score += 90;
            }
            if label.contains("查看全部章节") {
                score += 110;
            }
            if label.contains("章节目录") {
                score += 90;
            }
            if label.contains("章节") {
                score += 40;
            }
            if label.contains("目录") {
                score += 25;
            }
            if lower_url.contains("catalog") {
                score += 10;
            }

            if lower_url.contains("/book/category/") || lower_url.contains("/ranking") {
                score -= 200;
            }

            if score > 0 {
                Some((score, url.to_string()))
            } else {
                None
            }
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
}

pub(super) fn infer_followup_toc_url_from_dom(
    paths: &AppPaths,
    task_id: &str,
    inspection: &PageInspection,
) -> Option<String> {
    let document = sampled_document(paths, task_id, &inspection.page).ok()?;
    let selectors = [
        "a[href*=\"/book/chapter/\"]",
        "a[href*=\"catalog\"]",
        "a[href*=\"chapter\"]",
        "a[href]",
    ];
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for (selector_index, selector_text) in selectors.iter().enumerate() {
        let Ok(selector) = Selector::parse(selector_text) else {
            continue;
        };
        for (element_index, anchor) in document.select(&selector).enumerate() {
            let href = anchor.value().attr("href").unwrap_or("").trim();
            if href.is_empty() {
                continue;
            }
            let url = normalize_url(&inspection.page.url, href).ok()?;
            if !seen.insert(url.clone()) {
                continue;
            }
            let label = anchor
                .text()
                .map(str::trim)
                .filter(|chunk| !chunk.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let lower_url = url.to_ascii_lowercase();
            let mut score = ((selectors.len() - selector_index) as i32) * 20 - element_index as i32;
            if lower_url.contains("/book/chapter/") {
                score += 120;
            }
            if lower_url.contains("/chapter/") {
                score += 90;
            }
            if lower_url.contains("catalog") {
                score += 50;
            }
            if label.contains("查看全部章节") {
                score += 110;
            }
            if label.contains("章节目录") {
                score += 90;
            }
            if label.contains("章节") {
                score += 40;
            }
            if label.contains("目录") {
                score += 25;
            }
            if lower_url.contains("/book/category/") || lower_url.contains("/ranking") {
                score -= 200;
            }
            if score > 0 {
                candidates.push((score, url));
            }
        }
    }

    candidates
        .into_iter()
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
}

pub(super) fn infer_followup_book_url(
    inspection: &PageInspection,
    keyword: Option<&str>,
) -> Option<String> {
    let expected = keyword
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());

    inspection
        .links
        .iter()
        .enumerate()
        .filter_map(|(index, link)| {
            let label = link.label.trim();
            let url = link.url.trim();
            if url.is_empty() {
                return None;
            }
            let lower_url = url.to_ascii_lowercase();

            let mut score = 0i32;
            if Regex::new(r"/book/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 220;
            }
            if Regex::new(r"/book/[^/]+/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score -= 320;
            }
            if lower_url.contains("/book/chapter/") {
                score -= 360;
            }
            if lower_url.contains("/book/category/")
                || lower_url.contains("/ranking")
                || lower_url.contains("/finish")
                || lower_url.contains("/book/so")
                || lower_url.contains("/search")
            {
                score -= 280;
            }
            if label.contains("阅读") {
                score += 30;
            }
            if label.contains("开始阅读") || label.contains("最新章节") || label.contains("第一章")
            {
                score -= 120;
            }
            if let Some(expected_keyword) = expected.as_deref() {
                score += keyword_match_score(label, expected_keyword);
            }

            if score > 0 {
                Some((score, Reverse(index), url.to_string()))
            } else {
                None
            }
        })
        .max_by_key(|(score, reverse_index, _)| (*score, *reverse_index))
        .map(|(_, _, url)| url)
}

pub(super) fn infer_followup_book_url_from_dom(
    paths: &AppPaths,
    task_id: &str,
    inspection: &PageInspection,
    keyword: Option<&str>,
) -> Option<String> {
    let document = sampled_document(paths, task_id, &inspection.page).ok()?;
    let selectors = [
        ".bookbox .bookname a[href]",
        ".bookinfo .bookname a[href]",
        ".librarylist .novelname[href]",
        ".result-title a[href]",
        ".book-title a[href]",
        "h2 a[href]",
        "h3 a[href]",
        "a.del_but[href]",
        "a[href]",
    ];
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for (selector_index, selector_text) in selectors.iter().enumerate() {
        let Ok(selector) = Selector::parse(selector_text) else {
            continue;
        };
        for (element_index, anchor) in document.select(&selector).enumerate() {
            let href = anchor.value().attr("href").unwrap_or("").trim();
            if href.is_empty() {
                continue;
            }
            let url = normalize_url(&inspection.page.url, href).ok()?;
            if !seen.insert(url.clone()) {
                continue;
            }
            let label = anchor
                .text()
                .map(str::trim)
                .filter(|chunk| !chunk.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let lower_url = url.to_ascii_lowercase();
            let mut score = ((selectors.len() - selector_index) as i32) * 20 - element_index as i32;
            if Regex::new(r"/book/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 220;
            }
            if Regex::new(r"/book/[^/]+/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score -= 320;
            }
            if lower_url.contains("/book/chapter/") {
                score -= 360;
            }
            if lower_url.contains("/book/category/")
                || lower_url.contains("/ranking")
                || lower_url.contains("/finish")
                || lower_url.contains("/book/so")
                || lower_url.contains("/search")
                || lower_url.contains("/history")
            {
                score -= 280;
            }
            if label.contains("阅读") {
                score += 20;
            }
            if label.contains("开始阅读") || label.contains("最新章节") || label.contains("第一章")
            {
                score -= 120;
            }
            if let Some(expected_keyword) = keyword {
                score += keyword_match_score(&label, expected_keyword);
            }

            if score > 0 {
                candidates.push((score, url));
            }
        }
    }

    candidates
        .into_iter()
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
}

pub(super) fn infer_followup_content_url(inspection: &PageInspection) -> Option<String> {
    inspection
        .links
        .iter()
        .enumerate()
        .filter_map(|link| {
            let (index, link) = link;
            let label = link.label.trim();
            let url = link.url.trim();
            if url.is_empty() {
                return None;
            }
            let lower_url = url.to_ascii_lowercase();

            let mut score = 0i32;
            if lower_url.contains("/book/") && lower_url.ends_with(".html") {
                score += 20;
            }
            if lower_url.contains("/book/chapter/") {
                score -= 300;
            }
            if lower_url.contains("/book/category/") || lower_url.contains("/ranking") {
                score -= 400;
            }
            if lower_url.contains("/book/so") || lower_url.contains("/search") {
                score -= 250;
            }
            if Regex::new(r"/book/[^/]+/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 250;
            }
            if Regex::new(r"/book/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 30;
            }
            if label.contains("开始阅读") {
                score += 240;
            }
            if label.contains("第一章") {
                score += 220;
            }
            if label.contains("最新章节") {
                score += 40;
            }
            if Regex::new(r"^第.{0,24}(章|节|卷|回)")
                .ok()
                .is_some_and(|pattern| pattern.is_match(label))
            {
                score += 180;
            }
            if label.contains("查看全部章节") || label.contains("章节") || label.contains("目录")
            {
                score -= 120;
            }

            if score > 0 {
                Some((score, Reverse(index), url.to_string()))
            } else {
                None
            }
        })
        .max_by_key(|(score, reverse_index, _)| (*score, *reverse_index))
        .map(|(_, _, url)| url)
}

pub(super) fn infer_followup_content_url_from_dom(
    paths: &AppPaths,
    task_id: &str,
    inspection: &PageInspection,
) -> Option<String> {
    let document = sampled_document(paths, task_id, &inspection.page).ok()?;
    let selectors = [
        ".directoryArea a[href]",
        ".dirlist a[href]",
        ".chapter-list a[href]",
        ".listmain a[href]",
        "#list a[href]",
        ".bookbox a[href]",
        ".bookinfo a[href]",
        ".delbutton a[href]",
        ".btn a[href]",
        "a[href]",
    ];
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for (selector_index, selector_text) in selectors.iter().enumerate() {
        let Ok(selector) = Selector::parse(selector_text) else {
            continue;
        };
        for (element_index, anchor) in document.select(&selector).enumerate() {
            let href = anchor.value().attr("href").unwrap_or("").trim();
            if href.is_empty() {
                continue;
            }
            let url = normalize_url(&inspection.page.url, href).ok()?;
            if !seen.insert(url.clone()) {
                continue;
            }
            let label = anchor
                .text()
                .map(str::trim)
                .filter(|chunk| !chunk.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let lower_url = url.to_ascii_lowercase();
            let mut score = ((selectors.len() - selector_index) as i32) * 20 - element_index as i32;
            if Regex::new(r"/book/[^/]+/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 260;
            }
            if Regex::new(r"/book/[^/]+\.html$")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&lower_url))
            {
                score += 20;
            }
            if lower_url.contains("/book/chapter/") {
                score -= 320;
            }
            if lower_url.contains("/book/category/")
                || lower_url.contains("/ranking")
                || lower_url.contains("/book/so")
                || lower_url.contains("/search")
                || lower_url.contains("/history")
            {
                score -= 300;
            }
            if label.contains("开始阅读") {
                score += 240;
            }
            if label.contains("第一章") {
                score += 220;
            }
            if label.contains("最新章节") {
                score += 60;
            }
            if Regex::new(r"^第.{0,24}(章|节|卷|回)")
                .ok()
                .is_some_and(|pattern| pattern.is_match(&label))
            {
                score += 180;
            }
            if label.contains("查看全部章节") || label.contains("章节") || label.contains("目录")
            {
                score -= 120;
            }

            if score > 0 {
                candidates.push((score, url));
            }
        }
    }

    candidates
        .into_iter()
        .max_by_key(|(score, _)| *score)
        .map(|(_, url)| url)
}

pub(super) fn agent_has_minimum_coverage(pages: &[crate::domain::PageSample]) -> bool {
    let has_search = pages
        .iter()
        .any(|page| page.role == "search" || page.role == "explore");
    let has_book = pages.iter().any(|page| page.role == "book");
    let has_toc = pages.iter().any(|page| page.role == "toc");
    let has_content = pages.iter().any(|page| page.role == "content");
    has_search && has_book && has_toc && has_content
}

pub(super) async fn request_agent_plan(
    paths: &AppPaths,
    task_id: &str,
    ai_settings: &StartAiSettings,
    api_key: &str,
    target_url: &str,
    test_keyword: Option<&str>,
    inspections: &[PageInspection],
) -> Result<AgentPlanResponse> {
    let prompt = build_agent_plan_prompt(target_url, test_keyword, inspections);
    let content = request_model_message(
        paths,
        task_id,
        "agent-plan",
        ai_settings,
        api_key,
        "Return only one JSON object with keys completed, summary and actions. Do not output markdown fences, comments, explanations, or trailing characters. actions must be an array of objects with kind, url, role and optional reason. Every action may include waitForSelector. For kind=search, also include formAction, formMethod and queryParam when a search form was observed. All urls must be copied exactly from observed pages or observed links/forms. Never normalize, simplify, or invent url variants. For kind=search, url must be the exact observed page url that contains the form, not the form action target. Base selectors on observed hints and avoid hardcoded site-specific guesses.",
        &prompt,
    )
    .await?;
    parse_agent_plan_response(&content)
}

fn build_agent_plan_prompt(
    target_url: &str,
    test_keyword: Option<&str>,
    inspections: &[PageInspection],
) -> String {
    let inspected = inspections
        .iter()
        .map(|inspection| {
            let links = inspection
                .links
                .iter()
                .take(8)
                .map(|link| format!("{} -> {}", link.label, link.url))
                .collect::<Vec<_>>()
                .join(" | ");
            let forms = inspection
                .forms
                .iter()
                .map(|form| {
                    format!(
                        "{} {} [{}]",
                        form.method,
                        form.action,
                        form.input_names.join(",")
                    )
                })
                .collect::<Vec<_>>()
                .join(" | ");
            let hints = format!(
                "bookList={} titleLink={} tocLink={} content={} next={}",
                inspection.hints.book_list_candidates.join(","),
                inspection.hints.title_link_candidates.join(","),
                inspection.hints.toc_link_candidates.join(","),
                inspection.hints.content_candidates.join(","),
                inspection.hints.next_page_candidates.join(",")
            );
            format!(
                "- role={} url={}\n  title={}\n  excerpt={}\n  links={}\n  forms={}\n  hints={}",
                inspection.page.role,
                inspection.page.url,
                inspection.title,
                inspection.excerpt,
                links,
                forms,
                hints
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are planning the next Playwright actions for a book-source agent.\nFollow this skill contract exactly:\n{}\n\nTarget URL: {}\nTest keyword: {}\nObserved pages:\n{}\n\nDecide the next actions needed to discover explore/search/book/toc/content coverage. Reuse observed links and forms whenever possible. Every action url must be copied exactly from Observed pages or links/forms above. Never rewrite '/x.html' into '/x' or fabricate new paths. For every action, provide the execution parameters worker should use directly. For search, url must be the exact observed page url that contains the form and must include formAction, formMethod and queryParam. For open actions, include waitForSelector when the target page needs a stable selector before capture.",
        PLAYWRIGHT_DISCOVERY_SKILL,
        target_url,
        test_keyword.unwrap_or(""),
        inspected
    )
}

pub(super) fn parse_agent_plan_response(content: &str) -> Result<AgentPlanResponse> {
    let trimmed = content
        .trim()
        .strip_prefix("```json")
        .or_else(|| content.trim().strip_prefix("```"))
        .unwrap_or(content.trim())
        .trim();
    let trimmed = trimmed.strip_suffix("```").unwrap_or(trimmed).trim();
    if let Ok(plan) = serde_json::from_str::<AgentPlanResponse>(trimmed) {
        return Ok(plan);
    }

    let mut stream = serde_json::Deserializer::from_str(trimmed).into_iter::<Value>();
    if let Some(value) = stream.next() {
        if let Ok(plan) = serde_json::from_value::<AgentPlanResponse>(value?) {
            return Ok(plan);
        }
    }

    if let Some(object_json) = extract_first_json_object(trimmed) {
        if let Ok(plan) = serde_json::from_str::<AgentPlanResponse>(&object_json) {
            return Ok(plan);
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
