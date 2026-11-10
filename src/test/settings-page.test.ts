import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createApp, nextTick, type App } from 'vue';
import { createPinia, setActivePinia } from 'pinia';
import SettingsPage from '@/pages/SettingsPage.vue';
import { useSettingsStore } from '@/stores/settings';
import * as desktop from '@/lib/tauri';

vi.mock('@/lib/tauri', () => ({
  loadSettings: vi.fn(), saveSettings: vi.fn(), testAiConnection: vi.fn(), getRuntimeDiagnostics: vi.fn(), openDataDirectory: vi.fn(),
  repairPlaywright: vi.fn(), getRuntimeRepairLog: vi.fn(),
}));

let app: App;
let pinia: ReturnType<typeof createPinia>;
const custom = { baseUrl: 'https://custom.example/v1', model: 'my-model', apiKey: 'test-private-key' };

beforeEach(() => {
  vi.resetAllMocks();
  pinia = createPinia();
  setActivePinia(pinia);
  const store = useSettingsStore();
  store.hydrated = true;
  store.settings = { ...custom };
  store.savedSettings = { ...custom };
  vi.mocked(desktop.getRuntimeDiagnostics).mockResolvedValue({
    version: '0.1.0', dataDir: '/tmp/qa', dbPath: '/tmp/qa/db', settingsPath: '/tmp/qa/settings',
    workerScript: '/tmp/qa/worker', workerReady: false, databaseReady: true, nodeVersion: null, browserVersion: null, error: null,
    nodePath: null, playwrightPath: null, playwrightVersion: null,
  });
  vi.mocked(desktop.getRuntimeRepairLog).mockResolvedValue('');
  document.body.innerHTML = '<div id="mount"></div>';
  app = createApp(SettingsPage).use(pinia);
  app.mount('#mount');
});
afterEach(() => { app.unmount(); document.body.innerHTML = ''; });

async function click(label: string) {
  const button = [...document.querySelectorAll<HTMLButtonElement>('button')].find((item) => item.textContent?.trim() === label || (item.classList.contains('provider-preset') && item.textContent?.includes(label)));
  expect(button, `button ${label}`).toBeDefined();
  button!.click();
  await nextTick();
}

describe('provider presets', () => {
  it('previews and cancels a preset without replacing the endpoint, model or key', async () => {
    const store = useSettingsStore();
    await click('OpenAI');
    expect(document.querySelector('.provider-preview')?.textContent).toContain('https://api.openai.com/v1');
    expect(store.settings).toEqual(custom);
    await click('取消');
    expect(document.querySelector('.provider-preview')).toBeNull();
    expect(store.settings).toEqual(custom);
    expect(desktop.saveSettings).not.toHaveBeenCalled();
  });

  it('applies only on request and clears a key belonging to the old endpoint', async () => {
    const store = useSettingsStore();
    await click('DeepSeek');
    expect(store.settings).toEqual(custom);
    await click('应用预设');
    expect(store.settings).toEqual({ baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat', apiKey: '' });
    expect(store.savedSettings).toEqual(custom);
    expect(desktop.saveSettings).not.toHaveBeenCalled();
  });

  it('restores the last saved settings including its key without writing to disk', async () => {
    await click('Ollama');
    await click('应用预设');
    await click('恢复已保存');
    expect(useSettingsStore().settings).toEqual(custom);
    expect(useSettingsStore().saved).toBe(true);
    expect(desktop.saveSettings).not.toHaveBeenCalled();
  });

  it('retains the key when applying a preset for the same endpoint', async () => {
    const store = useSettingsStore();
    store.settings.baseUrl = 'https://api.openai.com/v1/';
    await nextTick();
    await click('OpenAI');
    expect(store.settings.model).toBe('my-model');
    await click('应用预设');
    expect(store.settings.apiKey).toBe(custom.apiKey);
  });
});

describe('runtime repair', () => {
  it('shows a verified repair without changing or saving AI settings', async () => {
    const original = await vi.mocked(desktop.getRuntimeDiagnostics).mock.results[0].value;
    vi.mocked(desktop.repairPlaywright).mockResolvedValue({ ...original, workerReady: true, error: null });
    vi.mocked(desktop.getRuntimeRepairLog).mockResolvedValue('安装 Playwright\n验证 Chromium 启动\n修复完成');
    await click('一键修复');
    await nextTick();
    expect(desktop.repairPlaywright).toHaveBeenCalledTimes(1);
    expect(document.body.textContent).toContain('修复完成 · Chromium 启动通过');
    expect(useSettingsStore().settings).toEqual(custom);
    expect(desktop.saveSettings).not.toHaveBeenCalled();
  });

  it('shows installation failures and retains the repair log', async () => {
    vi.mocked(desktop.repairPlaywright).mockRejectedValue(new Error('Chromium 下载超时'));
    vi.mocked(desktop.getRuntimeRepairLog).mockResolvedValue('download failed');
    await click('一键修复');
    for (let index = 0; index < 8; index += 1) await nextTick();
    expect(document.body.textContent).toContain('Chromium 下载超时');
    expect(document.body.textContent).toContain('download failed');
    expect(document.body.textContent).not.toContain('Chromium 启动通过');
  });

  it('reloads the last repair log even when runtime detection fails', async () => {
    app.unmount();
    vi.mocked(desktop.getRuntimeDiagnostics).mockRejectedValue(new Error('系统 Node.js 未找到'));
    vi.mocked(desktop.getRuntimeRepairLog).mockResolvedValue('上次修复：Chromium 下载失败');
    app = createApp(SettingsPage).use(pinia);
    app.mount('#mount');
    for (let index = 0; index < 8; index += 1) await nextTick();
    expect(document.body.textContent).toContain('系统 Node.js 未找到');
    expect(document.body.textContent).toContain('上次修复：Chromium 下载失败');
  });
});
