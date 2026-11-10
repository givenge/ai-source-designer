import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { promises as fs } from 'node:fs';
import { createServer } from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const root = await fs.mkdtemp(path.join(os.tmpdir(), 'asd-worker-regression-'));
const externalRequests = [];
const external = createServer((request, response) => {
  externalRequests.push(request.url);
  response.end('external search');
});
await new Promise(resolve => external.listen(0, '127.0.0.1', resolve));
const externalOrigin = `http://127.0.0.1:${external.address().port}`;
const sockets = new Set();
const requests = [];
const site = createServer(async (request, response) => {
  if (request.url === '/blocked.js') return;
  let body = '';
  for await (const chunk of request) body += chunk;
  requests.push({ url: request.url, method: request.method, body });
  const url = new URL(request.url, 'http://localhost');
  response.setHeader('content-type', 'text/html; charset=utf-8');
  if (url.pathname === '/no-form') {
    response.end(`<a href="${externalOrigin}/search">搜索</a><h3><a href="/book/42">星门</a></h3>`);
  } else if (url.pathname === '/forms') {
    response.end(`<form action="${externalOrigin}/search"><input name="q"><button>搜索</button></form><form action="/results"><input name="unrelated"><input name="q"><button>站内搜索</button></form>`);
  } else if (url.pathname === '/post') {
    response.end('<form action="/results" method="post"><input name="q"><button>搜索</button></form>');
  } else if (url.pathname === '/results') {
    const keyword = url.searchParams.get('q') || new URLSearchParams(body).get('q');
    response.end(`<h1>搜索结果</h1><ul class="search-result"><li><h3><a href="/book/42">${keyword}</a></h3></li></ul>`);
  } else {
    response.end('<html><body><h1>可读页面</h1><a href="/book/42">星门</a><script src="/blocked.js"></script></body></html>');
  }
});
site.on('connection', socket => {
  sockets.add(socket);
  socket.on('close', () => sockets.delete(socket));
});
await new Promise(resolve => site.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${site.address().port}`;

async function run(name, entry, searchForm, method = 'inspectSearchPage') {
  const outputDir = path.join(root, name);
  const child = spawn(process.execPath, [path.join(repo, 'playwright/worker/index.mjs')], {
    cwd: repo,
    env: { ...process.env },
  });
  let stdout = '', stderr = '';
  child.stdout.on('data', chunk => { stdout += chunk; });
  child.stderr.on('data', chunk => { stderr += chunk; });
  const timer = setTimeout(() => child.kill('SIGKILL'), 60000);
  child.stdin.end(JSON.stringify({ id: 1, method, params: {
    targetUrl: `${origin}${entry}`, outputDir, role: method === 'inspectPage' ? 'explore' : 'search',
    testKeyword: '星门', searchForm,
  } }));
  try {
    const code = await new Promise((resolve, reject) => {
      child.on('error', reject);
      child.on('close', resolve);
    });
    assert.notEqual(code, null, `${name} timed out`);
    return { code, payload: stdout ? JSON.parse(stdout) : {}, stderr, outputDir };
  } finally {
    clearTimeout(timer);
  }
}

try {
  for (const [name, entry, form] of [
    ['no-form', '/no-form', null],
    ['missing-plan', '/forms', null],
    ['mismatched-plan', '/forms', { action: `${origin}/invented`, method: 'get', queryParam: 'q' }],
  ]) {
    const result = await run(name, entry, form);
    assert.notEqual(result.code, 0, name);
    assert.match(result.stderr + JSON.stringify(result.payload), /search unavailable/);
    await fs.access(path.join(result.outputDir, 'screenshots/search-failed.png'));
    await assert.rejects(fs.access(path.join(result.outputDir, 'screenshots/search.png')));
    assert.equal(externalRequests.length, 0);
  }
  for (const [name, entry, method] of [['explicit-get', '/forms', 'get'], ['explicit-post', '/post', 'post']]) {
    const result = await run(name, entry, { action: `${origin}/results`, method, queryParam: 'q' });
    assert.equal(result.code, 0, result.stderr || JSON.stringify(result.payload));
    assert.equal(new URL(result.payload.result.page.url).pathname, '/results');
    assert.equal(typeof result.payload.result.hints, 'object');
    assert(result.payload.result.links.some(link => link.label === '星门'));
    const sent = requests.findLast(request => request.url.startsWith('/results'));
    const params = method === 'get' ? new URL(sent.url, origin).searchParams : new URLSearchParams(sent.body);
    assert.equal(params.get('q'), '星门');
    assert.equal(sent.method.toLowerCase(), method);
    assert.equal(externalRequests.length, 0);
  }
  const blocked = await run('blocked-script', '/', null, 'inspectPage');
  assert.equal(blocked.code, 0, blocked.stderr || JSON.stringify(blocked.payload));
  assert(blocked.payload.result.links.some(link => link.label === '星门'));
  await fs.access(path.join(blocked.outputDir, 'screenshots/explore.png'));
  console.log('6 worker regression checks passed; no external search requests.');
  await fs.rm(root, { recursive: true, force: true });
} catch (error) {
  console.error(`Worker regression evidence retained at ${root}`);
  throw error;
} finally {
  for (const socket of sockets) socket.destroy();
  await Promise.all([new Promise(resolve => site.close(resolve)), new Promise(resolve => external.close(resolve))]);
}
