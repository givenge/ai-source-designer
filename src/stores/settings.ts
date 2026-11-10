import { defineStore } from 'pinia';
import type { AiSettings, ConnectionTestResult } from '@/types/models';
import { loadSettings, saveSettings, testAiConnection } from '@/lib/tauri';

const defaultSettings = (): AiSettings => ({
  baseUrl: 'mock://fixture',
  model: 'mock-fixture',
  apiKey: '',
});

export const useSettingsStore = defineStore('settings', {
  state: () => ({
    settings: defaultSettings() as AiSettings,
    hydrated: false,
    loading: false,
    saving: false,
    error: '' as string,
    savedSettings: null as AiSettings | null,
    testing: false,
    connection: null as ConnectionTestResult | null,
    connectionError: '',
    connectionSettings: null as AiSettings | null,
  }),
  getters: {
    ready(state) {
      return Boolean(state.settings.baseUrl.trim() && state.settings.model.trim());
    },
    saved(state): boolean {
      return JSON.stringify(state.savedSettings) === JSON.stringify(state.settings);
    },
    connectionStatus(state): 'unknown' | 'testing' | 'mock' | 'connected' | 'failed' {
      if (state.testing) return 'testing';
      if (JSON.stringify(state.connectionSettings) !== JSON.stringify(state.settings)) return 'unknown';
      if (state.connectionError) return 'failed';
      return state.connection ? (state.connection.mock ? 'mock' : 'connected') : 'unknown';
    },
  },
  actions: {
    async hydrate() {
      this.loading = true;
      this.error = '';

      try {
        const loaded = await withTimeout(loadSettings(), 3000, '读取桌面设置超时');
        this.settings = loaded;
        this.savedSettings = { ...loaded };
        this.hydrated = true;
      } catch (error) {
        this.error = `加载桌面设置失败：${errorMessage(error, '未知错误')}`;
        this.settings = defaultSettings();
        this.hydrated = true;
      } finally {
        this.loading = false;
      }
    },
    async persist() {
      this.saving = true;
      this.error = '';
      const nextSettings = {
        ...this.settings,
        baseUrl: this.settings.baseUrl.trim(),
        model: this.settings.model.trim(),
        apiKey: this.settings.apiKey.trim(),
      };
      this.settings = nextSettings;

      try {
        this.settings = await withTimeout(
          saveSettings(nextSettings),
          3000,
          '写入 settings.json 超时',
        );
        this.savedSettings = { ...this.settings };
      } catch (error) {
        this.error = `设置写入失败：${errorMessage(error, '未知错误')}`;
      } finally {
        this.saving = false;
      }
    },
    async testConnection() {
      if (!this.ready || this.testing) return;
      this.testing = true;
      this.connectionError = '';
      this.connection = null;
      const settings = { ...this.settings };
      try {
        this.connection = await testAiConnection(settings);
      } catch (error) {
        this.connectionError = errorMessage(error, '连接测试失败');
      } finally {
        this.connectionSettings = settings;
        this.testing = false;
      }
    },
  },
});

function withTimeout<T>(
  promise: Promise<T>,
  timeoutMs: number,
  message: string,
): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timer = window.setTimeout(() => reject(new Error(message)), timeoutMs);

    promise
      .then((value) => {
        window.clearTimeout(timer);
        resolve(value);
      })
      .catch((error) => {
        window.clearTimeout(timer);
        reject(error);
      });
  });
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error) {
    return error.message || fallback;
  }
  if (typeof error === 'string' && error.trim().length > 0) {
    return error;
  }
  return fallback;
}
