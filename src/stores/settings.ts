import { defineStore } from 'pinia';
import type { AiSettings } from '@/types/models';
import { loadSettings, saveSettings } from '@/lib/tauri';

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
  }),
  getters: {
    ready(state) {
      return Boolean(state.settings.baseUrl && state.settings.model);
    },
  },
  actions: {
    async hydrate() {
      this.loading = true;
      this.error = '';

      try {
        const loaded = await withTimeout(loadSettings(), 3000, '读取桌面设置超时');
        this.settings = loaded;
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
        apiKey: this.settings.apiKey.trim(),
      };
      this.settings = nextSettings;

      try {
        this.settings = await withTimeout(
          saveSettings(nextSettings),
          3000,
          '写入 settings.json 超时',
        );
      } catch (error) {
        this.error = `设置写入失败：${errorMessage(error, '未知错误')}`;
      } finally {
        this.saving = false;
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
