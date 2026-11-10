import { promises as fs } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..', '..');
const require = createRequire(import.meta.url);

async function readInput() {
  const chunks = [];
  for await (const chunk of process.stdin) {
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString('utf8'));
}

async function ensureDir(dir) {
  await fs.mkdir(dir, { recursive: true });
}

async function writeArtifact(outputDir, relativePath, contents) {
  const absolutePath = path.join(outputDir, relativePath);
  await ensureDir(path.dirname(absolutePath));
  await fs.writeFile(absolutePath, contents, 'utf8');
  return relativePath;
}

async function copyArtifact(source, outputDir, relativePath) {
  const absolutePath = path.join(outputDir, relativePath);
  await ensureDir(path.dirname(absolutePath));
  await fs.copyFile(source, absolutePath);
  return relativePath;
}

function createLogger(outputDir) {
  const relativePath = path.join('logs', 'worker.log');
  const absolutePath = path.join(outputDir, relativePath);

  return {
    path: relativePath,
    async write(message) {
      await ensureDir(path.dirname(absolutePath));
      await fs.appendFile(absolutePath, `${new Date().toISOString()} ${message}\n`, 'utf8');
    },
  };
}

function pushUnique(items, value) {
  if (value && !items.includes(value)) {
    items.push(value);
  }
}

async function capturePage({ page, outputDir, role, artifactManifest, pageResults, log }) {
  // shortcut: 取证截图跳过字体等待；升级 Playwright 时复测阻塞脚本场景。
  process.env.PW_TEST_SCREENSHOT_NO_FONTS_READY = '1';
  await page.waitForLoadState('domcontentloaded', { timeout: 5000 }).catch(() =>
    log.write(`capture-load-timeout role=${role} url=${page.url()} evidence=current-dom`));
  const htmlRelativePath = path.join('dom', `${role}.html`);
  const screenshotRelativePath = path.join('screenshots', `${role}.png`);
  await writeArtifact(outputDir, htmlRelativePath, await page.content());
  const screenshotOptions = {
    path: path.join(outputDir, screenshotRelativePath),
    fullPage: true,
  };
  try {
    await page.screenshot(screenshotOptions);
  } catch (error) {
    if (!String(error).includes('Unable to capture screenshot')) throw error;
    await log.write(`capture-screenshot-retry role=${role} url=${page.url()}`);
    await page.waitForTimeout(150);
    await page.screenshot(screenshotOptions);
  }
  await log.write(`capture role=${role} url=${page.url()}`);

  pushUnique(artifactManifest.dom, htmlRelativePath);
  pushUnique(artifactManifest.screenshots, screenshotRelativePath);
  pageResults.push({
    role,
    url: page.url(),
    htmlPath: htmlRelativePath,
    screenshotPath: screenshotRelativePath,
    snippetPaths: [],
  });
}

async function summarizePage(page) {
  return page.evaluate(() => {
    const toSafeClass = (value) => {
      if (!value) {
        return '';
      }
      return String(value)
        .trim()
        .replace(/[^\w-]+/g, '-')
        .replace(/^-+/, '')
        .replace(/-+$/, '');
    };
    const pushUnique = (items, value, max = 12) => {
      const normalized = String(value || '').trim();
      if (!normalized) {
        return;
      }
      if (!items.includes(normalized) && items.length < max) {
        items.push(normalized);
      }
    };
    const topTokens = (map, minCount = 2, max = 10) => {
      return [...map.entries()]
        .filter(([, count]) => count >= minCount)
        .sort((left, right) => right[1] - left[1])
        .slice(0, max)
        .map(([token]) => token);
    };

    const links = [];
    const seen = new Set();
    for (const anchor of [...document.querySelectorAll('a[href]')]) {
      const href = anchor.getAttribute('href');
      const label = (anchor.textContent || '').replace(/\s+/g, ' ').trim();
      if (!href) {
        continue;
      }
      const url = new URL(href, window.location.href).toString();
      if (seen.has(url)) {
        continue;
      }
      seen.add(url);
      links.push({
        label: label || anchor.getAttribute('title') || url,
        url,
      });
      if (links.length >= 96) {
        break;
      }
    }

    const forms = [...document.querySelectorAll('form')]
      .map((form) => ({
        action: new URL(form.getAttribute('action') || window.location.href, window.location.href).toString(),
        method: (form.getAttribute('method') || 'get').toLowerCase(),
        inputNames: [...form.querySelectorAll('input[name], textarea[name], select[name]')]
          .map((input) => input.getAttribute('name') || '')
          .filter(Boolean)
          .slice(0, 12),
      }))
      .slice(0, 8);

    const linkClassCounter = new Map();
    const containerClassCounter = new Map();
    const textContainerClassCounter = new Map();
    const chapterTextMatcher = /(第.{0,16}(章|节|卷|回))|章节|目录/i;
    const nextTextMatcher = /下一|下页|next/i;
    const nextPageCandidates = [];
    const tocLinkCandidates = [];
    const titleLinkCandidates = [];

    for (const anchor of [...document.querySelectorAll('a[href]')]) {
      const text = (anchor.textContent || '').replace(/\s+/g, ' ').trim();
      if (!text) {
        continue;
      }

      const anchorClass = toSafeClass(anchor.className || '');
      if (anchorClass) {
        linkClassCounter.set(anchorClass, (linkClassCounter.get(anchorClass) || 0) + 1);
      }

      const parent = anchor.parentElement;
      const parentClass = toSafeClass(parent?.className || '');
      if (parentClass) {
        containerClassCounter.set(parentClass, (containerClassCounter.get(parentClass) || 0) + 1);
      }

      if (chapterTextMatcher.test(text)) {
        if (parentClass) {
          pushUnique(tocLinkCandidates, `.${parentClass} a[href]`);
        }
        if (anchorClass) {
          pushUnique(tocLinkCandidates, `a.${anchorClass}[href]`);
        }
      }

      if (nextTextMatcher.test(text)) {
        if (anchorClass) {
          pushUnique(nextPageCandidates, `a.${anchorClass}[href]`);
        }
        if (parentClass) {
          pushUnique(nextPageCandidates, `.${parentClass} a[href]`);
        }
      }
    }

    for (const selector of ['article', 'main', 'section', 'div']) {
      for (const node of [...document.querySelectorAll(selector)]) {
        const textLength = (node.textContent || '').replace(/\s+/g, ' ').trim().length;
        if (textLength < 240) {
          continue;
        }
        const linksCount = node.querySelectorAll('a[href]').length;
        if (linksCount > 8) {
          continue;
        }
        const className = toSafeClass(node.className || '');
        if (className) {
          textContainerClassCounter.set(className, (textContainerClassCounter.get(className) || 0) + 1);
        }
      }
    }

    const bookListCandidates = [];
    for (const token of topTokens(containerClassCounter, 2, 8)) {
      pushUnique(bookListCandidates, `.${token}`);
    }
    for (const token of topTokens(linkClassCounter, 2, 6)) {
      pushUnique(titleLinkCandidates, `a.${token}[href]`);
    }
    for (const token of topTokens(containerClassCounter, 2, 8)) {
      pushUnique(titleLinkCandidates, `.${token} a[href]`);
    }
    pushUnique(titleLinkCandidates, 'h2 a[href]');
    pushUnique(titleLinkCandidates, 'h3 a[href]');
    pushUnique(tocLinkCandidates, '#list a[href]');
    pushUnique(tocLinkCandidates, '[class*="chapter"] a[href]');
    pushUnique(tocLinkCandidates, '[class*="catalog"] a[href]');
    pushUnique(nextPageCandidates, 'a[href*="next"]');

    const contentCandidates = [];
    const contentById = document.querySelector('[id*="content"], [id*="chapter"], [id*="read"]');
    if (contentById?.id) {
      pushUnique(contentCandidates, `#${toSafeClass(contentById.id)}`);
    }
    for (const token of topTokens(textContainerClassCounter, 1, 8)) {
      pushUnique(contentCandidates, `.${token}`);
    }
    pushUnique(contentCandidates, 'article');
    pushUnique(contentCandidates, 'main');

    const excerpt = (document.body?.innerText || '')
      .replace(/\s+/g, ' ')
      .trim()
      .slice(0, 1600);

    return {
      title: document.title || '',
      excerpt,
      links,
      forms,
      hints: {
        bookListCandidates,
        titleLinkCandidates,
        tocLinkCandidates,
        contentCandidates,
        nextPageCandidates,
      },
    };
  });
}

async function detectSearchForm(page) {
  return page.evaluate(() => {
    const forms = [...document.querySelectorAll('form[action]')];

    for (const form of forms) {
      const inputs = [...form.querySelectorAll('input[name]')];
      const textInput = inputs.find((input) => {
        const type = (input.getAttribute('type') || 'text').toLowerCase();
        const name = (input.getAttribute('name') || '').trim();
        return (
          type === 'text'
          || type === 'search'
          || /key|search|query|keyword|wd/i.test(name)
        );
      });

      if (!textInput) {
        continue;
      }

      return {
        action: form.getAttribute('action') || '',
        method: (form.getAttribute('method') || 'get').toLowerCase(),
        onsubmit: form.getAttribute('onsubmit') || '',
        inputName: textInput.getAttribute('name') || '',
        hiddenFields: inputs
          .filter((input) => (input.getAttribute('type') || '').toLowerCase() === 'hidden')
          .map((input) => ({
            name: input.getAttribute('name') || '',
            value: input.getAttribute('value') || '',
          })),
      };
    }

    return null;
  });
}

async function submitDetectedSearchForm(page, keyword, preferredInputName, expectedForm = null) {
  if (!keyword) {
    return null;
  }

  const startUrl = page.url();
  const result = await page.evaluate(({ value, preferredName, expectedForm }) => {
    const forms = [...document.querySelectorAll('form')];
    for (const form of forms) {
      const action = form.action;
      const method = (form.getAttribute('method') || 'get').toLowerCase();
      if (expectedForm && (action !== expectedForm.action || method !== expectedForm.method)) {
        continue;
      }
      const inputs = [...form.querySelectorAll('input[name], textarea[name]')];
      const textInput = inputs.find((input) => {
        const type = (input.getAttribute('type') || 'text').toLowerCase();
        const name = (input.getAttribute('name') || '').trim();
        if (expectedForm) {
          return name === preferredName;
        }
        return (
          type === 'text'
          || type === 'search'
          || name === preferredName
          || /key|search|query|keyword|wd/i.test(name)
        );
      });

      if (!textInput) {
        continue;
      }

      textInput.value = value;
      textInput.setAttribute('value', value);

      const inputName = textInput.getAttribute('name') || preferredName || '';
      const submitter = form.querySelector('button[type="submit"], input[type="submit"], button');

      if (submitter instanceof HTMLElement) {
        submitter.click();
        return {
          submitMode: 'submitter-click',
          action,
          method,
          queryParam: inputName,
        };
      }

      if (typeof form.requestSubmit === 'function') {
        form.requestSubmit();
        return {
          submitMode: 'request-submit',
          action,
          method,
          queryParam: inputName,
        };
      }

      const submitEvent = new Event('submit', { bubbles: true, cancelable: true });
      const dispatched = form.dispatchEvent(submitEvent);
      if (!dispatched) {
        return {
          submitMode: 'submit-event',
          action,
          method,
          queryParam: inputName,
        };
      }

      if (typeof form.onsubmit === 'function') {
        const onSubmitResult = form.onsubmit();
        return {
          submitMode: onSubmitResult === false ? 'onsubmit-handler' : 'onsubmit-handler-submit',
          action,
          method,
          queryParam: inputName,
        };
      }

      if (typeof window.handleSearch === 'function' && inputName) {
        window.handleSearch(form, null);
        return {
          submitMode: 'handleSearch',
          action,
          method,
          queryParam: inputName,
        };
      }

      if (typeof form.submit === 'function') {
        form.submit();
        return {
          submitMode: 'dom-submit',
          action,
          method,
          queryParam: inputName,
        };
      }
    }

    return null;
  }, {
    value: keyword,
    preferredName: preferredInputName || '',
    expectedForm,
  });

  if (!result) {
    return null;
  }

  await page.waitForURL((url) => url.toString() !== startUrl, { timeout: 5000 }).catch(() => {});
  await page.waitForLoadState('domcontentloaded').catch(() => {});
  await page.waitForTimeout(500);
  return result;
}

async function findFirstHref(page, selectors) {
  return page.evaluate((selectorList) => {
    for (const selector of selectorList) {
      const element = document.querySelector(selector);
      const href = element?.getAttribute('href');
      if (href) {
        return new URL(href, window.location.href).toString();
      }
    }
    return null;
  }, selectors);
}

async function hasAnySelector(page, selectors) {
  return page.evaluate((selectorList) => {
    return selectorList.some((selector) => Boolean(document.querySelector(selector)));
  }, selectors);
}

function extractBookIdFromUrl(url) {
  const match = String(url || '').match(/\/book\/([^/?#]+)\.html(?:[?#].*)?$/i);
  return match?.[1] || null;
}

function isBookSpecificTocUrl(url, bookId) {
  if (!url) {
    return false;
  }
  let pathname = '';
  try {
    pathname = new URL(url).pathname || '';
  } catch {
    return false;
  }

  if (/\/book\/category\/catalog\.html$/i.test(pathname)) {
    return false;
  }
  if (/\/book\/chapter\/[^/]+\.html$/i.test(pathname)) {
    return !bookId || pathname.includes(`/book/chapter/${bookId}.html`);
  }
  if (bookId && pathname.includes(`/book/${bookId}/`)) {
    return true;
  }
  return /chapter|catalog|mulu|list|dir|toc/i.test(pathname);
}

function isBookSpecificContentUrl(url, bookId) {
  if (!url || !bookId) {
    return false;
  }
  let pathname = '';
  try {
    pathname = new URL(url).pathname || '';
  } catch {
    return false;
  }

  if (pathname.includes('/book/chapter/')) {
    return false;
  }
  const chapterPattern = new RegExp(`/book/${bookId}/[^/]+\\.html$`, 'i');
  return chapterPattern.test(pathname);
}

async function openSearchPage(page, testKeyword) {
  if (!testKeyword) {
    return null;
  }

  const formInfo = await detectSearchForm(page);
  if (!formInfo?.inputName) {
    return null;
  }

  const submitted = await submitDetectedSearchForm(page, testKeyword, formInfo.inputName);
  if (!submitted) {
    return null;
  }

  return {
    source: 'detected',
    action: submitted.action
      ? new URL(submitted.action, page.url()).toString()
      : page.url(),
    method: submitted.method || formInfo.method || 'get',
    queryParam: submitted.queryParam || formInfo.inputName,
    submitMode: submitted.submitMode,
  };
}

async function openSearchPageWithPlan(page, testKeyword, searchForm) {
  if (!testKeyword) {
    return null;
  }

  const providedAction = String(searchForm?.action || '').trim();
  const providedMethod = String(searchForm?.method || 'get').trim().toLowerCase();
  const providedQueryParam = String(searchForm?.queryParam || '').trim();
  if (!providedAction || !providedQueryParam) {
    return null;
  }

  const submitted = await submitDetectedSearchForm(page, testKeyword, providedQueryParam, {
    action: new URL(providedAction, page.url()).toString(),
    method: providedMethod,
  });
  if (!submitted) {
    return null;
  }

  return {
    source: 'planned',
    action: submitted.action
      ? new URL(submitted.action, page.url()).toString()
      : page.url(),
    method: submitted.method || providedMethod,
    queryParam: submitted.queryParam || providedQueryParam,
    submitMode: submitted.submitMode,
  };
}

async function safeGoto(page, url) {
  if (!url) {
    return false;
  }

  try {
    await page.goto(url, {
      waitUntil: 'domcontentloaded',
      timeout: 12000,
    });
    return true;
  } catch (error) {
    await page.waitForTimeout(500);
    if (page.url() !== 'about:blank') {
      return true;
    }

    throw error;
  }
}

async function waitForSelectorIfNeeded(page, selector) {
  const value = String(selector || '').trim();
  if (!value) {
    return false;
  }

  try {
    await page.waitForSelector(value, {
      state: 'attached',
      timeout: 12000,
    });
    return true;
  } catch {
    return false;
  }
}

function isEffectivelyEmptyHtml(html) {
  const normalized = String(html || '')
    .replace(/<script[\s\S]*?<\/script>/gi, '')
    .replace(/<style[\s\S]*?<\/style>/gi, '')
    .replace(/<[^>]+>/g, '')
    .replace(/\s+/g, '')
    .trim();
  return normalized.length < 20;
}

async function waitForAnySelector(page, selectors, timeoutMs) {
  const timeout = Number(timeoutMs || 0);
  if (timeout <= 0) {
    return null;
  }
  const startedAt = Date.now();
  while (Date.now() - startedAt < timeout) {
    for (const selector of selectors) {
      try {
        const found = await page.$(selector);
        if (found) {
          return selector;
        }
      } catch {
        // Ignore invalid selectors from hints.
      }
    }
    await page.waitForTimeout(200);
  }
  return null;
}

async function hasSearchResultSignal(page, keyword) {
  const strongSelectors = [
    '.librarylist li',
    '.result-list .result-item',
    '.novellist .item',
    '.search-result li',
    '[class*="search"] [class*="result"]',
    '[id*="search"] [class*="result"]',
  ];
  const genericSelectors = [
    '.book-list li',
    '.bookbox',
  ];
  const strongFound = await waitForAnySelector(page, strongSelectors, 2000);

  return page.evaluate(({ value, foundStrong, genericSelectorList }) => {
    const bodyText = (document.body?.innerText || '').replace(/\s+/g, ' ');
    const links = [...document.querySelectorAll('a[href]')];
    const normalizedKeyword = String(value || '').trim().toLowerCase();
    const titleCandidates = links
      .map((anchor) => (anchor.textContent || '').replace(/\s+/g, ' ').trim())
      .filter(Boolean)
      .slice(0, 80);
    const bookLikeLinks = links.filter((anchor) => {
      const href = anchor.getAttribute('href') || '';
      return /\/book\/|\/biqu|\/chapter\//i.test(href);
    }).length;
    const keywordHits = normalizedKeyword
      ? titleCandidates.filter((text) => text.toLowerCase().includes(normalizedKeyword)).length
      : 0;
    const hasResultWords = /搜索|结果|找到|共\d+|相关/.test(bodyText);
    const decodedUrlText = decodeURIComponent(window.location.href).toLowerCase();
    const urlCarriesKeyword = normalizedKeyword && decodedUrlText.includes(normalizedKeyword);
    const genericFound = genericSelectorList.find((selector) => Boolean(document.querySelector(selector))) || '';

    if (foundStrong) {
      if (!normalizedKeyword) {
        return { matched: true, selector: foundStrong, reason: 'strong-selector' };
      }
      if (keywordHits >= 1 || (hasResultWords && urlCarriesKeyword)) {
        return { matched: true, selector: foundStrong, reason: 'strong-selector+keyword' };
      }
    }

    if (normalizedKeyword && keywordHits >= 1 && bookLikeLinks >= 1) {
      return { matched: true, selector: '', reason: 'keyword-hit' };
    }
    if (normalizedKeyword && hasResultWords && keywordHits >= 1) {
      return { matched: true, selector: '', reason: 'result-copy+keyword-hit' };
    }
    if (!normalizedKeyword && bookLikeLinks >= 3) {
      return { matched: true, selector: '', reason: 'book-links' };
    }
    if (genericFound && normalizedKeyword && keywordHits >= 1) {
      return { matched: true, selector: genericFound, reason: 'generic-selector+keyword-hit' };
    }
    if (normalizedKeyword && bodyText.includes(value) && bookLikeLinks >= 1 && urlCarriesKeyword) {
      return { matched: true, selector: '', reason: 'keyword+links' };
    }
    return { matched: false, selector: '', reason: '' };
  }, {
    value: String(keyword || '').trim(),
    foundStrong: strongFound || '',
    genericSelectorList: genericSelectors,
  });
}

async function findBestSearchResultHref(page, testKeyword) {
  return page.evaluate(({ keyword }) => {
    const normalize = (value) =>
      String(value || '')
        .toLowerCase()
        .replace(/[^\p{L}\p{N}]+/gu, '');

    const needle = normalize(keyword);
    const containerSelectors = [
      '.librarylist li',
      '.result-list .result-item',
      '.novellist .item',
      '.search-result li',
      '.book-list li',
      '.bookbox',
    ];
    const linkSelectors = [
      '.bookname a[href]',
      '.bookinfo .bookname a[href]',
      '.novelname[href]',
      '.result-title a[href]',
      '.book-title a[href]',
      'h3 a[href]',
      'a[href]',
    ];

    const scoredResults = [];
    for (const containerSelector of containerSelectors) {
      const containers = [...document.querySelectorAll(containerSelector)];
      if (!containers.length) {
        continue;
      }

      for (const container of containers) {
        let link = null;
        for (const linkSelector of linkSelectors) {
          link = container.querySelector(linkSelector);
          if (link) {
            break;
          }
        }

        if (!link) {
          continue;
        }

        const href = link.getAttribute('href') || '';
        const title = (link.textContent || '').trim();
        if (!href || !title) {
          continue;
        }

        const normalizedTitle = normalize(title);
        let score = 1;
        if (needle) {
          if (normalizedTitle === needle) {
            score = 1000;
          } else if (normalizedTitle.startsWith(needle)) {
            score = 800;
          } else if (normalizedTitle.includes(needle)) {
            score = 600;
          } else {
            score = 0;
          }
        }

        scoredResults.push({
          href: new URL(href, window.location.href).toString(),
          score,
          titleLength: normalizedTitle.length || title.length,
        });
      }

      if (scoredResults.length) {
        break;
      }
    }

    scoredResults.sort((left, right) => {
      if (right.score !== left.score) {
        return right.score - left.score;
      }
      return left.titleLength - right.titleLength;
    });

    return scoredResults[0]?.href || null;
  }, { keyword: testKeyword || '' });
}

async function sampleFixture({ fixtureName, fixturesDir, outputDir }) {
  const fixtureRoot = path.join(fixturesDir || path.join(repoRoot, 'fixtures'), fixtureName);
  const manifest = JSON.parse(
    await fs.readFile(path.join(fixtureRoot, 'manifest.json'), 'utf8'),
  );
  const log = createLogger(outputDir);
  await log.write(`fixture-start name=${fixtureName}`);

  const pageResults = [];
  const artifactManifest = {
    screenshots: [],
    dom: [],
    network: [],
    logs: [],
    output: [],
  };

  for (const page of manifest.pages) {
    await log.write(`fixture-page role=${page.role} html=${page.htmlFile} screenshot=${page.screenshotFile}`);
    const htmlSource = await fs.readFile(path.join(fixtureRoot, page.htmlFile), 'utf8');
    const htmlRelativePath = path.join('dom', `${page.role}.html`);
    await writeArtifact(outputDir, htmlRelativePath, htmlSource);
    artifactManifest.dom.push(htmlRelativePath);

    const screenshotRelativePath = path.join(
      'screenshots',
      `${path.parse(page.screenshotFile).name}.svg`,
    );
    await copyArtifact(path.join(fixtureRoot, page.screenshotFile), outputDir, screenshotRelativePath);
    artifactManifest.screenshots.push(screenshotRelativePath);

    const snippetPaths = [];
    for (const snippet of page.snippets ?? []) {
      const snippetRelativePath = path.join('dom', snippet.fileName);
      await writeArtifact(outputDir, snippetRelativePath, snippet.contents);
      artifactManifest.dom.push(snippetRelativePath);
      snippetPaths.push(snippetRelativePath);
    }

    pageResults.push({
      role: page.role,
      url: page.url,
      htmlPath: htmlRelativePath,
      screenshotPath: screenshotRelativePath,
      snippetPaths,
    });
  }

  const networkSamples = [];
  for (const networkSample of manifest.networkSamples ?? []) {
    await log.write(`fixture-network label=${networkSample.label} method=${networkSample.method} url=${networkSample.url}`);
    const contents = await fs.readFile(
      path.join(fixtureRoot, networkSample.responseFile),
      'utf8',
    );
    const responseRelativePath = path.join(
      'network',
      `${path.parse(networkSample.responseFile).name}.json`,
    );
    await writeArtifact(outputDir, responseRelativePath, contents);
    artifactManifest.network.push(responseRelativePath);
    networkSamples.push({
      label: networkSample.label,
      requestUrl: networkSample.url,
      method: networkSample.method,
      responsePath: responseRelativePath,
      notes: networkSample.notes,
    });
  }

  pushUnique(artifactManifest.logs, log.path);

  return {
    fixtureName,
    siteLabel: manifest.name,
    baseUrl: manifest.baseUrl,
    pages: pageResults,
    networkSamples,
    manifest: artifactManifest,
  };
}

async function sampleLiveSite({ targetUrl, outputDir, testKeyword }) {
  let browser;
  try {
    const playwright = require(process.env.AISD_PLAYWRIGHT_MODULE || 'playwright');
    browser = await playwright.chromium.launch();
    const page = await browser.newPage();
    page.setDefaultNavigationTimeout(12000);
    page.setDefaultTimeout(12000);
    const log = createLogger(outputDir);
    await log.write(`live-start target=${targetUrl} keyword=${testKeyword || ''}`);
    const networkSamples = [];
    const artifactManifest = {
      screenshots: [],
      dom: [],
      network: [],
      logs: [],
      output: [],
    };
    const pageResults = [];
    let networkCounter = 0;

    page.on('response', async (response) => {
      if (networkCounter >= 8) {
        return;
      }
      const sampleIndex = ++networkCounter;

      const request = response.request();
      const responseUrl = response.url();
      const contentType = response.headers()['content-type'] || '';
      if (!responseUrl.startsWith(new URL(targetUrl).origin)) {
        return;
      }
      if (!(contentType.includes('json') || contentType.includes('html') || contentType.includes('text'))) {
        return;
      }

      let extension = 'txt';
      if (contentType.includes('json')) {
        extension = 'json';
      } else if (contentType.includes('html')) {
        extension = 'html';
      }

      const relativePath = path.join('network', `live-${sampleIndex}.${extension}`);
      try {
        const body = await response.text();
        await writeArtifact(outputDir, relativePath, body.slice(0, 20000));
        pushUnique(artifactManifest.network, relativePath);
        await log.write(`network-sample label=live-${sampleIndex} status=${response.status()} type=${contentType} url=${responseUrl}`);
        networkSamples.push({
          label: `live-${sampleIndex}`,
          requestUrl: responseUrl,
          method: request.method(),
          responsePath: relativePath,
          notes: `status=${response.status()} content-type=${contentType}`,
        });
      } catch (error) {
        networkSamples.push({
          label: `live-${sampleIndex}`,
          requestUrl: responseUrl,
          method: request.method(),
          responsePath: '',
          notes: `body-unavailable: ${error instanceof Error ? error.message : String(error)}`,
        });
      }
    });

    await log.write(`goto landing url=${targetUrl}`);
    await safeGoto(page, targetUrl);

    await capturePage({
      page,
      outputDir,
      role: 'landing',
      artifactManifest,
      pageResults,
      log,
    });

    if (await openSearchPage(page, testKeyword)) {
      await log.write(`open-search keyword=${testKeyword}`);
        await capturePage({
          page,
          outputDir,
          role: 'search',
          artifactManifest,
          pageResults,
          log,
        });

      const bookUrl = (await findBestSearchResultHref(page, testKeyword)) || await findFirstHref(page, [
        '.bookbox .bookname a[href]',
        '.bookinfo .bookname a[href]',
        '.librarylist .novelname[href]',
        '.book-title a[href]',
        '.novelname[href]',
        '.novellist .item a[href*="/book/"]',
        'a[href^="/biqu"]',
        'a[href*="/book/"]',
        '.result-list a[href]',
      ]);

      if (bookUrl) {
        const bookId = extractBookIdFromUrl(bookUrl);
        await log.write(`open-book url=${bookUrl}`);
        await safeGoto(page, bookUrl);
          await capturePage({
            page,
            outputDir,
            role: 'book',
            artifactManifest,
            pageResults,
            log,
          });

        const inlineTocSelectors = [
          '#chapterlist p',
          '.recommend #chapterlist p',
          '.directoryArea p',
          '.fulldir .dirlist li',
          '.dirlist li',
          '.chapter-list li',
          '.listmain dd',
          '#list dd',
          '.booklist li',
        ];

        const hasInlineToc = await hasAnySelector(page, inlineTocSelectors);
        if (hasInlineToc) {
          await log.write('inline-toc detected');
          await capturePage({
            page,
            outputDir,
            role: 'toc',
            artifactManifest,
            pageResults,
            log,
          });
        }

        let tocUrl = await findFirstHref(page, [
          'a[href*="/book/chapter/"]',
          '#chapterlist a[href]',
          '.recommend #chapterlist a[href]',
          '.directoryArea a[href]',
          '.fulldir .dirlist a[href]',
          '.dirlist a[href]',
          '.toc-link[href]',
          'a[href*="list"]',
        ]);

        if (hasInlineToc) {
          tocUrl = null;
        }
        if (tocUrl && !isBookSpecificTocUrl(tocUrl, bookId)) {
          await log.write(`skip-toc url=${tocUrl} reason=non-book-specific`);
          tocUrl = null;
        }

        if (!tocUrl) {
          const chapterUrl = await findFirstHref(page, [
          '#chapterlist a[href]',
          '.recommend #chapterlist a[href]',
          '.directoryArea a[href]',
            '.btn a[href]',
            '.fulldir .dirlist a[href]',
            '.dirlist a[href]',
            '.chapter-list a[href]',
            '.listmain a[href]',
            '#list a[href]',
            '.booklist a[href]',
          ]);
          if (
            chapterUrl
            && !hasInlineToc
            && (!bookId || chapterUrl.includes(`/book/${bookId}/`))
          ) {
            await capturePage({
              page,
              outputDir,
              role: 'toc',
              artifactManifest,
              pageResults,
            });
            tocUrl = page.url();
          }
        }

        if (tocUrl && tocUrl !== page.url()) {
          await log.write(`open-toc url=${tocUrl}`);
          await safeGoto(page, tocUrl);
          await capturePage({
            page,
            outputDir,
            role: 'toc',
            artifactManifest,
            pageResults,
            log,
          });
        }

        let chapterUrl = await findFirstHref(page, [
          '#chapterlist a[href]',
          '.recommend #chapterlist a[href]',
          '.directoryArea a[href]',
          '.btn a[href]',
          '.fulldir .dirlist a[href]',
          '.dirlist a[href]',
          '.chapter-list a[href]',
          '.listmain a[href]',
          '#list a[href]',
          '.booklist a[href]',
          'a[href*="/chapter"]',
        ]);
        if (!isBookSpecificContentUrl(chapterUrl, bookId)) {
          const narrowedChapterUrl = await findFirstHref(page, [
            `a[href*="/book/${bookId}/"]`,
            '#allchapter a[href]',
            '.chapterlist a[href]',
          ]);
          chapterUrl = isBookSpecificContentUrl(narrowedChapterUrl, bookId)
            ? narrowedChapterUrl
            : null;
        }

        if (chapterUrl) {
          await log.write(`open-content url=${chapterUrl}`);
          await safeGoto(page, chapterUrl);
          await capturePage({
            page,
            outputDir,
            role: 'content',
            artifactManifest,
            pageResults,
            log,
          });
        }
      }
    }

    pushUnique(artifactManifest.logs, log.path);

    return {
      fixtureName: null,
      siteLabel: new URL(targetUrl).hostname,
      baseUrl: new URL(targetUrl).origin,
      pages: pageResults,
      manifest: artifactManifest,
      networkSamples,
    };
  } catch (error) {
    throw new Error(
      `实时 Playwright 采样未就绪：${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    if (browser) {
      await browser.close().catch(() => {});
    }
  }
}

async function inspectLivePage({
  targetUrl,
  outputDir,
  role,
  testKeyword,
  mode,
  searchForm,
  waitForSelector,
}) {
  let browser;
  try {
    const playwright = require(process.env.AISD_PLAYWRIGHT_MODULE || 'playwright');
    browser = await playwright.chromium.launch();
    const page = await browser.newPage();
    page.setDefaultNavigationTimeout(12000);
    page.setDefaultTimeout(12000);
    const log = createLogger(outputDir);
    await log.write(`inspect-start mode=${mode} role=${role} target=${targetUrl} keyword=${testKeyword || ''}`);

    const networkSamples = [];
    const artifactManifest = {
      screenshots: [],
      dom: [],
      network: [],
      logs: [],
      output: [],
    };
    const pageResults = [];
    let networkCounter = 0;
    const targetOrigin = new URL(targetUrl).origin;

    page.on('response', async (response) => {
      if (networkCounter >= 6) {
        return;
      }
      const responseUrl = response.url();
      if (!responseUrl.startsWith(targetOrigin)) {
        return;
      }
      const contentType = response.headers()['content-type'] || '';
      if (!(contentType.includes('json') || contentType.includes('html') || contentType.includes('text'))) {
        return;
      }
      const sampleIndex = ++networkCounter;
      const extension = contentType.includes('json')
        ? 'json'
        : contentType.includes('html')
          ? 'html'
          : 'txt';
      const relativePath = path.join('network', `${role || 'inspect'}-${sampleIndex}.${extension}`);

      try {
        const body = await response.text();
        await writeArtifact(outputDir, relativePath, body.slice(0, 16000));
        pushUnique(artifactManifest.network, relativePath);
        await log.write(`network-sample label=${role || 'inspect'}-${sampleIndex} status=${response.status()} type=${contentType} url=${responseUrl}`);
        networkSamples.push({
          label: `${role || 'inspect'}-${sampleIndex}`,
          requestUrl: responseUrl,
          method: response.request().method(),
          responsePath: relativePath,
          notes: `status=${response.status()} content-type=${contentType}`,
        });
      } catch (error) {
        networkSamples.push({
          label: `${role || 'inspect'}-${sampleIndex}`,
          requestUrl: responseUrl,
          method: response.request().method(),
          responsePath: '',
          notes: `body-unavailable: ${error instanceof Error ? error.message : String(error)}`,
        });
      }
    });

    let searchExecution = null;
    if (mode === 'search') {
      await log.write(`goto-search-entry url=${targetUrl}`);
      await safeGoto(page, targetUrl);
      searchExecution = await openSearchPageWithPlan(page, testKeyword, searchForm);
      if (!searchExecution) {
        await log.write('search-unavailable reason=no-matching-executable-observed-form');
        await capturePage({ page, outputDir, role: 'search-failed', artifactManifest, pageResults, log });
        throw new Error('search unavailable: no matching executable observed search form');
      } else {
        await log.write(
          `search-submitted source=${searchExecution.source} submitMode=${searchExecution.submitMode || ''} formAction=${searchExecution.action} formMethod=${searchExecution.method} queryParam=${searchExecution.queryParam}`,
        );
      }
    } else {
      await log.write(`goto-page url=${targetUrl}`);
      await safeGoto(page, targetUrl);
    }

    const waited = await waitForSelectorIfNeeded(page, waitForSelector);
    if (waited) {
      await log.write(`wait-selector success selector=${waitForSelector}`);
    } else if (String(waitForSelector || '').trim()) {
      await log.write(`wait-selector timeout selector=${waitForSelector} fallback=continue`);
    }

    if (mode === 'search') {
      const searchSignal = await hasSearchResultSignal(page, testKeyword || '');
      if (searchSignal?.matched) {
        await log.write(
          `wait-search-ready success reason=${searchSignal.reason} selector=${searchSignal.selector || ''}`,
        );
      } else {
        await log.write('search-unavailable reason=no-observed-result');
        await capturePage({ page, outputDir, role: 'search-failed', artifactManifest, pageResults, log });
        throw new Error('search unavailable: submitted form produced no observed search results');
      }
      const html = await page.content();
      if (isEffectivelyEmptyHtml(html)) {
        await log.write('search-empty-html detected retry=networkidle');
        await page.waitForLoadState('networkidle', { timeout: 5000 }).catch(() => {});
        await page.waitForTimeout(600);
      }
    }

    await capturePage({
      page,
      outputDir,
      role,
      artifactManifest,
      pageResults,
      log,
    });

    const summary = await summarizePage(page);
    pushUnique(artifactManifest.logs, log.path);

    return {
      page: pageResults[0],
      title: summary.title,
      excerpt: summary.excerpt,
      links: summary.links,
      forms: summary.forms,
      hints: summary.hints,
      networkSamples,
      manifest: artifactManifest,
    };
  } catch (error) {
    throw new Error(
      `Playwright 页面检查失败：${error instanceof Error ? error.message : String(error)}`,
    );
  } finally {
    if (browser) {
      await browser.close().catch(() => {});
    }
  }
}

async function main() {
  const request = await readInput();

  const {
    targetUrl,
    outputDir,
    fixturesDir,
    testKeyword,
    role,
    searchForm,
    waitForSelector,
  } = request.params;
  await ensureDir(outputDir);

  let result;
  if (request.method === 'sampleSite') {
    result = targetUrl.startsWith('fixture://')
      ? await sampleFixture({
          fixtureName: targetUrl.replace('fixture://', '').split('?')[0],
          fixturesDir,
          outputDir,
        })
      : await sampleLiveSite({ targetUrl, outputDir, testKeyword });
  } else if (request.method === 'inspectPage') {
    if (targetUrl.startsWith('fixture://')) {
      throw new Error('inspectPage does not support fixture targets');
    }
    result = await inspectLivePage({
      targetUrl,
      outputDir,
      role: role || 'inspect',
      testKeyword,
      mode: 'open',
      searchForm: null,
      waitForSelector,
    });
  } else if (request.method === 'inspectSearchPage') {
    if (targetUrl.startsWith('fixture://')) {
      throw new Error('inspectSearchPage does not support fixture targets');
    }
    result = await inspectLivePage({
      targetUrl,
      outputDir,
      role: role || 'search',
      testKeyword,
      mode: 'search',
      searchForm: searchForm || null,
      waitForSelector,
    });
  } else {
    throw new Error(`Unsupported worker method: ${request.method}`);
  }

  process.stdout.write(
    JSON.stringify({
      id: request.id,
      result,
    }),
  );
}

main().catch((error) => {
  process.stderr.write(
    JSON.stringify({
      id: null,
      error: error instanceof Error ? error.message : String(error),
    }),
  );
  process.exitCode = 1;
});
