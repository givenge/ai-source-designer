use super::*;

pub(super) fn normalize_url(base: &str, maybe_relative: &str) -> Result<String> {
    if maybe_relative.trim().is_empty() {
        return Ok(String::new());
    }

    if maybe_relative.starts_with("http://") || maybe_relative.starts_with("https://") {
        return Ok(maybe_relative.to_string());
    }

    Ok(Url::parse(base)?.join(maybe_relative)?.to_string())
}

pub(super) fn canonical_compare_url(url: &str) -> Result<String> {
    let mut parsed = Url::parse(url)?;
    if parsed.path() != "/" {
        let trimmed = parsed.path().trim_end_matches('/').to_string();
        parsed.set_path(if trimmed.is_empty() { "/" } else { &trimmed });
    }
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

pub(super) fn keyword_match_score(title: &str, keyword: &str) -> i32 {
    let normalized_title = normalize_match_text(title);
    let normalized_keyword = normalize_match_text(keyword);

    if normalized_keyword.is_empty() || normalized_title.is_empty() {
        return 0;
    }

    if normalized_title == normalized_keyword {
        100
    } else if normalized_title.starts_with(&normalized_keyword) {
        80
    } else if normalized_title.contains(&normalized_keyword) {
        60
    } else {
        0
    }
}

fn normalize_match_text(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

pub(super) fn sampled_page<'a>(
    sampled: &'a SamplingResult,
    role: &str,
) -> Result<&'a crate::domain::PageSample> {
    sampled
        .pages
        .iter()
        .find(|page| page.role == role)
        .ok_or_else(|| anyhow!("missing sampled page for role {role}"))
}
