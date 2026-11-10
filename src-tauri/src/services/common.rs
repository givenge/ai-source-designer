use super::*;

pub(super) fn is_site_url(target_url: &str, url: &str) -> bool {
    if target_url.starts_with("fixture://") {
        return true;
    }
    let (Ok(target), Ok(candidate)) = (Url::parse(target_url), Url::parse(url)) else {
        return false;
    };
    if !matches!(candidate.scheme(), "http" | "https") {
        return false;
    }
    let (Some(target_host), Some(candidate_host)) = (target.host_str(), candidate.host_str())
    else {
        return false;
    };
    let target_host = target_host.strip_prefix("www.").unwrap_or(target_host);
    let candidate_host = candidate_host
        .strip_prefix("www.")
        .unwrap_or(candidate_host);
    // shortcut: 范围限于目标域名及子域，跨域书籍托管需经过单独证据验证再扩展。
    (target_host == candidate_host
        || (target.domain().is_some() && candidate_host.ends_with(&format!(".{target_host}"))))
        && (target.port().is_none() || target.port() == candidate.port())
}

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
