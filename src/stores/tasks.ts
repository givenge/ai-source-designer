import { defineStore } from 'pinia';
import {
  chatRepairTask as invokeChatRepairTask,
  createTask as invokeCreateTask,
  deleteTask as invokeDeleteTask,
  exportTaskSource,
  getTask as invokeGetTask,
  getTaskArtifacts as invokeGetTaskArtifacts,
  listTasks,
  revalidateTask as invokeRevalidateTask,
  startTask as invokeStartTask,
} from '@/lib/tauri';
import type { ArtifactBundle, TaskDraft, TaskRecord } from '@/types/models';
import { useSettingsStore } from '@/stores/settings';

export const useTasksStore = defineStore('tasks', {
  state: () => ({
    records: [] as TaskRecord[],
    current: null as TaskRecord | null,
    artifacts: null as ArtifactBundle | null,
    loading: false,
    running: false,
    runtimeRepairing: false,
    exporting: false,
    deleting: false,
    chatting: false,
    removedTaskIds: [] as string[],
    error: '' as string,
    exportedJson: '' as string,
    openedTaskId: '',
    openRequest: 0,
  }),
  actions: {
    async refresh() {
      this.loading = true;
      this.error = '';
      try {
        this.records = await listTasks();
      } catch (error) {
        this.error = errorMessage(error, '读取任务列表失败，请通过桌面应用运行。');
      } finally {
        this.loading = false;
      }
    },
    async open(taskId: string, options: { silent?: boolean; artifacts?: boolean } = {}) {
      if (this.removedTaskIds.includes(taskId) || (options.silent && this.openedTaskId !== taskId)) return;
      const request = ++this.openRequest;
      if (!options.silent) {
        this.loading = true;
        this.error = '';
        this.openedTaskId = taskId;
        if (this.current?.id !== taskId) {
          this.current = null;
          this.artifacts = null;
          this.exportedJson = '';
        }
      }
      try {
        const task = await invokeGetTask(taskId);
        if (this.openedTaskId !== taskId || request !== this.openRequest || this.removedTaskIds.includes(taskId)) return;
        this.current = task;
        this.records = this.records.map((record) => record.id === taskId ? task : record);
        if (options.artifacts !== false) {
          const artifacts = await invokeGetTaskArtifacts(taskId);
          if (this.openedTaskId === taskId && request === this.openRequest && !this.removedTaskIds.includes(taskId)) this.artifacts = artifacts;
        }
      } catch (error) {
        if (this.openedTaskId === taskId && request === this.openRequest) this.error = errorMessage(error, '读取任务详情失败。');
      } finally {
        if (this.openedTaskId === taskId && (!options.silent || request === this.openRequest)) {
          this.loading = false;
        }
      }
    },
    async create(draft: TaskDraft) {
      this.running = true;
      this.error = '';
      this.exportedJson = '';
      try {
        const record = await invokeCreateTask(draft);
        this.openedTaskId = record.id;
        this.openRequest += 1;
        this.current = record;
        this.artifacts = null;
        await this.refresh();
        return record;
      } finally {
        this.running = false;
      }
    },
    async start(taskId: string) {
      this.running = true;
      this.error = '';
      this.exportedJson = '';
      const settingsStore = useSettingsStore();

      try {
        if (this.current?.id === taskId) {
          this.current = {
            ...this.current,
            errorSummary: null,
          };
        }

        await invokeStartTask({
          taskId,
          aiSettings: {
            baseUrl: settingsStore.settings.baseUrl,
            model: settingsStore.settings.model,
          },
          apiKey: settingsStore.settings.apiKey,
        });

        if (this.removedTaskIds.includes(taskId)) {
          await this.refresh();
          return;
        }

        if (this.openedTaskId === taskId) await this.open(taskId, { silent: true });
        await this.refresh();
      } catch (error) {
        if (this.removedTaskIds.includes(taskId)) {
          await this.refresh();
          return;
        }

        if (this.openedTaskId === taskId) await this.open(taskId, { silent: true });
        await this.refresh();
        if (this.openedTaskId === taskId) this.error = errorMessage(error, '任务启动失败，请检查设置或运行环境。');
      } finally {
        this.running = false;
      }
    },
    async revalidate(taskId: string) {
      this.running = true;
      this.error = '';
      this.exportedJson = '';
      try {
        await invokeRevalidateTask(taskId);
        if (this.openedTaskId === taskId) await this.open(taskId, { silent: true });
        await this.refresh();
      } catch (error) {
        if (this.openedTaskId === taskId) this.error = errorMessage(error, '重新验证失败');
      } finally {
        this.running = false;
      }
    },
    async exportCurrent(outputPath?: string | null) {
      if (!this.current) {
        return;
      }
      const taskId = this.current.id;
      this.exporting = true;
      this.error = '';
      try {
        const json = await exportTaskSource(taskId, outputPath);
        if (this.openedTaskId === taskId) {
          this.exportedJson = json;
          await this.open(taskId, { silent: true });
        }
      } catch (error) {
        if (this.openedTaskId === taskId) this.error = errorMessage(error, '导出书源 JSON 失败。');
      } finally {
        this.exporting = false;
      }
    },
    async delete(taskId: string) {
      this.deleting = true;
      this.error = '';
      if (!this.removedTaskIds.includes(taskId)) {
        this.removedTaskIds = [...this.removedTaskIds, taskId];
      }

      try {
        await invokeDeleteTask(taskId);
        if (this.current?.id === taskId) {
          this.openedTaskId = '';
          this.openRequest += 1;
          this.current = null;
          this.artifacts = null;
          this.exportedJson = '';
        }
        await this.refresh();
      } catch (error) {
        this.removedTaskIds = this.removedTaskIds.filter((id) => id !== taskId);
        this.error = errorMessage(error, '删除任务失败。');
      } finally {
        this.deleting = false;
      }
    },
    async chatRepair(taskId: string, message: string) {
      this.chatting = true;
      this.error = '';
      if (this.openedTaskId === taskId) this.exportedJson = '';
      const settingsStore = useSettingsStore();
      try {
        const result = await invokeChatRepairTask({
          taskId,
          message,
          aiSettings: {
            baseUrl: settingsStore.settings.baseUrl,
            model: settingsStore.settings.model,
          },
          apiKey: settingsStore.settings.apiKey,
        });
        if (this.openedTaskId === taskId) await this.open(taskId, { silent: true });
        await this.refresh();
        return result;
      } catch (error) {
        if (this.openedTaskId === taskId) this.error = errorMessage(error, '对话修复失败。');
        throw error;
      } finally {
        this.chatting = false;
      }
    },
  },
});

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error) {
    return error.message || fallback;
  }
  if (typeof error === 'string' && error.trim().length > 0) {
    return error;
  }
  return fallback;
}
