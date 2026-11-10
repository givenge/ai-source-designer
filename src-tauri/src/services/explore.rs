use super::*;

pub(super) fn next_page_url(base: &str, document: &Html) -> Option<String> {
    document
        .select(&Selector::parse("a[href]").ok()?)
        .find_map(|link| {
            let label = link.text().collect::<String>().trim().to_ascii_lowercase();
            let rel = link.value().attr("rel").unwrap_or_default();
            if !rel
                .split_whitespace()
                .any(|word| word.eq_ignore_ascii_case("next"))
                && !matches!(
                    label.as_str(),
                    "下一页" | "下页" | "next" | "next page" | "›"
                )
            {
                return None;
            }
            let url = normalize_url(base, link.value().attr("href")?).ok()?;
            (is_site_url(base, &url) && canonical_agent_key(base) != canonical_agent_key(&url))
                .then_some(url)
        })
}

pub(super) fn pagination_template(first: &str, next: &str) -> Option<String> {
    let first = Url::parse(first).ok()?;
    let next = Url::parse(next).ok()?;
    if first.origin() != next.origin() {
        return None;
    }
    let first = first.as_str();
    let next = next.as_str();
    let prefix = first
        .bytes()
        .zip(next.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    let suffix = first[prefix..]
        .bytes()
        .rev()
        .zip(next[prefix..].bytes().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let old = &first[prefix..first.len() - suffix];
    let new = &next[prefix..next.len() - suffix];
    let numbers = Regex::new(r"\d+").ok()?;
    let mut matches = numbers.find_iter(new);
    let number = matches.next()?;
    if number.as_str() != "2" || matches.next().is_some() {
        return None;
    }
    let start = prefix + number.start();
    let end = prefix + number.end();
    if start > 0
        && (next.as_bytes()[start - 1].is_ascii_digit() || next.as_bytes()[start - 1] == b'%')
        || end < next.len() && next.as_bytes()[end].is_ascii_digit()
    {
        return None;
    }
    let expression = if old == "1" && new == "2" {
        "{{page}}".to_string()
    } else {
        let before = serde_json::to_string(&new[..number.start()]).ok()?;
        let after = &new[number.end()..];
        let after = if after.is_empty() {
            String::new()
        } else {
            format!(" + {}", serde_json::to_string(after).ok()?)
        };
        format!(
            "{{{{page == 1 ? {} : {before} + page{after}}}}}",
            serde_json::to_string(old).ok()?
        )
    };
    Some(format!(
        "{}{expression}{}",
        &first[..prefix],
        &first[first.len() - suffix..]
    ))
}

pub(super) fn category_template(first: &str, template: &str, category: &str) -> Option<String> {
    let first = Url::parse(first).ok()?;
    let category = Url::parse(category).ok()?;
    if first == category {
        return Some(template.to_string());
    }
    // Infer sibling categories only when their URL structure matches the sampled category.
    if first.origin() != category.origin()
        || first.path_segments()?.count() != category.path_segments()?.count()
        || first
            .query_pairs()
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>()
            != category
                .query_pairs()
                .map(|(key, _)| key.to_string())
                .collect::<Vec<_>>()
    {
        return None;
    }
    let start = template.find("{{")?;
    let end = start + template[start..].find("}}")? + 2;
    let tail = &template[end..];
    let old = first
        .as_str()
        .get(start..first.as_str().len().checked_sub(tail.len())?)?;
    let first_tail = format!("{old}{tail}");
    if !category.as_str().ends_with(&first_tail) {
        return None;
    }
    Some(format!(
        "{}{}{}",
        &category.as_str()[..category.as_str().len() - first_tail.len()],
        &template[start..end],
        tail
    ))
}

fn literal(value: &str) -> Option<String> {
    serde_json::from_str(value).ok().or_else(|| {
        value
            .strip_prefix('\'')?
            .strip_suffix('\'')
            .map(str::to_string)
    })
}

pub(super) fn page_url(rule: &str, base: &str, page: i32) -> Option<String> {
    // shortcut: Only page placeholders and generated ternaries run locally; other JS needs Reader verification.
    let templates = Regex::new(r"(?s)\{\{(.*?)\}\}").ok()?;
    let conditional = Regex::new(
        r#"(?x)^\s*page\s*={2,3}\s*1\s*\?\s*
        ((?:"(?:[^"\\]|\\.)*")|(?:'[^']*'))\s*:\s*
        ((?:"(?:[^"\\]|\\.)*")|(?:'[^']*'))\s*\+\s*page
        (?:\s*\+\s*((?:"(?:[^"\\]|\\.)*")|(?:'[^']*')))?\s*$"#,
    )
    .ok()?;
    let mut url = rule.to_string();
    for entry in templates
        .captures_iter(rule)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let expression = entry[1].trim();
        let value = if expression == "page" {
            page.to_string()
        } else {
            let parts = conditional.captures(expression)?;
            if page == 1 {
                literal(&parts[1])?
            } else {
                format!(
                    "{}{}{}",
                    literal(&parts[2])?,
                    page,
                    parts
                        .get(3)
                        .map(|part| literal(part.as_str()))
                        .unwrap_or(Some(String::new()))?
                )
            }
        };
        let matched = entry.get(0)?;
        url.replace_range(matched.range(), &value);
    }
    let url = url
        .replace("${page}", &page.to_string())
        .replace("{page}", &page.to_string());
    if url.contains(['{', '}']) {
        return None;
    }
    normalize_url(base, &url).ok()
}

pub(super) fn urls(value: &str) -> Vec<String> {
    if let Ok(Value::Array(entries)) = serde_json::from_str::<Value>(value) {
        entries
            .iter()
            .filter_map(|entry| entry.get("url").and_then(Value::as_str))
            .filter(|url| !url.trim().is_empty())
            .map(str::to_string)
            .collect()
    } else {
        vec![value
            .split_once("::")
            .filter(|_| {
                !Url::parse(value).is_ok_and(|url| matches!(url.scheme(), "http" | "https"))
            })
            .map(|(_, url)| url)
            .unwrap_or(value)
            .to_string()]
    }
}

pub(super) fn complete_pagination(
    paths: &AppPaths,
    task_id: &str,
    sampled: &SamplingResult,
    source: &mut BookSource,
) -> Result<()> {
    let Ok(page) = sampled_page(sampled, "explore") else {
        return Ok(());
    };
    let document = sampled_document(paths, task_id, page)?;
    let Some(next) = next_page_url(&page.url, &document) else {
        return Ok(());
    };
    let Some(template) = pagination_template(&page.url, &next) else {
        return Ok(());
    };
    let Some(raw) = source.explore_url.as_mut() else {
        return Ok(());
    };
    if let Ok(Value::Array(mut entries)) = serde_json::from_str::<Value>(raw) {
        for entry in &mut entries {
            let Some(url) = entry.get("url").and_then(Value::as_str) else {
                continue;
            };
            if url.is_empty() || url.contains(['{', '}', '<']) {
                continue;
            }
            if let Some(url) = page_url(url, &source.book_source_url, 1)
                .and_then(|url| category_template(&page.url, &template, &url))
            {
                entry["url"] = Value::String(url);
            }
        }
        *raw = serde_json::to_string(&entries)?;
    } else if !raw.contains(['{', '}', '<']) {
        let (label, url) = raw
            .split_once("::")
            .filter(|_| !Url::parse(raw).is_ok_and(|url| matches!(url.scheme(), "http" | "https")))
            .map(|(label, url)| (Some(label), url))
            .unwrap_or((None, raw));
        if let Some(url) = page_url(url, &source.book_source_url, 1)
            .and_then(|url| category_template(&page.url, &template, &url))
        {
            *raw = label.map(|label| format!("{label}::{url}")).unwrap_or(url);
        }
    }
    Ok(())
}

pub(super) fn capture_next_page(
    state: &AppState,
    task: &mut TaskRecord,
    sampled: &mut SamplingResult,
) -> Result<()> {
    let Ok(page) = sampled_page(sampled, "explore") else {
        return Ok(());
    };
    let document = sampled_document(&state.paths, &task.id, page)?;
    let Some(next) = next_page_url(&page.url, &document) else {
        return Ok(());
    };
    if sampled.pages.iter().any(|page| {
        page.role == "explore-next" && canonical_agent_key(&page.url) == canonical_agent_key(&next)
    }) {
        return Ok(());
    }
    ensure_not_cancelled(state, &task.id)?;
    append_log(
        task,
        TaskStatus::Sampling,
        format!("采样书海下一页，核对分页是否返回新书籍：{next}"),
    );
    persist_and_flush(state, task)?;
    match worker::inspect_page(state, &task.id, &next, "explore-next", None) {
        Ok(inspection) => {
            let final_url = inspection.page.url.clone();
            if !merge_inspection(
                &task.target_url,
                &mut sampled.pages,
                &mut sampled.network_samples,
                &mut sampled.manifest,
                &mut Vec::new(),
                inspection,
            ) {
                append_log(
                    task,
                    TaskStatus::Sampling,
                    format!("书海下一页跳转到站外，已保留证据但不计入验证：{next} -> {final_url}"),
                );
            }
            task.artifact_manifest = Some(sampled.manifest.clone());
        }
        Err(error) => {
            append_log(
                task,
                TaskStatus::Sampling,
                format!("书海下一页采样失败：{error}"),
            );
        }
    }
    ensure_not_cancelled(state, &task.id)?;
    persist_and_flush(state, task)
}
