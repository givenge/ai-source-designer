<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import EvidenceGallery from '@/components/EvidenceGallery.vue';
import JsonPanel from '@/components/JsonPanel.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { formatDate } from '@/lib/format';
import { confirmAction, pickJsonSavePath } from '@/lib/tauri';
import type { TaskStatus } from '@/types/models';
import { useTasksStore } from '@/stores/tasks';

const route = useRoute();
const router = useRouter();
const tasksStore = useTasksStore();

const taskId = computed(() => String(route.params.taskId ?? ''));
let pollTimer: number | null = null;
let streamTimer: number | null = null;
let typingTimer: number | null = null;
let waitTicker: number | null = null;
let resizeActive = false;
let resizeStartY = 0;
let resizeStartHeight = 520;
let chatMessageSeq = 0;
let copyTimer: number | null = null;
let exportTimer: number | null = null;
const copyFeedback = ref('');
const exportFeedback = ref('');
const selectedExportPath = ref('');
const chatInput = ref('');
const chatPanelOpen = ref(false);
const chatPanelHeight = ref(520);
const modelOutputText = ref('');
const waitingSeconds = ref(0);
const logView = ref<'task' | 'worker'>('task');
const chatMessages = ref<
  Array<{ id: number; role: 'user' | 'assistant'; content: string; applied?: boolean }>
>([]);

onMounted(async () => {
  if (taskId.value) {
    await tasksStore.open(taskId.value);
    startPolling();
  }
});

onUnmounted(() => {
  stopPolling();
  stopStreamPolling();
  if (typingTimer !== null) {
    window.clearInterval(typingTimer);
  }
  if (copyTimer !== null) {
    window.clearTimeout(copyTimer);
  }
  if (exportTimer !== null) {
    window.clearTimeout(exportTimer);
  }
  if (waitTicker !== null) {
    window.clearInterval(waitTicker);
  }
  window.removeEventListener('mousemove', onResizeMove);
  window.removeEventListener('mouseup', stopResize);
});

const validationEntries = computed(() => {
  const report = tasksStore.current?.validationReport;
  if (!report) {
    return [];
  }

  return [
    ['探索', report.explore],
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
const canChatRepair = computed(() => {
  const task = tasksStore.current;
  return Boolean(
    task &&
      task.sourceCandidate &&
      !isActiveStatus(task.status) &&
      task.status !== 'draft',
  );
});
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
      sampling:
        task.taskMode === 'agent'
          ? '正在由模型驱动 Playwright 多轮探索页面与证据。'
          : '正在采样页面、DOM 与网络请求。',
      synthesizing: '正在根据采样结果生成规则候选。',
      validating: '正在验证探索、搜索、详情、目录与正文规则。',
      repairing: '正在根据失败原因尝试自动修复。',
      completed: '任务已完成，产物可导出。',
    } satisfies Record<Exclude<TaskStatus, 'failed'>, string>
  )[task.status];
});

const validationSummary = computed(() => {
  const report = tasksStore.current?.validationReport;
  if (!report) {
    return { passed: 0, total: 0, rate: 0 };
  }
  const phases = [
    report.explore,
    report.search,
    report.bookInfo,
    report.toc,
    report.content,
    report.cleanup,
  ];
  const total = phases.length;
  const passed = phases.filter((phase) => phase.passed).length;
  return {
    passed,
    total,
    rate: Math.round((passed / total) * 100),
  };
});

const artifactStats = computed(() => {
  const manifest = tasksStore.artifacts?.manifest ?? tasksStore.current?.artifactManifest;
  return {
    screenshots: manifest?.screenshots.length ?? 0,
    dom: manifest?.dom.length ?? 0,
    network: manifest?.network.length ?? 0,
    logs: manifest?.logs.length ?? 0,
    output: manifest?.output.length ?? 0,
  };
});

const ruleCoverage = computed(() => {
  const source = tasksStore.current?.sourceCandidate?.bookSource;
  if (!source) {
    return { filled: 0, total: 6 };
  }
  const checkpoints = [
    source.exploreUrl,
    source.ruleSearch,
    source.ruleBookInfo,
    source.ruleToc,
    source.ruleContent,
    source.searchUrl,
  ];
  const filled = checkpoints.filter((value) => value !== null && value !== undefined && value !== '').length;
  return {
    filled,
    total: checkpoints.length,
  };
});

const keyRuleSnapshot = computed(() => {
  const source = tasksStore.current?.sourceCandidate?.bookSource;
  if (!source) {
    return [];
  }
  return [
    ['searchUrl', source.searchUrl || '空'],
    ['ruleSearch.bookList', source.ruleSearch?.bookList || '空'],
    ['ruleBookInfo.name', source.ruleBookInfo?.name || '空'],
    ['ruleToc.chapterList', source.ruleToc?.chapterList || '空'],
    ['ruleToc.nextTocUrl', source.ruleToc?.nextTocUrl || '空'],
    ['ruleContent.content', source.ruleContent?.content || '空'],
  ] as const;
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

function startStreamPolling() {
  stopStreamPolling();
  streamTimer = window.setInterval(async () => {
    if (!taskId.value) {
      return;
    }
    await tasksStore.open(taskId.value, { silent: true });
  }, 700);
}

function stopStreamPolling() {
  if (streamTimer !== null) {
    window.clearInterval(streamTimer);
    streamTimer = null;
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

  if (!await confirmAction('确定删除这个任务以及对应证据吗？')) {
    return;
  }

  const currentId = tasksStore.current.id;
  stopPolling();
  await tasksStore.delete(currentId);
  if (!tasksStore.error) {
    router.push('/tasks');
    return;
  }

  if (tasksStore.current?.id === currentId) {
    startPolling();
  }
}

async function sendChatRepair() {
  if (!taskId.value || !canChatRepair.value || tasksStore.chatting) {
    return;
  }

  const message = chatInput.value.trim();
  if (!message) {
    return;
  }

  chatMessages.value.push({
    id: ++chatMessageSeq,
    role: 'user',
    content: message,
  });
  chatInput.value = '';
  chatPanelOpen.value = true;
  waitingSeconds.value = 0;
  modelOutputText.value = '正在请求模型...';
  if (waitTicker !== null) {
    window.clearInterval(waitTicker);
  }
  waitTicker = window.setInterval(() => {
    waitingSeconds.value += 1;
    if (tasksStore.chatting) {
      modelOutputText.value = `正在请求模型，已等待 ${waitingSeconds.value}s ...`;
    }
  }, 1000);
  startStreamPolling();

  try {
    const result = await tasksStore.chatRepair(taskId.value, message);
    await streamTextToModelOutput(
      toDisplayModelOutput(result.modelRawOutput, result.reply),
    );
    const statusNote = result.task.status === 'completed'
      ? '（已通过验证）'
      : result.applied
        ? '（已应用但仍未通过验证）'
        : '（未应用规则变更）';
    await appendAssistantStream(
      `${result.reply || '已执行修复。'}${statusNote}`,
      result.applied,
    );
  } catch {
    modelOutputText.value = tasksStore.error || '修复请求失败。';
    await appendAssistantStream(tasksStore.error || '修复请求失败。', false);
  }
  finally {
    if (waitTicker !== null) {
      window.clearInterval(waitTicker);
      waitTicker = null;
    }
    stopStreamPolling();
  }
}

function toggleChatPanel() {
  chatPanelOpen.value = !chatPanelOpen.value;
}

function handleComposerKeydown(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    void sendChatRepair();
  }
}

function getArtifactLog(filename: string): string {
  return (
    tasksStore.artifacts?.logs.find(
      (item) => item.label === filename || item.path.endsWith(`/${filename}`) || item.path.endsWith(filename),
    )?.contents ?? ''
  );
}

const workerLogLines = computed(() =>
  getArtifactLog('worker.log')
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line.length > 0)
    .slice(-240),
);

async function appendAssistantStream(content: string, applied: boolean): Promise<void> {
  const message = {
    id: ++chatMessageSeq,
    role: 'assistant' as const,
    content: '',
    applied,
  };
  chatMessages.value.push(message);

  const chars = Array.from(content);
  let index = 0;
  await new Promise<void>((resolve) => {
    if (typingTimer !== null) {
      window.clearInterval(typingTimer);
    }
    typingTimer = window.setInterval(() => {
      index += 3;
      message.content = chars.slice(0, index).join('');
      if (index >= chars.length) {
        if (typingTimer !== null) {
          window.clearInterval(typingTimer);
          typingTimer = null;
        }
        resolve();
      }
    }, 14);
  });
}

async function streamTextToModelOutput(content: string): Promise<void> {
  modelOutputText.value = '';
  const chars = Array.from(content);
  let index = 0;
  await new Promise<void>((resolve) => {
    const timer = window.setInterval(() => {
      index += 4;
      modelOutputText.value = chars.slice(0, index).join('');
      if (index >= chars.length) {
        window.clearInterval(timer);
        resolve();
      }
    }, 12);
  });
}

function toDisplayModelOutput(rawOutput?: string | null, fallbackReply?: string): string {
  const trimmed = rawOutput?.trim();
  if (!trimmed) {
    return fallbackReply?.trim() || '无模型输出内容。';
  }

  try {
    const parsed = JSON.parse(trimmed) as {
      reply?: string;
      bookSource?: unknown;
      strategy?: string;
      confidence?: unknown;
    };
    const reply = parsed.reply?.trim() || fallbackReply?.trim() || '无 reply 字段。';
    const hasPatch = parsed.bookSource ? '包含规则补丁：是' : '包含规则补丁：否';
    const strategy = parsed.strategy ? `策略：${parsed.strategy}` : '';
    const confidence = parsed.confidence !== undefined ? `置信度：${String(parsed.confidence)}` : '';
    return [reply, hasPatch, strategy, confidence].filter((line) => line.length > 0).join('\n');
  } catch {
    return trimmed;
  }
}

function startResize(event: MouseEvent) {
  resizeActive = true;
  resizeStartY = event.clientY;
  resizeStartHeight = chatPanelHeight.value;
  window.addEventListener('mousemove', onResizeMove);
  window.addEventListener('mouseup', stopResize);
}

function onResizeMove(event: MouseEvent) {
  if (!resizeActive) {
    return;
  }
  const delta = resizeStartY - event.clientY;
  chatPanelHeight.value = Math.max(420, Math.min(920, resizeStartHeight + delta));
}

function stopResize() {
  resizeActive = false;
  window.removeEventListener('mousemove', onResizeMove);
  window.removeEventListener('mouseup', stopResize);
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
              创建于 {{ formatDate(tasksStore.current.createdAt) }}，最近更新 {{ formatDate(tasksStore.current.updatedAt) }}。当前模式：{{ tasksStore.current.taskMode === 'agent' ? 'Agent 全流程' : 'Classic 固定采样' }}。
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
        <section class="panel overview-panel">
          <header class="panel-header">
            <h3>任务概览</h3>
            <p>这里汇总验证通过率、产物规模和规则覆盖情况，避免空白首屏。</p>
          </header>

          <div class="overview-grid">
            <article class="overview-card">
              <span>验证通过率</span>
              <strong>{{ validationSummary.total ? `${validationSummary.rate}%` : '未验证' }}</strong>
              <p>{{ validationSummary.passed }} / {{ validationSummary.total }} 阶段通过</p>
            </article>
            <article class="overview-card">
              <span>规则覆盖</span>
              <strong>{{ ruleCoverage.filled }} / {{ ruleCoverage.total }}</strong>
              <p>search / explore / 详情 / 目录 / 正文</p>
            </article>
            <article class="overview-card">
              <span>证据产物</span>
              <strong>{{ artifactStats.dom + artifactStats.network + artifactStats.screenshots }}</strong>
              <p>DOM {{ artifactStats.dom }} / 抓包 {{ artifactStats.network }} / 截图 {{ artifactStats.screenshots }}</p>
            </article>
            <article class="overview-card">
              <span>导出与日志</span>
              <strong>{{ artifactStats.output }} / {{ artifactStats.logs }}</strong>
              <p>导出文件 {{ artifactStats.output }} 个，日志 {{ artifactStats.logs }} 个</p>
            </article>
          </div>

          <div
            v-if="keyRuleSnapshot.length"
            class="rule-snapshot"
          >
            <h4>关键规则快照</h4>
            <div class="rule-snapshot-grid">
              <article
                v-for="[label, value] in keyRuleSnapshot"
                :key="label"
              >
                <span>{{ label }}</span>
                <p>{{ value }}</p>
              </article>
            </div>
          </div>
        </section>

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
          <header class="panel-header panel-header--row">
            <div>
              <h3>运行日志</h3>
              <p>按任务状态机记录每一步执行情况，可切换查看 worker 原始日志。</p>
            </div>
            <div class="log-switch">
              <button
                class="secondary-button"
                :data-active="logView === 'task'"
                @click="logView = 'task'"
              >
                任务日志
              </button>
              <button
                class="secondary-button"
                :data-active="logView === 'worker'"
                @click="logView = 'worker'"
              >
                Worker 日志
              </button>
            </div>
          </header>

          <div
            v-if="logView === 'task'"
            class="log-list"
          >
            <article
              v-for="entry in tasksStore.current.logs"
              :key="`${entry.at}-${entry.message}`"
            >
              <span>{{ formatDate(entry.at) }}</span>
              <strong>{{ entry.stage }}</strong>
              <p>{{ entry.message }}</p>
            </article>
          </div>
          <div
            v-else
            class="worker-log-list"
          >
            <article
              v-for="(line, index) in workerLogLines"
              :key="`${index}-${line}`"
            >
              <p>{{ line }}</p>
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

      <section class="agent-chat-fab">
        <button
          class="agent-chat-fab__toggle"
          :data-open="chatPanelOpen"
          @click="toggleChatPanel"
        >
          {{ chatPanelOpen ? '收起 Agent' : 'Agent 修复' }}
        </button>

        <div
          v-if="chatPanelOpen"
          class="agent-chat-panel"
          :style="{ height: `${chatPanelHeight}px` }"
        >
          <div
            class="agent-chat-panel__resize-handle"
            @mousedown="startResize"
          />
          <header class="agent-chat-panel__header">
            <div>
              <h4>Agent 对话修复</h4>
              <p>仅显示大模型输出。</p>
            </div>
            <span>{{ tasksStore.chatting ? `执行中 ${waitingSeconds}s` : '空闲' }}</span>
          </header>

          <div class="model-process">
            <h5>过程输出</h5>
            <pre>{{ modelOutputText || '暂无模型输出。' }}</pre>
          </div>

          <div class="chat-history">
            <article
              v-for="(item, index) in chatMessages"
              :key="item.id"
              class="chat-item"
              :data-role="item.role"
            >
              <strong>{{ item.role === 'user' ? '你' : 'Agent' }}</strong>
              <p>{{ item.content }}</p>
              <span v-if="item.role === 'assistant'">{{ item.applied ? '已应用修复' : '仅回复，未改规则' }}</span>
            </article>
            <article
              v-if="tasksStore.chatting"
              class="chat-item"
              data-role="assistant"
            >
              <strong>Agent</strong>
              <p>模型生成中...</p>
            </article>
          </div>

          <div class="chat-composer">
            <div class="chat-input-wrap">
              <textarea
                v-model="chatInput"
                rows="3"
                placeholder="例如：修复 toc 选择器，避免 category/catalog，保持 book/chapter 路径。"
                @keydown="handleComposerKeydown"
              />
              <button
                class="chat-send-inside"
                :disabled="!canChatRepair || tasksStore.chatting || !chatInput.trim()"
                @click="sendChatRepair"
              >
                {{ tasksStore.chatting ? '修复中...' : '发送' }}
              </button>
            </div>
          </div>
        </div>
      </section>
    </template>
  </section>
</template>

<style scoped>
.overview-panel {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.82), rgba(255, 255, 255, 0.58)),
    rgba(245, 239, 227, 0.62);
}

.overview-grid {
  display: grid;
  gap: 0.9rem;
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.overview-card {
  border: 1px solid rgba(20, 34, 56, 0.12);
  border-radius: 14px;
  background: linear-gradient(160deg, rgba(255, 255, 255, 0.82), rgba(245, 250, 255, 0.58));
  padding: 0.8rem 0.9rem;
  min-width: 0;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.42);
}

.overview-card span {
  display: block;
  font-size: 0.76rem;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #84674c;
  margin-bottom: 0.3rem;
}

.overview-card strong {
  display: block;
  font-size: 1.35rem;
  line-height: 1.1;
  color: #1f2a37;
  overflow-wrap: anywhere;
}

.overview-card p {
  margin: 0.38rem 0 0;
  color: #5e6877;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.rule-snapshot {
  margin-top: 1rem;
  border: 1px solid rgba(20, 34, 56, 0.09);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.62);
  padding: 0.8rem 0.9rem;
}

.rule-snapshot h4 {
  margin: 0 0 0.65rem;
  font-size: 1rem;
}

.rule-snapshot-grid {
  display: grid;
  gap: 0.7rem;
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.rule-snapshot-grid article {
  border: 1px solid rgba(20, 34, 56, 0.08);
  border-radius: 12px;
  background: linear-gradient(165deg, rgba(255, 255, 255, 0.9), rgba(246, 251, 255, 0.76));
  padding: 0.6rem 0.68rem;
}

.rule-snapshot-grid span {
  display: block;
  font-size: 0.75rem;
  color: #7a889b;
  margin-bottom: 0.25rem;
}

.rule-snapshot-grid p {
  margin: 0;
  font-size: 0.86rem;
  line-height: 1.42;
  color: #26374d;
  word-break: break-word;
}

.agent-chat-fab {
  position: fixed;
  right: 1.1rem;
  bottom: 1.1rem;
  z-index: 40;
  display: grid;
  justify-items: end;
  gap: 0.55rem;
}

.agent-chat-fab__toggle {
  border: 1px solid rgba(20, 34, 56, 0.12);
  border-radius: 12px;
  background: linear-gradient(135deg, #13233b, #294a78);
  color: #fff;
  padding: 0.58rem 1rem;
  font-weight: 700;
  letter-spacing: 0.04em;
  cursor: pointer;
  box-shadow: 0 8px 24px rgba(17, 36, 62, 0.28);
  transition: transform 0.18s ease, box-shadow 0.18s ease;
}

.agent-chat-fab__toggle:hover {
  transform: translateY(-1px);
  box-shadow: 0 12px 28px rgba(17, 36, 62, 0.35);
}

.agent-chat-fab__toggle[data-open='true'] {
  background: linear-gradient(135deg, #263f68, #3f679f);
}

.agent-chat-panel {
  width: min(520px, calc(100vw - 2.2rem));
  max-height: min(88vh, 920px);
  border: 1px solid rgba(20, 34, 56, 0.08);
  border-radius: 16px;
  background: rgba(253, 254, 255, 0.95);
  backdrop-filter: blur(8px);
  box-shadow: 0 22px 56px rgba(20, 34, 56, 0.22);
  display: grid;
  grid-template-rows: 10px auto auto minmax(120px, 1fr) auto;
  overflow: hidden;
}

.agent-chat-panel__resize-handle {
  cursor: ns-resize;
  background: linear-gradient(
    to bottom,
    rgba(20, 34, 56, 0.25),
    rgba(20, 34, 56, 0.08)
  );
}

.agent-chat-panel__header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 0.75rem;
  padding: 0.85rem 1rem;
  border-bottom: 1px solid rgba(20, 34, 56, 0.1);
  background: linear-gradient(180deg, rgba(236, 242, 250, 0.92), rgba(244, 247, 252, 0.92));
}

.agent-chat-panel__header h4 {
  margin: 0;
}

.agent-chat-panel__header p {
  margin: 0.2rem 0 0;
  font-size: 0.82rem;
  color: #5f6f82;
}

.agent-chat-panel__header span {
  font-size: 0.78rem;
  color: #2f4a6b;
  background: rgba(53, 94, 152, 0.12);
  border: 1px solid rgba(53, 94, 152, 0.2);
  border-radius: 999px;
  padding: 0.2rem 0.5rem;
}

.chat-history {
  overflow: auto;
  display: grid;
  gap: 0.6rem;
  margin: 0 0 0.1rem;
  padding: 0.75rem 0.9rem;
  border-bottom: 1px solid rgba(20, 34, 56, 0.08);
  min-height: 0;
  background: rgba(251, 252, 254, 0.82);
}

.chat-item {
  padding: 0.5rem 0.6rem;
  border-radius: 12px;
  background: rgba(20, 34, 56, 0.045);
  border: 1px solid rgba(20, 34, 56, 0.08);
}

.chat-item[data-role='assistant'] {
  background: rgba(74, 130, 88, 0.11);
  border-color: rgba(74, 130, 88, 0.14);
}

.chat-item[data-role='user'] {
  background: rgba(61, 96, 150, 0.08);
  border-color: rgba(61, 96, 150, 0.15);
}

.chat-item p {
  margin: 0.35rem 0;
  white-space: pre-wrap;
  line-height: 1.5;
}

.chat-item span {
  font-size: 0.8rem;
  color: #5f6f82;
}

.chat-composer {
  display: block;
  padding: 0.75rem 0.9rem 0.9rem;
  background: rgba(247, 249, 253, 0.85);
}

.chat-input-wrap {
  position: relative;
}

.chat-composer textarea {
  width: 100%;
  min-height: 88px;
  resize: vertical;
  border: 1px solid rgba(20, 34, 56, 0.18);
  border-radius: 12px;
  padding: 0.6rem 4.7rem 0.6rem 0.75rem;
  background: rgba(255, 255, 255, 0.98);
  box-shadow: inset 0 1px 2px rgba(20, 34, 56, 0.06);
}

.chat-composer textarea:focus {
  outline: none;
  border-color: rgba(48, 96, 164, 0.55);
  box-shadow: 0 0 0 3px rgba(48, 96, 164, 0.13);
}

.chat-send-inside {
  position: absolute;
  right: 0.5rem;
  bottom: 0.5rem;
  border: none;
  border-radius: 10px;
  padding: 0.4rem 0.68rem;
  background: linear-gradient(135deg, #153057, #274f87);
  color: #fff;
  font-size: 0.82rem;
  font-weight: 700;
  cursor: pointer;
  box-shadow: 0 6px 14px rgba(27, 57, 98, 0.22);
}

.chat-send-inside:hover:not(:disabled) {
  filter: brightness(1.06);
}

.chat-send-inside:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.model-process {
  overflow: auto;
  border-bottom: 1px solid rgba(20, 34, 56, 0.08);
  padding: 0.55rem 0.9rem;
  background: linear-gradient(180deg, rgba(246, 249, 253, 0.93), rgba(250, 251, 254, 0.93));
  max-height: 160px;
}

.model-process h5 {
  margin: 0 0 0.38rem;
  font-size: 0.82rem;
  color: #314660;
}

.model-process pre {
  margin: 0;
  font-size: 0.77rem;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
  color: #283a51;
}

.log-switch {
  display: inline-flex;
  gap: 0.4rem;
  padding: 0.24rem;
  border-radius: 999px;
  background: rgba(20, 34, 56, 0.08);
  border: 1px solid rgba(20, 34, 56, 0.08);
}

.log-switch .secondary-button {
  border-radius: 999px;
  padding: 0.3rem 0.72rem;
  min-height: unset;
  font-size: 0.82rem;
}

.log-switch .secondary-button[data-active='true'] {
  background: rgba(20, 34, 56, 0.12);
  border-color: rgba(20, 34, 56, 0.25);
  color: #142238;
}

.worker-log-list {
  max-height: 440px;
  overflow: auto;
  display: grid;
  gap: 0.5rem;
}

.worker-log-list article {
  border: 1px solid rgba(20, 34, 56, 0.08);
  border-radius: 10px;
  padding: 0.45rem 0.6rem;
  background: rgba(248, 250, 252, 0.96);
}

.worker-log-list p {
  margin: 0;
  font-size: 0.84rem;
  line-height: 1.45;
  white-space: pre-wrap;
  word-break: break-word;
}

@media (max-width: 900px) {
  .overview-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .rule-snapshot-grid {
    grid-template-columns: 1fr;
  }

  .agent-chat-fab {
    right: 0.7rem;
    bottom: 0.7rem;
  }

  .agent-chat-panel {
    width: min(560px, calc(100vw - 1.2rem));
  }
}
</style>
