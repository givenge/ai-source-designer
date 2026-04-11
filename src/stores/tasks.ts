import { defineStore } from 'pinia';
import {
  chatRepairTask as invokeChatRepairTask,
  createTask as invokeCreateTask,
  deleteTask as invokeDeleteTask,
  exportTaskSource,
  getTask as invokeGetTask,
  getTaskArtifacts as invokeGetTaskArtifacts,
  listTasks,
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
    exporting: false,
    deleting: false,
    chatting: false,
    removedTaskIds: [] as string[],
    error: '' as string,
    exportedJson: '' as string,
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
    async open(taskId: string, options: { silent?: boolean } = {}) {
      if (!options.silent) {
        this.loading = true;
        this.error = '';
      }
      try {
        this.current = await invokeGetTask(taskId);
        this.artifacts = await invokeGetTaskArtifacts(taskId);
      } catch (error) {
        this.error = errorMessage(error, '读取任务详情失败。');
      } finally {
        if (!options.silent) {
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
        this.current = record;
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

        const result = await invokeStartTask({
          taskId,
          aiSettings: {
            baseUrl: settingsStore.settings.baseUrl,
            model: settingsStore.settings.model,
          },
          apiKey: settingsStore.settings.apiKey || null,
        });

        if (this.removedTaskIds.includes(taskId)) {
          await this.refresh();
          return;
        }

        this.current = result;
        this.artifacts = await invokeGetTaskArtifacts(taskId);
        await this.refresh();
      } catch (error) {
        if (this.removedTaskIds.includes(taskId)) {
          await this.refresh();
          return;
        }

        this.error = errorMessage(error, '任务启动失败，请检查设置或运行环境。');
        if (this.current?.id === taskId) {
          try {
            this.current = await invokeGetTask(taskId);
            this.artifacts = await invokeGetTaskArtifacts(taskId);
            await this.refresh();
          } catch {
            // ignore secondary refresh errors and preserve primary failure message
          }
        }
      } finally {
        this.running = false;
      }
    },
    async exportCurrent(outputPath?: string | null) {
      if (!this.current) {
        return;
      }

      this.exporting = true;
      this.error = '';
      try {
        this.exportedJson = await exportTaskSource(this.current.id, outputPath);
        this.artifacts = await invokeGetTaskArtifacts(this.current.id);
      } catch (error) {
        this.error = errorMessage(error, '导出书源 JSON 失败。');
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
      const settingsStore = useSettingsStore();
      try {
        const result = await invokeChatRepairTask({
          taskId,
          message,
          aiSettings: {
            baseUrl: settingsStore.settings.baseUrl,
            model: settingsStore.settings.model,
          },
          apiKey: settingsStore.settings.apiKey || null,
        });
        this.current = result.task;
        this.artifacts = await invokeGetTaskArtifacts(taskId);
        await this.refresh();
        return result;
      } catch (error) {
        this.error = errorMessage(error, '对话修复失败。');
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
