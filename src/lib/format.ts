import type { TaskStatus } from '@/types/models';

export function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat('zh-CN', {
        dateStyle: 'medium',
        timeStyle: 'short',
      }).format(date);
}

export function maskSecret(value: string): string {
  if (!value) {
    return '';
  }

  if (value.length <= 8) {
    return `${value.slice(0, 2)}***`;
  }

  return `${value.slice(0, 4)}••••${value.slice(-3)}`;
}

export function statusLabel(status: TaskStatus): string {
  return (
    {
      draft: '草稿',
      sampling: '采样中',
      synthesizing: '生成中',
      validating: '验证中',
      repairing: '修复中',
      completed: '已完成',
      failed: '失败',
    } satisfies Record<TaskStatus, string>
  )[status];
}

export function statusTone(status: TaskStatus): string {
  return (
    {
      draft: 'muted',
      sampling: 'info',
      synthesizing: 'accent',
      validating: 'warning',
      repairing: 'warning',
      completed: 'success',
      failed: 'danger',
    } satisfies Record<TaskStatus, string>
  )[status];
}
