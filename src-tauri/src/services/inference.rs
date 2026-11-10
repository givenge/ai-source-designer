use super::*;

pub(super) fn infer_candidate_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<SourceCandidate> {
    let explore = infer_explore_rule_from_sampled(paths, task_id, sampled)?;
    let search = infer_search_rule_from_sampled(paths, task_id, sampled)?;
    let book_info = infer_book_info_rule_from_sampled(paths, task_id, sampled)?;
    let toc = infer_toc_rule_from_sampled(paths, task_id, sampled)?;
    let content = infer_content_rule_from_sampled(paths, task_id, sampled)?;

    let mut evidence_refs = Vec::new();
    for role in ["explore", "search", "book", "toc", "content"] {
        evidence_refs.extend(sampled_page_evidence(role, sampled));
    }

    let explore_url = infer_explore_url_from_sampled(paths, task_id, sampled)?;

    Ok(SourceCandidate {
        strategy: "sampled-dom-inference".into(),
        confidence: 0.78,
        evidence_refs,
        repair_history: Vec::new(),
        book_source: BookSource {
            book_source_name: sampled.site_label.clone(),
            book_source_url: sampled.base_url.clone(),
            enabled: Some(true),
            enabled_explore: explore_url.as_ref().map(|_| true),
            explore_url,
            rule_explore: explore,
            search_url: infer_search_url_from_sampled(paths, task_id, sampled)?,
            rule_search: Some(search),
            rule_book_info: Some(book_info),
            rule_toc: Some(toc),
            rule_content: Some(content),
            ..Default::default()
        },
    })
}

fn infer_explore_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<Option<ExploreRule>> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "explore") else {
        return Ok(None);
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(Some(ExploreRule {
        book_list: first_matching_selector(
            &document,
            &[
                ".hot_sale li",
                ".librarylist li",
                ".result-list .result-item",
                ".novellist .item",
                ".book-list li",
                ".bookbox",
                ".item",
            ],
        ),
        name: first_matching_element_rule(
            &document,
            &[
                ".title@text",
                ".bookname a@text",
                ".novelname@text",
                ".result-title a@text",
                ".book-title a@text",
                "h3 a@text",
                "a[href]@text",
            ],
        ),
        author: first_matching_element_rule(
            &document,
            &[
                ".author@text",
                ".writer@text",
                ".novelauthor@text",
                ".info@text",
                "p@text",
            ],
        ),
        intro: first_matching_element_rule(
            &document,
            &[
                ".review@text",
                ".intro@text",
                ".description@text",
                ".desc@text",
                ".bookintro@text",
                "p@text",
            ],
        ),
        book_url: first_matching_element_rule(
            &document,
            &[
                ".title a@href",
                ".bookname a@href",
                ".novelname@href",
                ".result-title a@href",
                ".book-title a@href",
                "h3 a@href",
                "a[href]@href",
            ],
        ),
        cover_url: first_matching_element_rule(&document, &["img@src"]),
        ..Default::default()
    }))
}

fn infer_explore_url_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<Option<String>> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "explore") else {
        return Ok(None);
    };
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    let document = Html::parse_document(&html);
    let anchor_selector = Selector::parse("a[href]").map_err(|_| anyhow!("invalid selector"))?;
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let pagination = explore::next_page_url(&page.url, &document)
        .and_then(|next| explore::pagination_template(&page.url, &next));
    let book_items = infer_explore_rule_from_sampled(paths, task_id, sampled)?
        .and_then(|rule| rule.book_list)
        .and_then(|rule| selector(&rule).ok())
        .map(|selector| {
            document
                .select(&selector)
                .map(|item| item.id())
                .collect::<std::collections::HashSet<_>>()
        })
        .unwrap_or_default();
    let base_domain = Url::parse(&sampled.base_url)
        .ok()
        .and_then(|base| base.domain().map(str::to_string));

    for anchor in document.select(&anchor_selector) {
        if anchor
            .ancestors()
            .any(|node| book_items.contains(&node.id()))
        {
            continue;
        }
        let label = anchor
            .text()
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let href = anchor.value().attr("href").unwrap_or("").trim();
        if label.is_empty() || href.is_empty() {
            continue;
        }
        if matches!(label.as_str(), "首页" | "阅读历史" | "搜索" | "搜 索") {
            continue;
        }

        let lower_href = href.to_ascii_lowercase();
        let lower_label = label.to_ascii_lowercase();
        let is_discovery_link = lower_href.contains("/book/category/")
            || lower_href.contains("/book/ranking")
            || lower_href.contains("/book/finish")
            || lower_href.contains("/rank")
            || lower_label.contains("排行")
            || lower_label.contains("分类")
            || lower_label.contains("玄幻")
            || lower_label.contains("仙侠")
            || lower_label.contains("都市")
            || lower_label.contains("历史")
            || lower_label.contains("科幻")
            || lower_label.contains("言情")
            || lower_label.contains("武侠")
            || lower_label.contains("游戏")
            || lower_label.contains("连载")
            || lower_label.contains("完结")
            || lower_label.contains("奇幻")
            || lower_label.contains("军事")
            || lower_label.contains("体育")
            || lower_label.contains("悬疑")
            || lower_label.contains("轻小")
            || lower_label.contains("轻小说")
            || lower_label.contains("穿越");
        if !is_discovery_link {
            continue;
        }

        let normalized = normalize_url(&page.url, href)?;
        if !is_site_url(&page.url, &normalized) {
            continue;
        }
        let rendered_url = Url::parse(&normalized)
            .ok()
            .and_then(|url| {
                if url.scheme().starts_with("http")
                    && url.domain().map(str::to_string) == base_domain
                {
                    Some(format!(
                        "{}{}",
                        url.path(),
                        url.query()
                            .map(|query| format!("?{query}"))
                            .unwrap_or_default()
                    ))
                } else {
                    None
                }
            })
            .unwrap_or(normalized.clone());

        if !seen.insert(normalized.clone()) {
            continue;
        }
        let rendered_url = pagination
            .as_ref()
            .and_then(|template| explore::category_template(&page.url, template, &normalized))
            .unwrap_or(rendered_url);

        entries.push(json!({
            "title": label,
            "url": rendered_url,
            "style": {
                "layout_flexBasisPercent": if entries.is_empty() { 1.0 } else { 0.25 },
                "layout_flexGrow": 1
            }
        }));
    }

    if !entries.is_empty() {
        return Ok(Some(serde_json::to_string(&entries)?));
    }

    Ok(Some(pagination.unwrap_or_else(|| page.url.clone())))
}

pub(super) fn infer_search_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<SearchRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "search") else {
        return Ok(SearchRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(SearchRule {
        book_list: first_matching_selector(
            &document,
            &[
                ".librarylist li",
                ".result-list .result-item",
                ".novellist .item",
                ".search-result li",
                ".book-list li",
                ".bookbox",
                ".item",
            ],
        ),
        name: first_matching_element_rule(
            &document,
            &[
                ".bookname a@text",
                ".bookinfo .bookname a@text",
                ".novelname@text",
                ".result-title a@text",
                ".book-title a@text",
                "h3 a@text",
                "a[href]@text",
            ],
        ),
        author: first_matching_element_rule(
            &document,
            &[
                ".info span:nth-child(2) a@text",
                ".info span:nth-child(2)@text",
                ".info a[href*=\"/author/\"]@text",
                ".author@text",
                ".writer@text",
                ".novelauthor@text",
                ".info@text",
                "p@text",
            ],
        ),
        cover_url: first_matching_element_rule(
            &document,
            &[".pt-ll-l img@src", ".bookbox img@src", "img@src"],
        ),
        intro: first_matching_element_rule(
            &document,
            &[
                ".intro@text",
                ".description@text",
                ".desc@text",
                ".bookintro@text",
                "p@text",
            ],
        ),
        book_url: first_matching_element_rule(
            &document,
            &[
                ".bookname a@href",
                ".bookinfo .bookname a@href",
                ".novelname@href",
                ".result-title a@href",
                ".book-title a@href",
                "h3 a@href",
                "a[href]@href",
            ],
        ),
        ..Default::default()
    })
}

pub(super) fn infer_book_info_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<BookInfoRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "book") else {
        return Ok(BookInfoRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    let has_inline_toc = first_matching_selector(
        &document,
        &[
            ".recommend .directoryArea p",
            ".directoryArea p",
            ".fulldir .dirlist li",
            ".dirlist li",
            ".chapter-list li",
            ".listmain dd",
            "#list dd",
            ".booklist li",
        ],
    )
    .is_some();

    Ok(BookInfoRule {
        name: first_matching_document_rule(
            &document,
            &[
                ".channelHeader .title@text",
                "meta[property=\"og:novel:book_name\"]@content",
                "meta[property=\"og:title\"]@content",
                "#info h1@text",
                ".bookinfo h1@text",
                ".novelinfo h1@text",
                "h1@text",
            ],
        ),
        author: first_matching_document_rule(
            &document,
            &[
                ".synopsisArea_detail .author@text",
                "meta[property=\"og:novel:author\"]@content",
                ".novelinfo-l li a@text",
                ".meta .author@text",
                "#info p a@text",
                ".author a@text",
                ".author@text",
            ],
        ),
        cover_url: first_matching_document_rule(
            &document,
            &[
                ".novelinfo-r img@src",
                "meta[property=\"og:image\"]@content",
                ".book-cover img@src",
                ".cover img@src",
                "img@src",
            ],
        ),
        intro: first_matching_document_rule(
            &document,
            &[
                ".review@text",
                "meta[property=\"og:description\"]@content",
                ".novelintro@text",
                "#intro@text",
                ".intro@text",
                ".bookintro@text",
                ".desc@text",
            ],
        ),
        toc_url: if has_inline_toc {
            None
        } else {
            first_matching_document_rule(
                &document,
                &[
                    "a[href*=\"/book/chapter/\"]@href",
                    "dd a[href*=\"/book/chapter/\"]@href",
                    ".bookchapter@href",
                    ".toc-link@href",
                    "a[href*=\"chapter\"]@href",
                    "a[href*=\"list\"]@href",
                    "a[href*=\"catalog\"]@href",
                ],
            )
        },
        ..Default::default()
    })
}

pub(super) fn infer_toc_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<TocRule> {
    let page = sampled
        .pages
        .iter()
        .find(|page| page.role == "toc")
        .or_else(|| sampled.pages.iter().find(|page| page.role == "book"));
    let Some(page) = page else {
        return Ok(TocRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;

    Ok(TocRule {
        chapter_list: first_matching_selector(
            &document,
            &[
                ".book.chapterlist #allchapter dd",
                ".chapterlist #allchapter dd",
                "#allchapter dd",
                ".book.chapterlist dd",
                ".chapterlist dd",
                ".recommend .directoryArea p",
                ".directoryArea p",
                ".fulldir .dirlist li",
                ".dirlist li",
                ".chapter-list li",
                ".listmain dd",
                "#list dd",
                "#list li",
                ".booklist li",
            ],
        ),
        chapter_name: Some("a@text".into()),
        chapter_url: Some("a@href".into()),
        next_toc_url: infer_next_toc_rule(&document),
        ..Default::default()
    })
}

fn infer_next_toc_rule(document: &Html) -> Option<String> {
    let candidates = [
        "#indexselect option@value",
        ".index-container select option@value",
        "select#page_select option@value",
        "select option@value",
        ".pagination .next@href",
        "a.next@href",
        "a[rel=\"next\"]@href",
        "text.下一页@href",
        "text.下页@href",
    ];

    candidates.iter().find_map(|rule| {
        let Ok(value) = evaluate_on_document_safe(document, rule) else {
            return None;
        };
        let value = value.trim();
        if value.is_empty() || value.starts_with("javascript:") || value == "javascript:void(0);" {
            return None;
        }
        Some((*rule).to_string())
    })
}

pub(super) fn infer_content_rule_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<ContentRule> {
    let Some(page) = sampled.pages.iter().find(|page| page.role == "content") else {
        return Ok(ContentRule::default());
    };
    let document = sampled_document(paths, task_id, page)?;
    let content_rule = best_content_rule_from_document(&document);

    Ok(ContentRule {
        content: content_rule,
        next_content_url: first_matching_document_rule(
            &document,
            &[
                "#pt_next@href",
                ".Readpage #pt_next@href",
                ".next a@href",
                "a.next@href",
                "a#next_url@href",
            ],
        ),
        replace_regex: Some(
            "##\\n?(首页|目录|书签|报错|关灯|字体\\+|字体-|上一章|下一章)\\n?".into(),
        ),
        ..Default::default()
    })
}

fn best_content_rule_from_document(document: &Html) -> Option<String> {
    let candidates = [
        "#chaptercontent@textNodes",
        "#content@textNodes",
        ".content@textNodes",
        ".txt@textNodes",
        ".read-content@textNodes",
        ".article-content@textNodes",
    ];

    let mut best: Option<(i64, String)> = None;
    for candidate in candidates {
        let Ok(extracted) = evaluate_on_document_safe(document, candidate) else {
            continue;
        };
        let score = content_candidate_score(candidate, &extracted);
        if score <= 0 {
            continue;
        }
        if best.as_ref().is_none_or(|current| score > current.0) {
            best = Some((score, candidate.to_string()));
        }
    }

    best.map(|(_, rule)| rule).or_else(|| {
        first_matching_document_rule(
            document,
            &[
                "#chaptercontent@textNodes",
                "#content@textNodes",
                ".content@textNodes",
                ".txt@textNodes",
                ".read-content@textNodes",
                ".article-content@textNodes",
            ],
        )
    })
}

fn content_candidate_score(rule: &str, extracted: &str) -> i64 {
    let normalized = extracted.replace('\u{a0}', " ");
    let length = normalized.chars().count() as i64;
    if length < 80 {
        return -1;
    }

    let contamination_keywords = ["首页", "目录", "书签", "报错", "关灯", "字体+", "字体-"];
    let hits = contamination_keywords
        .iter()
        .map(|keyword| normalized.matches(keyword).count() as i64)
        .sum::<i64>();

    let heading_penalty = normalized
        .lines()
        .take(16)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .filter(|line| {
            contamination_keywords
                .iter()
                .any(|keyword| line.contains(keyword))
        })
        .count() as i64
        * 120;

    let extractor_bonus = if rule.ends_with("@textNodes") { 40 } else { 0 };

    length - hits * 40 - heading_penalty + extractor_bonus
}

pub(super) fn sampled_document(
    paths: &AppPaths,
    task_id: &str,
    page: &crate::domain::PageSample,
) -> Result<Html> {
    let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
    Ok(Html::parse_document(&html))
}

fn first_matching_selector(document: &Html, selectors: &[&str]) -> Option<String> {
    selectors.iter().find_map(|candidate| {
        let Ok(selector) = Selector::parse(candidate) else {
            return None;
        };
        document
            .select(&selector)
            .next()
            .map(|_| (*candidate).to_string())
    })
}

pub(super) fn first_matching_document_rule(document: &Html, rules: &[&str]) -> Option<String> {
    rules.iter().find_map(|rule| {
        evaluate_on_document_safe(document, rule)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .map(|_| (*rule).to_string())
    })
}

fn first_matching_element_rule(document: &Html, rules: &[&str]) -> Option<String> {
    let containers = [
        ".librarylist li",
        ".result-list .result-item",
        ".novellist .item",
        ".search-result li",
        ".book-list li",
        ".bookbox",
        ".item",
    ];

    for container_rule in containers {
        let Ok(container_selector) = Selector::parse(container_rule) else {
            continue;
        };
        let Some(element) = document.select(&container_selector).next() else {
            continue;
        };
        for rule in rules {
            if evaluate_on_element_safe(&element, rule)
                .ok()
                .filter(|value| !value.trim().is_empty())
                .is_some()
            {
                return Some((*rule).to_string());
            }
        }
    }

    None
}

pub(super) fn infer_search_url_from_sampled(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
) -> Result<Option<String>> {
    let form_selector = Selector::parse("form[action]").map_err(|_| anyhow!("invalid selector"))?;
    let input_selector = Selector::parse("input[name]").map_err(|_| anyhow!("invalid selector"))?;

    if let Ok(search_page) = sampled_page(sampled, "search") {
        let search_url = search_page.url.trim();
        if !search_url.is_empty() {
            let candidate = Regex::new(r"/book/so/[^/?#]+\.html$")
                .ok()
                .and_then(|pattern| pattern.find(search_url))
                .map(|matched| {
                    let mut rendered = search_url.to_string();
                    rendered.replace_range(matched.range(), "/book/so/{key}.html");
                    rendered
                });
            if candidate.is_some() {
                return Ok(candidate);
            }
        }
    }

    for role in ["landing", "search"] {
        let Ok(page) = sampled_page(sampled, role) else {
            continue;
        };
        let html = fs::read_to_string(paths.task_path(task_id, &page.html_path))?;
        let document = Html::parse_document(&html);

        for form in document.select(&form_selector) {
            let Some(action) = form.value().attr("action") else {
                continue;
            };
            let action = action.trim();
            if action.is_empty() {
                continue;
            }

            let query_name = form.select(&input_selector).find_map(|input| {
                let input_type = input
                    .value()
                    .attr("type")
                    .unwrap_or("text")
                    .trim()
                    .to_ascii_lowercase();
                let name = input.value().attr("name").unwrap_or("").trim().to_string();
                let lower_name = name.to_ascii_lowercase();
                if name.is_empty() {
                    return None;
                }
                if input_type == "text"
                    || input_type == "search"
                    || lower_name.contains("key")
                    || lower_name.contains("search")
                    || lower_name.contains("query")
                    || lower_name.contains("wd")
                {
                    return Some(name);
                }
                None
            });

            let Some(query_name) = query_name else {
                continue;
            };

            let resolved = normalize_url(&page.url, action)?;
            let inline_submit = form
                .value()
                .attr("onsubmit")
                .unwrap_or("")
                .to_ascii_lowercase();
            if inline_submit.contains("handlesearch")
                && !resolved.contains('?')
                && !resolved.to_ascii_lowercase().ends_with(".html")
            {
                return Ok(Some(format!("{resolved}/{{key}}.html")));
            }
            let separator = if resolved.contains('?') { "&" } else { "?" };
            return Ok(Some(format!(
                "{resolved}{separator}{query_name}={{key}}&page={{page}}"
            )));
        }

        if role == "search" {
            let Ok(mut url) = Url::parse(&page.url) else {
                continue;
            };
            let key_name = url.query_pairs().find_map(|(name, _)| {
                let lower = name.to_ascii_lowercase();
                if lower.contains("key")
                    || lower.contains("search")
                    || lower.contains("query")
                    || lower == "wd"
                {
                    Some(name.into_owned())
                } else {
                    None
                }
            });

            let Some(key_name) = key_name else {
                continue;
            };

            let existing_page = url
                .query_pairs()
                .any(|(name, _)| name.eq_ignore_ascii_case("page"));
            let mut pairs = vec![(key_name, "{key}".to_string())];
            if existing_page {
                pairs.push(("page".into(), "{page}".into()));
            }

            url.query_pairs_mut().clear().extend_pairs(pairs);
            let rendered = url
                .to_string()
                .replace("%7Bkey%7D", "{key}")
                .replace("%7Bpage%7D", "{page}");
            return Ok(Some(rendered));
        }
    }

    Ok(None)
}
