<script setup lang="ts">
import { computed, onMounted, onUnmounted } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EvidenceGallery from '@/components/EvidenceGallery.vue';
import JsonPanel from '@/components/JsonPanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { formatDate } from '@/lib/format';
import type { TaskStatus } from '@/types/models';
import { useTasksStore } from '@/stores/tasks';

const route = useRoute();
const router = useRouter();
const tasksStore = useTasksStore();

const taskId = computed(() => String(route.params.taskId ?? ''));
let pollTimer: number | null = null;

onMounted(async () => {
  if (taskId.value) {
    await tasksStore.open(taskId.value);
    startPolling();
  }
});

onUnmounted(() => {
  stopPolling();
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
const isExecuting = computed(() => isActiveStatus(tasksStore.current?.status));

const executionSummary = computed(() => {
  const task = tasksStore.current;
  if (!task) {
    return '';
  }

  if (task.status === 'failed') {
    return task.errorSummary || '任务失败。';
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
</script>

<template>
  <section class="page detail-page">
    <button
      class="back-button"
      @click="goBack"
    >
      返回任务台
    </button>

    <header
      v-if="tasksStore.current"
      class="page-hero"
    >
      <p class="eyebrow">
        Task Detail
      </p>
      <div class="hero-inline">
        <h2>{{ tasksStore.current.targetUrl }}</h2>
        <StatusBadge :status="tasksStore.current.status" />
      </div>
      <p>
        创建于 {{ formatDate(tasksStore.current.createdAt) }}，最近更新
        {{ formatDate(tasksStore.current.updatedAt) }}。
      </p>
    </header>

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
      <div class="dashboard-grid single-column">
        <section class="panel">
          <header class="panel-header">
            <h3>执行状态</h3>
            <p>{{ executionSummary }}</p>
          </header>

          <div class="hero-inline">
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
            <p
              v-if="isExecuting"
              class="muted-copy"
            >
              正在后台执行，页面会自动刷新状态与日志。
            </p>
          </div>

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
              <h4>{{ label }}</h4>
              <strong>{{ phase.passed ? '通过' : '失败' }}</strong>
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
            <p>按任务状态机记录每一步实际执行情况。</p>
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

        <section class="panel">
          <header class="panel-header">
            <h3>导出 JSON</h3>
            <p>导出时会把最终书源写入任务产物目录的 `output/`。</p>
          </header>

          <button
            class="primary-button"
            :disabled="tasksStore.exporting || tasksStore.current.status !== 'completed'"
            @click="tasksStore.exportCurrent()"
          >
            {{ tasksStore.exporting ? '导出中...' : '导出书源 JSON' }}
          </button>

          <pre
            v-if="tasksStore.exportedJson"
            class="code-block"
          >{{ tasksStore.exportedJson }}</pre>
        </section>

        <section
          v-if="tasksStore.artifacts"
          class="panel"
        >
          <header class="panel-header">
            <h3>DOM / 抓包 / 输出文件</h3>
            <p>这里展示任务产物目录中的文本内容。</p>
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
