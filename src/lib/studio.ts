import type { ArtifactFile, BookSource, TaskRecord } from '@/types/models';

export function siteLabel(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '') || url;
  } catch {
    return url;
  }
}

export function taskProgress(task: TaskRecord): { passed: number; total: number } {
  const report = task.validationReport;
  if (!report) return { passed: 0, total: 0 };
  const phases = [report.explore, report.search, report.bookInfo, report.toc, report.content, report.cleanup];
  return { passed: phases.filter((phase) => phase.passed).length, total: phases.length };
}

export function isTaskActive(task: TaskRecord | null): boolean {
  return Boolean(task && ['sampling', 'synthesizing', 'validating', 'repairing'].includes(task.status));
}

export function filterTasks(tasks: TaskRecord[], filter: string, query: string): TaskRecord[] {
  const needle = query.trim().toLocaleLowerCase();
  return tasks.filter((task) => {
    const matchesStatus = filter === 'all' || (filter === 'running' ? isTaskActive(task) : task.status === filter);
    return matchesStatus && `${task.targetUrl} ${task.testKeyword ?? ''}`.toLocaleLowerCase().includes(needle);
  });
}

export function taskDuration(task: TaskRecord): string {
  const terminal = (stage: string) => ['completed', 'failed'].includes(stage);
  let endIndex = task.logs.length;
  if (terminal(task.status)) {
    while (endIndex > 0 && !terminal(task.logs[endIndex - 1].stage)) endIndex -= 1;
    endIndex = endIndex ? endIndex - 1 : task.logs.length;
  }
  let previousEnd = endIndex - 1;
  while (previousEnd >= 0 && !terminal(task.logs[previousEnd].stage)) previousEnd -= 1;
  const runLogs = task.logs.slice(previousEnd + 1, endIndex);
  const started = [...runLogs].reverse().find((entry) => entry.stage === 'draft')?.at ?? runLogs[0]?.at ?? task.createdAt;
  const ended = task.logs[endIndex]?.at ?? task.updatedAt;
  const seconds = Math.round((Date.parse(ended) - Date.parse(started)) / 1000);
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}

export function artifactImageUrl(file: ArtifactFile): string {
  return file.contentType === 'image/svg+xml'
    ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(file.contents)}`
    : `data:${file.contentType};base64,${file.contents}`;
}

export interface RuleChange { path: string; before: string; after: string }

export function ruleChanges(before: BookSource | null, after: BookSource | null): RuleChange[] {
  if (!before || !after) return [];
  const changes: RuleChange[] = [];
  function visit(left: unknown, right: unknown, path: string) {
    if (left && right && typeof left === 'object' && typeof right === 'object' && !Array.isArray(left) && !Array.isArray(right)) {
      const a = left as Record<string, unknown>;
      const b = right as Record<string, unknown>;
      for (const key of new Set([...Object.keys(a), ...Object.keys(b)])) visit(a[key], b[key], path ? `${path}.${key}` : key);
    } else if (JSON.stringify(left) !== JSON.stringify(right)) {
      changes.push({ path, before: JSON.stringify(left) ?? '∅', after: JSON.stringify(right) ?? '∅' });
    }
  }
  visit(before, after, '');
  return changes;
}

export interface CodeToken { text: string; kind: string }

export function tokenizeJson(code: string): CodeToken[] {
  const tokens: CodeToken[] = [];
  const pattern = /"(?:\\.|[^"\\])*"|\b(?:true|false|null)\b|-?\b\d+(?:\.\d+)?(?:[eE][+-]?\d+)?\b/g;
  let offset = 0;
  for (const match of code.matchAll(pattern)) {
    const index = match.index!;
    if (index > offset) tokens.push({ text: code.slice(offset, index), kind: '' });
    const value = match[0];
    const kind = value.startsWith('"')
      ? (code.slice(index + value.length).trimStart().startsWith(':') ? 'key' : 'string')
      : ['true', 'false', 'null'].includes(value) ? 'literal' : 'number';
    tokens.push({ text: value, kind });
    offset = index + value.length;
  }
  if (offset < code.length) tokens.push({ text: code.slice(offset), kind: '' });
  return tokens;
}
