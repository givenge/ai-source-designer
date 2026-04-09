import {
  convertFileSrc,
  invoke,
  isTauri,
  type InvokeArgs,
} from '@tauri-apps/api/core';
import { save } from '@tauri-apps/plugin-dialog';
import type {
  AiSettings,
  ArtifactBundle,
  StartTaskPayload,
  TaskDraft,
  TaskRecord,
} from '@/types/models';

export function isTauriRuntime(): boolean {
  try {
    return isTauri();
  } catch {
    return false;
  }
}

async function invokeOrThrow<T>(
  command: string,
  args?: InvokeArgs,
): Promise<T> {
  if (!isTauriRuntime()) {
    throw new Error('请通过 `pnpm tauri dev` 启动桌面应用。');
  }

  return invoke<T>(command, args);
}

export async function loadSettings(): Promise<AiSettings> {
  return invokeOrThrow<AiSettings>('load_settings');
}

export async function saveSettings(settings: AiSettings): Promise<AiSettings> {
  return invokeOrThrow<AiSettings>('save_settings', { settings });
}

export async function listTasks(): Promise<TaskRecord[]> {
  return invokeOrThrow<TaskRecord[]>('list_tasks');
}

export async function createTask(draft: TaskDraft): Promise<TaskRecord> {
  return invokeOrThrow<TaskRecord>('create_task', { draft });
}

export async function deleteTask(taskId: string): Promise<void> {
  return invokeOrThrow<void>('delete_task', { taskId });
}

export async function getTask(taskId: string): Promise<TaskRecord> {
  return invokeOrThrow<TaskRecord>('get_task', { taskId });
}

export async function startTask(payload: StartTaskPayload): Promise<TaskRecord> {
  return invokeOrThrow<TaskRecord>('start_task', {
    taskId: payload.taskId,
    aiSettings: payload.aiSettings,
    apiKey: payload.apiKey,
  });
}

export async function getTaskArtifacts(taskId: string): Promise<ArtifactBundle> {
  return invokeOrThrow<ArtifactBundle>('get_task_artifacts', { taskId });
}

export async function exportTaskSource(
  taskId: string,
  outputPath?: string | null,
): Promise<string> {
  return invokeOrThrow<string>('export_task_source', { taskId, outputPath: outputPath ?? null });
}

export async function pickJsonSavePath(defaultPath: string): Promise<string | null> {
  if (!isTauriRuntime()) {
    throw new Error('请通过 `pnpm tauri dev` 启动桌面应用。');
  }

  const selected = await save({
    defaultPath,
    filters: [
      {
        name: 'JSON',
        extensions: ['json'],
      },
    ],
  });

  return typeof selected === 'string' ? selected : null;
}

export function toAssetUrl(path: string): string {
  return isTauriRuntime() ? convertFileSrc(path) : path;
}
