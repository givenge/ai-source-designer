import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp, nextTick, type App } from 'vue';
import { createMemoryHistory, createRouter, type Router } from 'vue-router';
import { createPinia, setActivePinia } from 'pinia';
import TaskDetailPage from '@/pages/TaskDetailPage.vue';
import * as desktop from '@/lib/tauri';
import type { TaskRecord } from '@/types/models';

vi.mock('@/lib/tauri', () => ({
  getTask: vi.fn(), getTaskArtifacts: vi.fn(), listTasks: vi.fn(), chatRepairTask: vi.fn(),
  loadSettings: vi.fn(), saveSettings: vi.fn(), testAiConnection: vi.fn(),
  startTask: vi.fn(), revalidateTask: vi.fn(), exportTaskSource: vi.fn(), deleteTask: vi.fn(),
  confirmAction: vi.fn(), exportTaskLog: vi.fn(), pickJsonSavePath: vi.fn(), pickLogSavePath: vi.fn(),
}));
vi.mock('@/components/ImageViewerModal.vue', () => ({ default: { render: () => null } }));

let app: App;
let router: Router;
function task(id: string): TaskRecord {
  return { id, status: 'completed', taskMode: 'agent', targetUrl: `https://${id}.example`,
    createdAt: '2026-11-10T00:00:00Z', updatedAt: '2026-11-10T00:00:10Z', logs: [],
    sourceCandidate: { strategy: 'fixture', confidence: 1, evidenceRefs: [], repairHistory: [],
      bookSource: { bookSourceName: id, bookSourceUrl: `https://${id}.example`, enabled: true } } };
}
async function flush() {
  for (let index = 0; index < 8; index += 1) await nextTick();
}
async function send() {
  const input = document.querySelector<HTMLTextAreaElement>('[aria-label="修复需求"]')!;
  input.value = '修复旧任务';
  input.dispatchEvent(new Event('input', { bubbles: true }));
  await nextTick();
  const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.trim() === '发送')!;
  button.click();
  await flush();
}

beforeEach(async () => {
  vi.resetAllMocks();
  vi.useFakeTimers();
  vi.mocked(desktop.getTask).mockImplementation(async (id) => task(id));
  vi.mocked(desktop.getTaskArtifacts).mockResolvedValue({ manifest: { dom: [], network: [], screenshots: [], output: [], logs: [] }, dom: [], network: [], screenshots: [], output: [], logs: [] });
  vi.mocked(desktop.listTasks).mockResolvedValue([]);
  const pinia = createPinia();
  setActivePinia(pinia);
  router = createRouter({ history: createMemoryHistory(), routes: [
    { path: '/tasks', component: { render: () => null } },
    { path: '/tasks/:taskId', component: TaskDetailPage },
  ] });
  await router.push('/tasks/a');
  document.body.innerHTML = '<div id="mount"></div>';
  app = createApp(TaskDetailPage).use(pinia).use(router);
  app.mount('#mount');
  await flush();
  document.querySelector<HTMLButtonElement>('.repair-dock__toggle')!.click();
  await nextTick();
});
afterEach(() => { app.unmount(); vi.useRealTimers(); document.body.innerHTML = ''; });

describe('task repair navigation', () => {
  it('stops old output and clears the old prompt when switching tasks', async () => {
    vi.mocked(desktop.chatRepairTask).mockResolvedValue({ reply: '旧任务回复', modelRawOutput: '旧输出'.repeat(200), applied: false, task: task('a') });
    await send();
    await vi.advanceTimersByTimeAsync(20);
    expect(document.querySelector('.repair-output')?.textContent).toContain('旧输出');
    await router.push('/tasks/b');
    await flush();
    await vi.advanceTimersByTimeAsync(200);
    expect(document.querySelector('.repair-output')?.textContent).not.toContain('旧输出');
    expect(document.querySelector('.chat-history')?.textContent).not.toContain('旧任务');
    expect(document.querySelector('.repair-prompts')?.textContent).not.toContain('重试');
  });

  it('ignores a late reply after leaving and returning to the same task', async () => {
    let resolve!: (value: Awaited<ReturnType<typeof desktop.chatRepairTask>>) => void;
    vi.mocked(desktop.chatRepairTask).mockReturnValue(new Promise((done) => { resolve = done; }));
    await send();
    await router.push('/tasks/b');
    await flush();
    await router.push('/tasks/a');
    await flush();
    resolve({ reply: '旧请求回复', modelRawOutput: '旧请求输出', applied: false, task: task('a') });
    await flush();
    await vi.advanceTimersByTimeAsync(200);
    expect(document.querySelector('.repair-output')?.textContent).not.toContain('旧请求');
    expect(document.querySelector('.chat-history')?.textContent).not.toContain('旧请求');
  });
});
