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
- Use search results to locate a stable book detail URL pattern.
- Use a detail page to infer metadata selectors and toc entry points.
- Use a toc page to infer chapter list, chapter name, next toc, and chapter URL rules.
- Use at least one content page to infer content extraction, pagination, and cleanup rules.
- If a book page exposes a direct toc link, prefer that observed toc link over re-inferring a synthetic toc URL.
- `ruleBookInfo.tocUrl` and toc evidence must prefer book-specific chapter catalog URLs. Do not use global category catalog URLs for a single book.

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
- Keep compatibility with Reader-Rust model constraints:
  - Do not output `ruleContent.title`.
  - Do not output `ruleToc.updateTime`.
