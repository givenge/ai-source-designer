import { describe, expect, it } from 'vitest';
import { artifactImageUrl, filterTasks, ruleChanges, siteLabel, taskDuration, taskProgress, tokenizeJson } from '@/lib/studio';
import type { BookSource, TaskRecord, ValidationPhase } from '@/types/models';

function task(overrides: Partial<TaskRecord> = {}): TaskRecord {
  return { id: 'task', status: 'draft', taskMode: 'agent', targetUrl: 'https://www.books.example',
    createdAt: '2026-11-10T00:00:00Z', updatedAt: '2026-11-10T00:00:10Z', logs: [], ...overrides };
}

describe('studio data', () => {
  it('filters active stages and searches URL or keyword without changing the records', () => {
    const records = [task({ id: 'a', status: 'sampling', testKeyword: '星门' }), task({ id: 'b', status: 'completed', targetUrl: 'https://library.example' })];
    expect(filterTasks(records, 'running', ' 星门 ')).toEqual([records[0]]);
    expect(filterTasks(records, 'all', 'LIBRARY')).toEqual([records[1]]);
    expect(filterTasks(records, 'failed', '')).toEqual([]);
    expect(records.map((record) => record.id)).toEqual(['a', 'b']);
    expect(siteLabel('fixture://novel-world')).toBe('novel-world');
    expect(siteLabel(records[0].targetUrl)).toBe('books.example');
  });

  it('counts all six validation stages including a failed discovery path', () => {
    const passed: ValidationPhase = { passed: true, summary: '', details: [], evidenceRefs: [] };
    expect(taskProgress(task())).toEqual({ passed: 0, total: 0 });
    expect(taskProgress(task({ validationReport: {
      explore: { ...passed, passed: false }, search: passed, bookInfo: passed, toc: passed, content: passed, cleanup: passed,
    } }))).toEqual({ passed: 5, total: 6 });
  });

  it('measures the latest revalidation instead of including idle time between runs', () => {
    const record = task({ status: 'completed', logs: [
      { at: '2026-11-10T00:00:00Z', stage: 'draft', message: '' },
      { at: '2026-11-10T00:02:00Z', stage: 'completed', message: '' },
      { at: '2026-11-10T01:00:00Z', stage: 'validating', message: '' },
      { at: '2026-11-10T01:00:03Z', stage: 'completed', message: '' },
    ] });
    expect(taskDuration(record)).toBe('3s');
    expect(taskDuration(task({ status: 'sampling', updatedAt: '2026-11-10T01:01:02Z', logs: record.logs.slice(0, 3) }))).toBe('1m 2s');
    expect(taskDuration(task({ updatedAt: 'invalid' }))).toBe('—');
  });

  it('shows added, removed and changed rules without modifying either source', () => {
    const before: BookSource = { bookSourceName: 'Book', bookSourceUrl: 'https://books.example', ruleContent: { content: '.old', replaceRegex: 'ad##' } };
    const after: BookSource = { ...before, ruleContent: { content: '.new', title: 'h1@text' } };
    const original = JSON.stringify([before, after]);
    expect(ruleChanges(before, after)).toEqual([
      { path: 'ruleContent.content', before: '".old"', after: '".new"' },
      { path: 'ruleContent.replaceRegex', before: '"ad##"', after: '∅' },
      { path: 'ruleContent.title', before: '∅', after: '"h1@text"' },
    ]);
    expect(JSON.stringify([before, after])).toBe(original);
    expect(ruleChanges(null, after)).toEqual([]);
  });

  it('highlights JSON keys, escapes and numbers without dropping or interpreting characters', () => {
    const code = JSON.stringify({ html: '<script>window.bad=true</script>', quote: '"', n: -2500, ok: true, empty: null }, null, 2).replace('-2500', '-2.5e+3');
    const tokens = tokenizeJson(code);
    expect(tokens.map((token) => token.text).join('')).toBe(code);
    expect(tokens.find((token) => token.text === '"html"')?.kind).toBe('key');
    expect(tokens.find((token) => token.text === '-2.5e+3')?.kind).toBe('number');
    expect(tokens.find((token) => token.text === 'true')?.kind).toBe('literal');
  });

  it('encodes fixture SVGs safely and preserves raster data', () => {
    const contents = '<svg xmlns="http://www.w3.org/2000/svg"><text># & 中文</text></svg>';
    expect(artifactImageUrl({ path: 'screenshots/a.svg', label: 'a.svg', contentType: 'image/svg+xml', contents })).toBe(`data:image/svg+xml;charset=utf-8,${encodeURIComponent(contents)}`);
    expect(artifactImageUrl({ path: 'screenshots/a.png', label: 'a.png', contentType: 'image/png', contents: 'YWJj' })).toBe('data:image/png;base64,YWJj');
  });
});
