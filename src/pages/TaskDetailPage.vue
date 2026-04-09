<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EvidenceGallery from '@/components/EvidenceGallery.vue';
import JsonPanel from '@/components/JsonPanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { formatDate } from '@/lib/format';
import { pickJsonSavePath } from '@/lib/tauri';
import type { TaskStatus } from '@/types/models';
import { useTasksStore } from '@/stores/tasks';

const route = useRoute();
const router = useRouter();
const tasksStore = useTasksStore();

const taskId = computed(() => String(route.params.taskId ?? ''));
let pollTimer: number | null = null;
let copyTimer: number | null = null;
let exportTimer: number | null = null;
const copyFeedback = ref('');
const exportFeedback = ref('');
const selectedExportPath = ref('');

onMounted(async () => {
  if (taskId.value) {
    await tasksStore.open(taskId.value);
    startPolling();
  }
});

onUnmounted(() => {
  stopPolling();
  if (copyTimer !== null) {
    window.clearTimeout(copyTimer);
  }
  if (exportTimer !== null) {
    window.clearTimeout(exportTimer);
  }
});

const validationEntries = computed(() => {
  const report = tasksStore.current?.validationReport;
  if (!report) {
    return [];
  }

  return [
    ['搜索', report.search],
    ['详情', report.bookInfo],
    ['目录', report.toc],
    ['正文', report.content],
    ['清洗', report.cleanup],
  ] as const;
});

const canContinue = computed(() => tasksStore.current?.status === 'failed');
const canStart = computed(() => tasksStore.current?.status === 'draft');
const canExport = computed(() => tasksStore.current?.status === 'completed');
const canDelete = computed(() => Boolean(tasksStore.current));
const isExecuting = computed(() => isActiveStatus(tasksStore.current?.status));
const exportPath = computed(() => {
  return selectedExportPath.value || tasksStore.artifacts?.output[0]?.path || 'output/book-source.json';
});
const suggestedExportName = computed(() => {
  const target = tasksStore.current?.targetUrl ?? 'book-source';
  try {
    const host = new URL(target).host.replace(/[^a-zA-Z0-9.-]+/g, '-');
    return `${host || 'book-source'}.json`;
  } catch {
    return 'book-source.json';
  }
});

const executionSummary = computed(() => {
  const task = tasksStore.current;
  if (!task) {
    return '';
  }

  if (task.status === 'failed') {
    return task.errorSummary || '任务失败，请查看日志与验证详情。';
  }

  return (
    {
      draft: '任务已创建，等待开始执行。',
      sampling: '正在采样页面、DOM 与网络请求。',
      synthesizing: '正在根据采样结果生成规则候选。',
      validating: '正在验证搜索、详情、目录与正文规则。',
      repairing: '正在根据失败原因尝试自动修复。',
      completed: '任务已完成，产物可导出。',
    } satisfies Record<Exclude<TaskStatus, 'failed'>, string>
  )[task.status];
});

async function startCurrentTask() {
  if (!taskId.value) {
    return;
  }

  await tasksStore.start(taskId.value);
}

function startPolling() {
  stopPolling();
  pollTimer = window.setInterval(async () => {
    if (!taskId.value) {
      return;
    }
    await tasksStore.open(taskId.value, { silent: true });
  }, 1500);
}

function stopPolling() {
  if (pollTimer !== null) {
    window.clearInterval(pollTimer);
    pollTimer = null;
  }
}

function isActiveStatus(status?: TaskStatus): boolean {
  return Boolean(status && ['sampling', 'synthesizing', 'validating', 'repairing'].includes(status));
}

function goBack() {
  router.push('/tasks');
}

function resetExportFeedbackLater() {
  if (exportTimer !== null) {
    window.clearTimeout(exportTimer);
  }
  exportTimer = window.setTimeout(() => {
    exportFeedback.value = '';
  }, 2400);
}

async function exportToFile() {
  if (!tasksStore.current) {
    return;
  }

  const destination = await pickJsonSavePath(suggestedExportName.value);
  if (!destination) {
    return;
  }

  selectedExportPath.value = destination;
  await tasksStore.exportCurrent(destination);
  if (!tasksStore.error) {
    exportFeedback.value = `已导出到 ${destination}`;
    resetExportFeedbackLater();
  }
}

async function copyExportedJson() {
  if (!tasksStore.exportedJson) {
    await tasksStore.exportCurrent();
  }

  if (!tasksStore.exportedJson) {
    copyFeedback.value = '暂时还没有可复制的 JSON。';
    return;
  }

  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(tasksStore.exportedJson);
    } else {
      const textarea = document.createElement('textarea');
      textarea.value = tasksStore.exportedJson;
      textarea.setAttribute('readonly', 'true');
      textarea.style.position = 'absolute';
      textarea.style.left = '-9999px';
      document.body.appendChild(textarea);
      textarea.select();
      document.execCommand('copy');
      document.body.removeChild(textarea);
    }
    copyFeedback.value = 'JSON 已复制到剪贴板。';
  } catch {
    copyFeedback.value = '复制失败，请稍后再试。';
  }

  if (copyTimer !== null) {
    window.clearTimeout(copyTimer);
  }
  copyTimer = window.setTimeout(() => {
    copyFeedback.value = '';
  }, 2000);
}

async function deleteCurrentTask() {
  if (!tasksStore.current) {
    return;
  }

  if (!window.confirm('确定删除这个任务以及对应证据吗？')) {
    return;
  }

  const currentId = tasksStore.current.id;
  await tasksStore.delete(currentId);
  if (!tasksStore.error) {
    router.push('/tasks');
  }
}
</script>

<template>
  <section class="page detail-page">
    <p
      v-if="tasksStore.error"
      class="error-copy"
    >
      {{ tasksStore.error }}
    </p>
    <p
      v-else-if="!tasksStore.current"
      class="muted-copy"
    >
      任务不存在或尚未加载。
    </p>

    <template v-else>
      <header class="page-hero detail-hero">
        <div class="hero-topline">
          <div class="hero-copy">
            <p class="eyebrow">
              Task Detail
            </p>
            <h2 class="detail-title">
              {{ tasksStore.current.targetUrl }}
            </h2>
            <p class="hero-lead">
              创建于 {{ formatDate(tasksStore.current.createdAt) }}，最近更新 {{ formatDate(tasksStore.current.updatedAt) }}。
            </p>
          </div>

          <div class="hero-actions">
            <button
              class="secondary-button"
              @click="goBack"
            >
              返回任务台
            </button>
            <button
              class="danger-button"
              :disabled="tasksStore.deleting || !canDelete"
              @click="deleteCurrentTask"
            >
              {{ tasksStore.deleting ? '删除中...' : '删除任务' }}
            </button>
            <StatusBadge :status="tasksStore.current.status" />
          </div>
        </div>
      </header>

      <div class="dashboard-grid single-column">
        <section class="panel export-panel">
          <div class="export-panel__header">
            <div class="panel-header">
              <h3>导出 JSON</h3>
              <p>导出结果会写入任务产物目录，同时这里提供一键复制与选择位置导出到文件。</p>
            </div>

            <div class="toolbar-actions">
              <button
                class="secondary-button"
                :disabled="tasksStore.exporting || !canExport"
                @click="copyExportedJson"
              >
                {{ tasksStore.exporting ? '准备中...' : '一键复制' }}
              </button>
              <button
                class="primary-button"
                :disabled="tasksStore.exporting || !canExport"
                @click="exportToFile"
              >
                {{ tasksStore.exporting ? '导出中...' : '导出到文件' }}
              </button>
            </div>
          </div>

          <div class="export-meta">
            <span>当前导出位置：{{ exportPath }}</span>
            <span v-if="copyFeedback">{{ copyFeedback }}</span>
            <span v-if="exportFeedback">{{ exportFeedback }}</span>
          </div>

          <pre
            v-if="tasksStore.exportedJson"
            class="code-block"
          >{{ tasksStore.exportedJson }}</pre>
          <p
            v-else
            class="muted-copy"
          >
            任务完成后可在这里查看最终导出的书源 JSON。
          </p>
        </section>

        <section class="panel">
          <header class="panel-header panel-header--row">
            <div>
              <h3>执行状态</h3>
              <p>{{ executionSummary }}</p>
            </div>

            <button
              v-if="canStart || canContinue"
              class="primary-button"
              :disabled="tasksStore.running"
              @click="startCurrentTask"
            >
              {{
                tasksStore.running
                  ? '执行中...'
                  : canContinue
                    ? '继续执行'
                    : '开始执行'
              }}
            </button>
          </header>

          <p
            v-if="isExecuting"
            class="muted-copy status-hint"
          >
            当前任务仍在后台运行中，页面会自动刷新状态、日志和产物。
          </p>

          <div
            v-if="validationEntries.length"
            class="phase-grid"
          >
            <article
              v-for="[label, phase] in validationEntries"
              :key="label"
              class="phase-card"
              :data-passed="phase.passed"
            >
              <div class="phase-card__head">
                <h4>{{ label }}</h4>
                <strong>{{ phase.passed ? '通过' : '失败' }}</strong>
              </div>
              <p>{{ phase.summary }}</p>
              <ul class="detail-list">
                <li
                  v-for="detail in phase.details"
                  :key="detail"
                >
                  {{ detail }}
                </li>
              </ul>
            </article>
          </div>
          <p
            v-else
            class="muted-copy"
          >
            当前阶段尚未产出验证报告，日志会持续更新。
          </p>
        </section>

        <EvidenceGallery :artifacts="tasksStore.artifacts" />

        <section class="panel">
          <header class="panel-header">
            <h3>运行日志</h3>
            <p>按任务状态机记录每一步的实际执行情况，便于追踪失败原因与修复过程。</p>
          </header>

          <div class="log-list">
            <article
              v-for="entry in tasksStore.current.logs"
              :key="`${entry.at}-${entry.message}`"
            >
              <span>{{ formatDate(entry.at) }}</span>
              <strong>{{ entry.stage }}</strong>
              <p>{{ entry.message }}</p>
            </article>
          </div>
        </section>

        <div class="panel-grid">
          <JsonPanel
            v-if="tasksStore.current.sourceCandidate"
            title="规则候选"
            :value="tasksStore.current.sourceCandidate"
          />
          <JsonPanel
            v-if="tasksStore.current.validationReport"
            title="验证报告"
            :value="tasksStore.current.validationReport"
          />
        </div>

        <section
          v-if="tasksStore.artifacts"
          class="panel"
        >
          <header class="panel-header">
            <h3>DOM / 抓包 / 输出文件</h3>
            <p>这里展示任务产物目录中的文本内容，方便快速回看采样与导出结果。</p>
          </header>

          <div class="artifact-columns">
            <div>
              <h4>DOM</h4>
              <pre
                v-for="item in tasksStore.artifacts.dom"
                :key="item.path"
                class="code-block compact"
              ><strong>{{ item.label }}</strong>
{{ item.contents }}</pre>
            </div>

            <div>
              <h4>网络样本</h4>
              <pre
                v-for="item in tasksStore.artifacts.network"
                :key="item.path"
                class="code-block compact"
              ><strong>{{ item.label }}</strong>
{{ item.contents }}</pre>
            </div>

            <div>
              <h4>输出</h4>
              <pre
                v-for="item in tasksStore.artifacts.output"
                :key="item.path"
                class="code-block compact"
              ><strong>{{ item.label }}</strong>
{{ item.contents }}</pre>
            </div>
          </div>
        </section>
      </div>
    </template>
  </section>
</template>
