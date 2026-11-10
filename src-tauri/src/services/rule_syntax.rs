use anyhow::{anyhow, Result};
use regex::Regex;
use scraper::{ElementRef, Html, Selector};
use serde_json::Value;

pub(super) fn normalize_export_value_for_reader(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for nested in map.values_mut() {
                normalize_export_value_for_reader(nested);
            }
        }
        Value::Array(items) => {
            for nested in items {
                normalize_export_value_for_reader(nested);
            }
        }
        Value::String(text) => {
            *text = normalize_reader_rule_syntax(text);
        }
        _ => {}
    }
}

pub(super) fn normalize_reader_rule_syntax(value: &str) -> String {
    let mut normalized = value.trim().to_string();
    if normalized.is_empty() {
        return normalized;
    }

    let attr_regex = Regex::new(r"@attr\[([^\]]+)\]").expect("valid attr regex");
    normalized = attr_regex.replace_all(&normalized, "@$1").into_owned();

    let double_colon_attr = Regex::new(r"::attr\(([^)]+)\)").expect("valid ::attr regex");
    normalized = double_colon_attr
        .replace_all(&normalized, "@$1")
        .into_owned();

    for (from, to) in [
        ("::textNodes", "@textNodes"),
        ("::ownText", "@ownText"),
        ("::html", "@html"),
        ("::all", "@all"),
        ("::text", "@text"),
    ] {
        normalized = normalized.replace(from, to);
    }

    if let Some(contains_index) = normalized.find(":contains(") {
        let needle_start = contains_index + ":contains(".len();
        let rest = &normalized[needle_start..];
        if let Some(end_index) = rest.find(')') {
            let raw_needle = rest[..end_index].trim();
            let needle = raw_needle.trim_matches('\'').trim_matches('"').trim();
            let suffix = rest[end_index + 1..].trim();
            if !needle.is_empty() {
                normalized = format!("text.{needle}{suffix}");
            }
        }
    }

    normalized
}

pub(super) fn normalize_css_rule_for_reader(value: &str) -> String {
    let value = value.trim();
    let rule = value
        .strip_prefix("@css:")
        .or_else(|| value.strip_prefix("@CSS:"))
        .unwrap_or(value)
        .trim_start();
    let selector = rule
        .split("##")
        .next()
        .unwrap_or(rule)
        .split('@')
        .next()
        .unwrap_or_default()
        .trim_end();
    if ["class.", "id.", "tag.", "text."]
        .iter()
        .any(|prefix| selector.starts_with(prefix))
        || !selector.chars().next().is_some_and(char::is_alphabetic)
        || selector.split_whitespace().count() < 2
        || Selector::parse(selector).is_err()
    {
        return value.to_string();
    }

    // Reader-Rust otherwise interprets tag-led descendant CSS as legacy class names.
    format!("@css::is({selector}){}", &rule[selector.len()..])
}

pub(super) fn selector(input: &str) -> Result<Selector> {
    let plain = input.split("##").next().unwrap_or(input).trim();
    let plain = plain
        .strip_prefix("@css:")
        .or_else(|| plain.strip_prefix("@CSS:"))
        .unwrap_or(plain)
        .trim();
    let selector = plain.split('@').next().unwrap_or_default().trim();

    Selector::parse(selector).map_err(|_| anyhow!("invalid selector: {selector}"))
}

pub(super) fn evaluate_on_document_safe(document: &Html, rule: &str) -> Result<String> {
    match evaluate_on_document(document, rule) {
        Ok(value) => Ok(value),
        Err(error) if error.to_string().contains("selector did not match") => Ok(String::new()),
        Err(error) => Err(error),
    }
}

pub(super) fn evaluate_on_element_safe(element: &ElementRef<'_>, rule: &str) -> Result<String> {
    match evaluate_on_element(element, rule) {
        Ok(value) => Ok(value),
        Err(error) if error.to_string().contains("selector did not match") => Ok(String::new()),
        Err(error) => Err(error),
    }
}

fn evaluate_on_document(document: &Html, rule: &str) -> Result<String> {
    if rule.trim().is_empty() {
        return Ok(String::new());
    }

    let parsed = parse_rule(rule);
    let element = select_first_from_document(document, parsed.selector)?;
    extract_value(element, &parsed.mode)
}

fn evaluate_on_element(element: &ElementRef<'_>, rule: &str) -> Result<String> {
    if rule.trim().is_empty() {
        return Ok(String::new());
    }

    let parsed = parse_rule(rule);
    if parsed.selector.is_empty() {
        return extract_value(*element, &parsed.mode);
    }

    // Reader field rules see the item's children, without the item or its ancestors.
    let mut document = Html::new_fragment();
    let mut pending = vec![(element.id(), document.tree.root().id())];
    while let Some((source_id, parent_id)) = pending.pop() {
        for child in element
            .tree()
            .get(source_id)
            .expect("source node exists")
            .children()
        {
            let id = document
                .tree
                .get_mut(parent_id)
                .expect("parent node exists")
                .append(child.value().clone())
                .id();
            pending.push((child.id(), id));
        }
    }
    let child = select_first_from_document(&document, parsed.selector)?;
    extract_value(child, &parsed.mode)
}

enum RuleMode {
    Text,
    Html,
    TextNodes,
    OwnText,
    Attr(String),
}

struct ParsedRule<'a> {
    selector: &'a str,
    mode: RuleMode,
}

fn parse_rule(rule: &str) -> ParsedRule<'_> {
    let base = rule.split("##").next().unwrap_or(rule).trim();
    let base = base
        .strip_prefix("@css:")
        .or_else(|| base.strip_prefix("@CSS:"))
        .unwrap_or(base)
        .trim();

    if let Some((selector, _)) = base.split_once("@textNodes") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::TextNodes,
        };
    }
    if let Some((selector, _)) = base.split_once("@ownText") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::OwnText,
        };
    }
    if let Some((selector, _)) = base.split_once("@html") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Html,
        };
    }
    if let Some((selector, _)) = base.split_once("@text") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Text,
        };
    }
    if let Some((selector, rest)) = base.split_once("@attr[") {
        return ParsedRule {
            selector: selector.trim(),
            mode: RuleMode::Attr(rest.trim_end_matches(']').to_string()),
        };
    }
    if let Some((selector, attr)) = base.rsplit_once('@') {
        let attr = attr.trim();
        if !attr.is_empty() {
            return ParsedRule {
                selector: selector.trim(),
                mode: match attr {
                    "text" => RuleMode::Text,
                    "html" => RuleMode::Html,
                    "textNodes" => RuleMode::TextNodes,
                    "ownText" => RuleMode::OwnText,
                    other => RuleMode::Attr(other.to_string()),
                },
            };
        }
    }

    ParsedRule {
        selector: base.trim(),
        mode: RuleMode::Text,
    }
}

fn extract_value(element: ElementRef<'_>, mode: &RuleMode) -> Result<String> {
    Ok(match mode {
        RuleMode::Text => element
            .text()
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        RuleMode::Html => element.inner_html(),
        RuleMode::TextNodes => element
            .text()
            .map(str::trim)
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        RuleMode::OwnText => element
            .children()
            .filter_map(|node| node.value().as_text())
            .map(|text| text.text.trim())
            .filter(|chunk| !chunk.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        RuleMode::Attr(attr) => element.value().attr(attr).unwrap_or_default().to_string(),
    })
}

fn select_first_from_document<'a>(
    document: &'a Html,
    selector_text: &str,
) -> Result<ElementRef<'a>> {
    let selector_text = selector_text.trim();
    if let Some(needle) = selector_text.strip_prefix("text.") {
        let selector = Selector::parse("*").map_err(|_| anyhow!("invalid selector"))?;
        return document
            .select(&selector)
            .find(|element| {
                element
                    .text()
                    .collect::<Vec<_>>()
                    .join(" ")
                    .contains(needle.trim())
            })
            .ok_or_else(|| anyhow!("selector did not match"));
    }

    let selector = Selector::parse(selector_text).map_err(|_| anyhow!("invalid selector"))?;
    document
        .select(&selector)
        .next()
        .ok_or_else(|| anyhow!("selector did not match"))
}
