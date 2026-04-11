use super::*;

pub(super) fn merge_candidates(
    mut inferred: SourceCandidate,
    model: SourceCandidate,
) -> SourceCandidate {
    inferred.strategy = format!("{} + {}", inferred.strategy, model.strategy);
    inferred.confidence = inferred.confidence.max(model.confidence);
    inferred.evidence_refs.extend(model.evidence_refs);
    inferred.repair_history.extend(model.repair_history);

    let model_source = model.book_source;
    if inferred.book_source.book_source_name.trim().is_empty() {
        inferred.book_source.book_source_name = model_source.book_source_name;
    }
    if inferred.book_source.book_source_url.trim().is_empty() {
        inferred.book_source.book_source_url = model_source.book_source_url;
    }
    fill_option(
        &mut inferred.book_source.book_source_group,
        model_source.book_source_group,
    );
    fill_i64_option(
        &mut inferred.book_source.book_source_type,
        model_source.book_source_type,
    );
    fill_option(
        &mut inferred.book_source.book_url_pattern,
        model_source.book_url_pattern,
    );
    fill_i64_option(
        &mut inferred.book_source.custom_order,
        model_source.custom_order,
    );
    fill_bool_option(&mut inferred.book_source.enabled, model_source.enabled);
    fill_bool_option(
        &mut inferred.book_source.enabled_explore,
        model_source.enabled_explore,
    );
    fill_bool_option(
        &mut inferred.book_source.enabled_cookie_jar,
        model_source.enabled_cookie_jar,
    );
    fill_option(&mut inferred.book_source.js_lib, model_source.js_lib);
    fill_option(
        &mut inferred.book_source.concurrent_rate,
        model_source.concurrent_rate,
    );
    fill_json_option(&mut inferred.book_source.header, model_source.header);
    fill_option(&mut inferred.book_source.login_url, model_source.login_url);
    fill_json_option(&mut inferred.book_source.login_ui, model_source.login_ui);
    fill_option(
        &mut inferred.book_source.login_check_js,
        model_source.login_check_js,
    );
    fill_option(
        &mut inferred.book_source.cover_decode_js,
        model_source.cover_decode_js,
    );
    fill_i64_option(
        &mut inferred.book_source.last_update_time,
        model_source.last_update_time,
    );
    fill_i64_option(&mut inferred.book_source.weight, model_source.weight);
    fill_option(
        &mut inferred.book_source.explore_url,
        model_source.explore_url,
    );
    fill_json_option(
        &mut inferred.book_source.explore_screen,
        model_source.explore_screen,
    );
    if inferred
        .book_source
        .search_url
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        inferred.book_source.search_url = model_source.search_url;
    }
    fill_json_option(
        &mut inferred.book_source.rule_review,
        model_source.rule_review,
    );
    fill_option(
        &mut inferred.book_source.book_source_comment,
        model_source.book_source_comment,
    );
    fill_option(
        &mut inferred.book_source.variable_comment,
        model_source.variable_comment,
    );
    fill_i64_option(
        &mut inferred.book_source.respond_time,
        model_source.respond_time,
    );
    fill_bool_option(
        &mut inferred.book_source.load_with_base_url,
        model_source.load_with_base_url,
    );
    fill_bool_option(
        &mut inferred.book_source.single_url,
        model_source.single_url,
    );

    merge_search_rule(
        inferred
            .book_source
            .rule_explore
            .get_or_insert_with(ExploreRule::default),
        model_source.rule_explore,
    );

    merge_search_rule(
        inferred
            .book_source
            .rule_search
            .get_or_insert_with(SearchRule::default),
        model_source.rule_search,
    );
    merge_book_info_rule(
        inferred
            .book_source
            .rule_book_info
            .get_or_insert_with(BookInfoRule::default),
        model_source.rule_book_info,
    );
    merge_toc_rule(
        inferred
            .book_source
            .rule_toc
            .get_or_insert_with(TocRule::default),
        model_source.rule_toc,
    );
    merge_content_rule(
        inferred
            .book_source
            .rule_content
            .get_or_insert_with(ContentRule::default),
        model_source.rule_content,
    );

    inferred
}

fn merge_search_rule(target: &mut SearchRule, source: Option<SearchRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.check_key_word, source.check_key_word);
    fill_option(&mut target.book_list, source.book_list);
    fill_option(&mut target.name, source.name);
    fill_option(&mut target.author, source.author);
    fill_option(&mut target.intro, source.intro);
    fill_option(&mut target.kind, source.kind);
    fill_option(&mut target.last_chapter, source.last_chapter);
    fill_option(&mut target.update_time, source.update_time);
    fill_option(&mut target.book_url, source.book_url);
    fill_option(&mut target.cover_url, source.cover_url);
    fill_option(&mut target.word_count, source.word_count);
}

fn merge_book_info_rule(target: &mut BookInfoRule, source: Option<BookInfoRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.init, source.init);
    fill_option(&mut target.name, source.name);
    fill_option(&mut target.author, source.author);
    fill_option(&mut target.intro, source.intro);
    fill_option(&mut target.kind, source.kind);
    fill_option(&mut target.last_chapter, source.last_chapter);
    fill_option(&mut target.update_time, source.update_time);
    fill_option(&mut target.cover_url, source.cover_url);
    fill_option(&mut target.word_count, source.word_count);
    fill_option(&mut target.toc_url, source.toc_url);
    fill_option(&mut target.can_re_name, source.can_re_name);
    fill_option(&mut target.download_urls, source.download_urls);
}

fn merge_toc_rule(target: &mut TocRule, source: Option<TocRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.pre_update_js, source.pre_update_js);
    fill_option(&mut target.init, source.init);
    fill_option(&mut target.chapter_list, source.chapter_list);
    fill_option(&mut target.chapter_name, source.chapter_name);
    fill_option(&mut target.chapter_url, source.chapter_url);
    fill_option(&mut target.format_js, source.format_js);
    fill_option(&mut target.is_volume, source.is_volume);
    fill_option(&mut target.is_vip, source.is_vip);
    fill_option(&mut target.is_pay, source.is_pay);
    fill_option(&mut target.update_time, source.update_time);
    fill_option(&mut target.next_toc_url, source.next_toc_url);
}

fn merge_content_rule(target: &mut ContentRule, source: Option<ContentRule>) {
    let Some(source) = source else {
        return;
    };
    fill_option(&mut target.content, source.content);
    fill_option(&mut target.title, source.title);
    fill_option(&mut target.next_content_url, source.next_content_url);
    fill_option(&mut target.web_js, source.web_js);
    fill_option(&mut target.source_regex, source.source_regex);
    fill_option(&mut target.replace_regex, source.replace_regex);
    fill_option(&mut target.image_style, source.image_style);
    fill_option(&mut target.image_decode, source.image_decode);
    fill_option(&mut target.pay_action, source.pay_action);
}

fn fill_option(target: &mut Option<String>, source: Option<String>) {
    if target.as_deref().unwrap_or_default().trim().is_empty() {
        *target = source.filter(|value| !value.trim().is_empty());
    }
}

fn fill_i64_option(target: &mut Option<i64>, source: Option<i64>) {
    if target.is_none() {
        *target = source;
    }
}

fn fill_bool_option(target: &mut Option<bool>, source: Option<bool>) {
    if target.is_none() {
        *target = source;
    }
}

fn fill_json_option(target: &mut Option<Value>, source: Option<Value>) {
    if target.is_none() {
        *target = source;
    }
}
