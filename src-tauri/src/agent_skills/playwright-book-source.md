name: playwright-book-source-agent
description: Use Playwright evidence to build a Reader-compatible book source with explore, search, book info, toc, and content rules.

Workflow
- Start from observable evidence, never from guesswork.
- Prefer JSON API over CSS, CSS over XPath, XPath over JavaScript.
- Normalize relative URLs against the sampled page URL.
- When a field cannot be justified by evidence, leave it null or omit it instead of fabricating.
- `searchUrl` is optional when `exploreUrl + ruleExplore` provide a stable discovery path. At least one of `search` or `explore` should be evidence-backed.
- Output must be one valid JSON object only. Do not add explanations before or after JSON.

Required coverage
- Explore: infer `exploreUrl` and `ruleExplore` when category, rank, or discovery pages are visible.
- Search: infer `searchUrl` and `ruleSearch`.
- Book info: infer `ruleBookInfo`.
- Toc: infer `ruleToc`.
- Content: infer `ruleContent`.

Playwright action policy
- Use landing page to discover search, rank, category, and navigation entry points.
- Infer explore categories from actual navigation links; exclude book-list links and preserve empty group headings and category styles. Do not copy unobserved category paths from example sources.
- When an explore page has a next-page link, capture it as `explore-next` and compare the first and second URLs. `exploreUrl` must use the observed page template, including a special first-page URL when needed, such as `https://books.example/catalog{{page == 1 ? '' : '/' + page}}`. A fixed category URL cannot paginate.
- Verify that page 2 yields book names, detail URLs, and new books; a changed URL alone does not prove pagination works. Keep existing dynamic templates when their expanded URLs match the evidence.
- Use search results to locate a stable book detail URL pattern.
- Use a detail page to infer metadata selectors and toc entry points.
- Use a toc page to infer chapter list, chapter name, next toc, and chapter URL rules.
- Use at least one content page to infer content extraction, pagination, and cleanup rules.
- If a book page exposes a direct toc link, prefer that observed toc link over re-inferring a synthetic toc URL.
- `ruleBookInfo.tocUrl` and toc evidence must prefer book-specific chapter catalog URLs. Do not use global category catalog URLs for a single book.
- Search/explore fields are relative to each `bookList` item; toc fields are relative to each `chapterList` item. For `bookList: .item`, use `@css::is(h3 a)@href`. Use `@text` or `@href` to extract from the item itself. Do not repeat the list item's selector in a field rule unless selecting a nested item.
- Use `@css::is(...)` for tag-led descendant CSS selectors, including list rules, so Reader-Rust keeps their CSS meaning. Keep the `@css:` prefix when the selector begins with a pseudo-class.
- `bookUrlPattern` is a regular expression for recognizing detail URLs, not a URL template. Leave it null unless supported by observed detail URLs; never put `{{id}}` or other placeholders in it.

Output policy
- Return `strategy`, `confidence`, `evidenceRefs`, `repairHistory`, and `bookSource`.
- `evidenceRefs` entries must be objects with `label`, `artifactPath`, and optional `detail`.
- `bookSource` should include richer metadata when evidence exists:
  - `bookSourceGroup`
  - `bookSourceType`
  - `bookUrlPattern`
  - `customOrder`
  - `enabledExplore`
  - `exploreUrl`
  - `ruleExplore`
  - `respondTime`
- Do not emit markdown fences.
- Keep compatibility with the current Reader-Rust model; `ruleContent.title` and `ruleToc.updateTime` are optional string rules.
