import { promises as fs } from 'node:fs';
import path from 'node:path';
import process from 'node:process';
import { fileURLToPath } from 'node:url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, '..', '..');

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

function pushUnique(items, value) {
  if (value && !items.includes(value)) {
    items.push(value);
  }
}

async function capturePage({ page, outputDir, role, artifactManifest, pageResults }) {
  const htmlRelativePath = path.join('dom', `${role}.html`);
  const screenshotRelativePath = path.join('screenshots', `${role}.png`);
  await writeArtifact(outputDir, htmlRelativePath, await page.content());
  await page.screenshot({
    path: path.join(outputDir, screenshotRelativePath),
    fullPage: true,
  });

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

async function openSearchPage(page, testKeyword) {
  if (!testKeyword) {
    return false;
  }

  const formInfo = await detectSearchForm(page);
  if (!formInfo?.action || !formInfo.inputName) {
    return false;
  }

  const searchUrl = new URL(formInfo.action, page.url());
  for (const field of formInfo.hiddenFields || []) {
    if (field.name && !searchUrl.searchParams.has(field.name)) {
      searchUrl.searchParams.set(field.name, field.value);
    }
  }
  searchUrl.searchParams.set(formInfo.inputName, testKeyword);
  if (!searchUrl.searchParams.has('page')) {
    searchUrl.searchParams.set('page', '1');
  }

  await page.goto(searchUrl.toString(), { waitUntil: 'domcontentloaded' });
  return true;
}

async function sampleFixture({ fixtureName, fixturesDir, outputDir }) {
  const fixtureRoot = path.join(fixturesDir || path.join(repoRoot, 'fixtures'), fixtureName);
  const manifest = JSON.parse(
    await fs.readFile(path.join(fixtureRoot, 'manifest.json'), 'utf8'),
  );

  const pageResults = [];
  const artifactManifest = {
    screenshots: [],
    dom: [],
    network: [],
    logs: [],
    output: [],
  };

  for (const page of manifest.pages) {
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
  try {
    const playwright = await import('playwright');
    const browser = await playwright.chromium.launch();
    const page = await browser.newPage();
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

    await page.goto(targetUrl, { waitUntil: 'domcontentloaded' });

    await capturePage({
      page,
      outputDir,
      role: 'landing',
      artifactManifest,
      pageResults,
    });

    if (await openSearchPage(page, testKeyword)) {
      await capturePage({
        page,
        outputDir,
        role: 'search',
        artifactManifest,
        pageResults,
      });

      const bookUrl = await findFirstHref(page, [
        '.book-title a[href]',
        '.novellist .item a[href*="/book/"]',
        'a[href*="/book/"]',
        '.result-list a[href]',
      ]);

      if (bookUrl) {
        await page.goto(bookUrl, { waitUntil: 'domcontentloaded' });
        await capturePage({
          page,
          outputDir,
          role: 'book',
          artifactManifest,
          pageResults,
        });

        let tocUrl = await findFirstHref(page, [
          '.toc-link[href]',
          'a[href*="list"]',
          'a[href*="catalog"]',
        ]);

        if (!tocUrl) {
          const chapterUrl = await findFirstHref(page, [
            '.chapter-list a[href]',
            '.listmain a[href]',
            '#list a[href]',
            '.booklist a[href]',
          ]);
          if (chapterUrl) {
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
          await page.goto(tocUrl, { waitUntil: 'domcontentloaded' });
          await capturePage({
            page,
            outputDir,
            role: 'toc',
            artifactManifest,
            pageResults,
          });
        }

        const chapterUrl = await findFirstHref(page, [
          '.chapter-list a[href]',
          '.listmain a[href]',
          '#list a[href]',
          '.booklist a[href]',
          'a[href*="/chapter"]',
        ]);

        if (chapterUrl) {
          await page.goto(chapterUrl, { waitUntil: 'domcontentloaded' });
          await capturePage({
            page,
            outputDir,
            role: 'content',
            artifactManifest,
            pageResults,
          });
        }
      }
    }

    await browser.close();
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
  }
}

async function main() {
  const request = await readInput();

  if (request.method !== 'sampleSite') {
    throw new Error(`Unsupported worker method: ${request.method}`);
  }

  const {
    targetUrl,
    outputDir,
    fixturesDir,
    testKeyword,
  } = request.params;
  await ensureDir(outputDir);

  const result = targetUrl.startsWith('fixture://')
    ? await sampleFixture({
        fixtureName: targetUrl.replace('fixture://', '').split('?')[0],
        fixturesDir,
        outputDir,
      })
    : await sampleLiveSite({ targetUrl, outputDir, testKeyword });

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
