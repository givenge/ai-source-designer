import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createPinia, setActivePinia } from 'pinia';
import * as desktop from '@/lib/tauri';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';
import type { ArtifactBundle, TaskRecord } from '@/types/models';

vi.mock('@/lib/tauri', () => ({
  loadSettings: vi.fn(), saveSettings: vi.fn(), testAiConnection: vi.fn(), listTasks: vi.fn(), createTask: vi.fn(),
  getTask: vi.fn(), getTaskArtifacts: vi.fn(), startTask: vi.fn(), revalidateTask: vi.fn(), exportTaskSource: vi.fn(),
  deleteTask: vi.fn(), chatRepairTask: vi.fn(),
}));

function task(id: string): TaskRecord {
  return { id, status: 'completed', taskMode: 'agent', targetUrl: `https://${id}.example`,
    createdAt: '2026-11-10T00:00:00Z', updatedAt: '2026-11-10T00:00:10Z', logs: [] };
}
function artifacts(id: string): ArtifactBundle {
  return { manifest: { dom: [], network: [], screenshots: [], output: [], logs: [] }, screenshots: [], network: [], logs: [], output: [],
    dom: [{ path: `dom/${id}.html`, label: `${id}.html`, contents: id, contentType: 'text/html' }] };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

beforeEach(() => {
  setActivePinia(createPinia());
  vi.resetAllMocks();
  vi.mocked(desktop.getTask).mockImplementation(async (id) => task(id));
  vi.mocked(desktop.getTaskArtifacts).mockImplementation(async (id) => artifacts(id));
  vi.mocked(desktop.listTasks).mockResolvedValue([]);
});

describe('task navigation', () => {
  it('ignores a detail response from the previously opened task', async () => {
    const store = useTasksStore();
    const pending = deferred<TaskRecord>();
    vi.mocked(desktop.getTask).mockReturnValueOnce(pending.promise);
    const oldRead = store.open('a');
    await store.open('b');
    pending.resolve(task('a'));
    await oldRead;
    expect(store.current?.id).toBe('b');
    expect(store.artifacts?.dom[0].contents).toBe('b');
  });

  it('ignores stale reads even when the task ID is the same', async () => {
    const store = useTasksStore();
    const pending = deferred<TaskRecord>();
    vi.mocked(desktop.getTask).mockReturnValueOnce(pending.promise);
    const oldRead = store.open('a');
    await store.open('a');
    pending.resolve({ ...task('a'), updatedAt: '2026-11-10T00:00:01Z' });
    await oldRead;
    expect(store.current?.updatedAt).toBe('2026-11-10T00:00:10Z');
  });

  it('does not attach late artifacts to another task', async () => {
    const store = useTasksStore();
    const pending = deferred<ArtifactBundle>();
    vi.mocked(desktop.getTaskArtifacts).mockReturnValueOnce(pending.promise);
    const oldRead = store.open('a');
    await Promise.resolve();
    await store.open('b');
    pending.resolve(artifacts('a'));
    await oldRead;
    expect(store.artifacts?.dom[0].contents).toBe('b');
  });

  it.each(['start', 'revalidate', 'repair'] as const)('keeps the current task when a background %s completes', async (kind) => {
    const store = useTasksStore();
    await store.open('a');
    const pending = deferred<TaskRecord>();
    vi.mocked(desktop.startTask).mockReturnValue(pending.promise);
    vi.mocked(desktop.revalidateTask).mockReturnValue(pending.promise);
    vi.mocked(desktop.chatRepairTask).mockImplementation(async () => ({ reply: 'done', applied: true, task: await pending.promise }));
    const operation = kind === 'repair' ? store.chatRepair('a', '修复') : kind === 'start' ? store.start('a') : store.revalidate('a');
    await store.open('b');
    pending.resolve(task('a'));
    await operation;
    expect(store.current?.id).toBe('b');
    expect(store.artifacts?.dom[0].contents).toBe('b');
    if (kind === 'start') expect(vi.mocked(desktop.startTask).mock.calls[0][0].apiKey).toBe('');
  });

  it('clears old evidence when a new task is created', async () => {
    const store = useTasksStore();
    await store.open('a');
    vi.mocked(desktop.createTask).mockResolvedValue(task('new'));
    await store.create({ targetUrl: 'fixture://novel-world' });
    expect(store.openedTaskId).toBe('new');
    expect(store.artifacts).toBeNull();
  });

  it('does not restore a deleted task when an artifact read finishes', async () => {
    const store = useTasksStore();
    const pending = deferred<ArtifactBundle>();
    vi.mocked(desktop.getTaskArtifacts).mockReturnValueOnce(pending.promise);
    const oldRead = store.open('a');
    await Promise.resolve();
    await store.delete('a');
    pending.resolve(artifacts('a'));
    await oldRead;
    expect(store.current).toBeNull();
    expect(store.artifacts).toBeNull();
  });

  it('does not show an old export on another task', async () => {
    const store = useTasksStore();
    await store.open('a');
    const pending = deferred<string>();
    vi.mocked(desktop.exportTaskSource).mockReturnValueOnce(pending.promise);
    const exporting = store.exportCurrent('/tmp/a.json');
    await store.open('b');
    pending.resolve('{"task":"a"}');
    await exporting;
    expect(store.current?.id).toBe('b');
    expect(store.exportedJson).toBe('');
    expect(desktop.exportTaskSource).toHaveBeenCalledWith('a', '/tmp/a.json');
  });

  it('preserves the primary start error after refreshing the task and list', async () => {
    const store = useTasksStore();
    await store.open('a');
    vi.mocked(desktop.startTask).mockRejectedValue(new Error('Worker 未启动'));
    await store.start('a');
    expect(store.error).toBe('Worker 未启动');
  });
});

describe('connection status', () => {
  it('discards the meaning of a connection test when settings change', async () => {
    const store = useSettingsStore();
    store.settings = { baseUrl: 'http://localhost:1234/v1', model: 'local', apiKey: '' };
    vi.mocked(desktop.testAiConnection).mockResolvedValue({ model: 'local', latencyMs: 12, mock: false });
    await store.testConnection();
    expect(store.connectionStatus).toBe('connected');
    store.settings.baseUrl = 'https://different.example/v1';
    expect(store.connectionStatus).toBe('unknown');
  });
});
