<script setup lang="ts">
import { computed, nextTick, onUnmounted, reactive, ref, watch } from 'vue';
import { useRoute, useRouter } from 'vue-router';
import AppIcon from '@/components/icons/AppIcon.vue';
import CodeViewer from '@/components/CodeViewer.vue';
import EvidenceGallery from '@/components/EvidenceGallery.vue';
import ImageViewerModal from '@/components/ImageViewerModal.vue';
import SegmentedControl from '@/components/SegmentedControl.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { copyText } from '@/lib/clipboard';
import { formatDate } from '@/lib/format';
import { isTaskActive, ruleChanges, siteLabel, taskProgress } from '@/lib/studio';
import { confirmAction, exportTaskLog, pickJsonSavePath, pickLogSavePath } from '@/lib/tauri';
import type { ArtifactFile, BookSource, EvidenceRef } from '@/types/models';
import { useTasksStore } from '@/stores/tasks';

const route = useRoute();
const router = useRouter();
const tasks = useTasksStore();
const taskId = computed(() => String(route.params.taskId ?? ''));
const workspace = ref('rules');
const selectedPhase = ref('explore');
const evidenceKind = ref('screenshots');
const selectedFile = reactive<Record<string, string>>({ dom: '', network: '', output: '' });
const previewImage = ref<ArtifactFile | null>(null);
const jsonView = ref('source');
const logView = ref('task');
const logQuery = ref('');
const autoScroll = ref(true);
const logOffset = reactive<Record<string, number>>({ task: 0, worker: 0 });
const feedback = ref('');
const uiError = ref('');
const repairOpen = ref(false);
const repairHeight = ref(320);
const repairView = ref('output');
const repairInput = ref('');
const repairBusy = ref(false);
const modelOutput = ref('');
const waitingSeconds = ref(0);
const beforeRepair = ref<BookSource | null>(null);
const afterRepair = ref<BookSource | null>(null);
const lastPrompt = ref('');
const chatMessages = ref<Array<{ id: number; role: 'user' | 'assistant'; content: string; applied?: boolean }>>([]);
let messageId = 0;
let pollTimer: number | undefined;
let waitTimer: number | undefined;
let feedbackTimer: number | undefined;
let outputTimer: number | undefined;
let finishOutput: (() => void) | undefined;
let disposed = false;
let taskRevision = 0;
let resizeStartY = 0;
let resizeStartHeight = 320;

const workspaceOptions = [
  { value: 'rules', label: '规则与验证', icon: 'code' },
  { value: 'evidence', label: '采集证据', icon: 'image' },
  { value: 'terminal', label: '执行终端', icon: 'terminal' },
] as const;
const evidenceOptions = [
  { value: 'screenshots', label: '截图' }, { value: 'dom', label: 'DOM' },
  { value: 'network', label: '网络样本' }, { value: 'output', label: '输出文件' },
];
const logOptions = [{ value: 'task', label: '任务日志' }, { value: 'worker', label: 'Worker 日志' }];
const repairOptions = [{ value: 'output', label: '模型输出' }, { value: 'diff', label: '规则变化' }];
const prompts = [
  { label: '正文过滤', text: '修复正文段落过滤，保留正文并移除已验证的导航和广告。' },
  { label: '目录分页', text: '根据采样证据修复下一页目录规则，并实际验证分页。' },
  { label: '书海分类', text: '根据站内导航补全书海分类，并验证翻页能返回新书籍。' },
];
const definitions = [
  { key: 'explore', role: 'explore', label: '书海' },
  { key: 'search', role: 'search', label: '搜索' },
  { key: 'bookInfo', role: 'book', label: '详情' },
  { key: 'toc', role: 'toc', label: '目录' },
  { key: 'content', role: 'content', label: '正文' },
  { key: 'cleanup', role: 'content', label: '清洗' },
] as const;
const phases = computed(() => definitions.map((definition) => ({ ...definition, result: tasks.current?.validationReport?.[definition.key] })));
const progress = computed(() => tasks.current ? taskProgress(tasks.current) : { passed: 0, total: 0 });
const busy = computed(() => tasks.running || tasks.chatting || tasks.runtimeRepairing || repairBusy.value || isTaskActive(tasks.current));
const canExport = computed(() => tasks.current?.status === 'completed');
const canRepair = computed(() => Boolean(tasks.current?.sourceCandidate && !busy.value && tasks.current.status !== 'draft'));
const source = computed(() => tasks.current?.sourceCandidate?.bookSource ?? null);
const sourceJson = computed(() => {
  const candidate = source.value ? JSON.stringify(source.value, null, 2) : '';
  return canExport.value ? tasks.artifacts?.output.find((file) => file.label === 'book-source.json')?.contents || tasks.exportedJson || candidate : candidate;
});
const shownJson = computed(() => jsonView.value === 'source' ? sourceJson.value : JSON.stringify(jsonView.value === 'candidate' ? tasks.current?.sourceCandidate : tasks.current?.validationReport, null, 2) || '');
const jsonTitle = computed(() => ({ source: 'book-source.json', candidate: 'source-candidate.json', report: 'validation-report.json' })[jsonView.value as 'source' | 'candidate' | 'report']);
const ruleSnapshot = computed(() => {
  const value = source.value;
  return value ? [
    ['searchUrl', value.searchUrl || '∅'], ['ruleExplore.bookList', value.ruleExplore?.bookList || '∅'],
    ['ruleSearch.bookList', value.ruleSearch?.bookList || '∅'], ['ruleBookInfo.name', value.ruleBookInfo?.name || '∅'],
    ['ruleToc.chapterList', value.ruleToc?.chapterList || '∅'], ['ruleContent.content', value.ruleContent?.content || '∅'],
  ] : [];
});
const preferredRole = computed(() => definitions.find((phase) => phase.key === selectedPhase.value)?.role || '');
const artifactFiles = computed(() => evidenceKind.value === 'screenshots' ? [] : tasks.artifacts?.[evidenceKind.value as 'dom' | 'network' | 'output'] ?? []);
const currentFile = computed(() => artifactFiles.value.find((file) => file.path === selectedFile[evidenceKind.value]) || artifactFiles.value.find((file) => file.label === `${preferredRole.value}.html`) || artifactFiles.value[0]);
const workerRows = computed(() => (tasks.artifacts?.logs.find((file) => file.label === 'worker.log')?.contents ?? '').split(/\r?\n/).filter(Boolean));
const taskRows = computed(() => (tasks.current?.logs ?? []).map((entry) => `${formatDate(entry.at)}  [${entry.stage}]  ${entry.message}`));
const terminalRows = computed(() => (logView.value === 'worker' ? workerRows.value : taskRows.value).slice(logOffset[logView.value]).filter((line) => line.toLowerCase().includes(logQuery.value.trim().toLowerCase())));
const terminalCode = computed(() => terminalRows.value.join('\n'));
const changes = computed(() => ruleChanges(beforeRepair.value, afterRepair.value));

watch(taskId, async (id) => {
  const revision = ++taskRevision;
  window.clearInterval(pollTimer);
  window.clearInterval(waitTimer);
  stopOutputStream();
  repairBusy.value = false;
  repairInput.value = '';
  lastPrompt.value = '';
  waitingSeconds.value = 0;
  previewImage.value = null;
  chatMessages.value = [];
  beforeRepair.value = null;
  afterRepair.value = null;
  modelOutput.value = '';
  logOffset.task = 0;
  logOffset.worker = 0;
  await tasks.open(id);
  if (taskRevision !== revision || disposed) return;
  selectedPhase.value = phases.value.find((phase) => phase.result && !phase.result.passed)?.key || 'explore';
  pollTimer = window.setInterval(() => {
    if (!tasks.loading && (busy.value || tasks.chatting)) {
      void tasks.open(id, { silent: true, artifacts: workspace.value === 'evidence' || (workspace.value === 'terminal' && logView.value === 'worker') });
    }
  }, 1500);
}, { immediate: true });
watch([workspace, logView], () => {
  if (workspace.value === 'evidence' || (workspace.value === 'terminal' && logView.value === 'worker')) void tasks.open(taskId.value, { silent: true });
});
onUnmounted(() => {
  disposed = true;
  window.clearInterval(pollTimer);
  window.clearInterval(waitTimer);
  window.clearTimeout(feedbackTimer);
  stopOutputStream();
  stopResize();
});

function notify(message: string) {
  feedback.value = message;
  window.clearTimeout(feedbackTimer);
  feedbackTimer = window.setTimeout(() => { feedback.value = ''; }, 2800);
}
async function copySource() {
  uiError.value = '';
  try { await copyText(sourceJson.value); notify('JSON 已复制'); }
  catch (error) { uiError.value = error instanceof Error ? error.message : '复制失败'; }
}
async function saveSource() {
  uiError.value = '';
  try {
    const path = await pickJsonSavePath(`${siteLabel(tasks.current?.targetUrl ?? 'book-source')}.json`);
    if (!path) return;
    await tasks.exportCurrent(path);
    if (!tasks.error) notify('书源已导出');
  } catch (error) { uiError.value = error instanceof Error ? error.message : '导出失败'; }
}
async function saveLog() {
  uiError.value = '';
  try {
    const path = await pickLogSavePath(`${siteLabel(tasks.current?.targetUrl ?? 'task')}-${logView.value}.log`);
    if (!path) return;
    await exportTaskLog(taskId.value, logView.value as 'task' | 'worker', path);
    notify('日志已导出');
  } catch (error) { uiError.value = error instanceof Error ? error.message : '导出日志失败'; }
}
function clearScreen() { logOffset[logView.value] = logView.value === 'worker' ? workerRows.value.length : taskRows.value.length; }
async function removeTask() {
  if (!await confirmAction('删除此任务及全部采样证据？此操作无法撤销。')) return;
  await tasks.delete(taskId.value);
  if (!tasks.error) await router.push('/tasks');
}
async function focusPhase(key: string) {
  selectedPhase.value = key;
  workspace.value = 'rules';
  await nextTick();
  document.getElementById(`phase-${key}`)?.focus({ preventScroll: true });
  document.getElementById(`phase-${key}`)?.scrollIntoView({ block: 'nearest' });
}
function phaseToggle(event: Event, key: string) {
  const open = (event.target as HTMLDetailsElement).open;
  if (open) selectedPhase.value = key;
  else if (selectedPhase.value === key) selectedPhase.value = '';
}
async function openEvidence(evidence: EvidenceRef) {
  workspace.value = 'evidence';
  const kind = evidence.artifactPath.split('/')[0];
  if (kind === 'screenshots') {
    evidenceKind.value = 'screenshots';
    previewImage.value = tasks.artifacts?.screenshots.find((file) => file.path === evidence.artifactPath) ?? null;
  } else if (['dom', 'network', 'output'].includes(kind)) {
    evidenceKind.value = kind;
    selectedFile[kind] = evidence.artifactPath;
  }
}

function stopOutputStream() {
  window.clearInterval(outputTimer);
  finishOutput?.();
  finishOutput = undefined;
}
function streamModelOutput(text: string): Promise<void> {
  stopOutputStream();
  modelOutput.value = '';
  return new Promise((resolve) => {
    let offset = 0;
    finishOutput = resolve;
    outputTimer = window.setInterval(() => {
      offset += 128;
      modelOutput.value = text.slice(0, offset);
      if (offset >= text.length || disposed) stopOutputStream();
    }, 20);
  });
}
function displayOutput(raw: string | null | undefined, reply: string) {
  const text = raw?.trim() || reply;
  try {
    const value = JSON.parse(text);
    return typeof value === 'object' && value !== null ? JSON.stringify(value, null, 2) : text;
  } catch { return text; }
}
async function sendRepair() {
  if (!canRepair.value || !repairInput.value.trim()) return;
  const id = taskId.value;
  const revision = taskRevision;
  const message = repairInput.value.trim();
  lastPrompt.value = message;
  repairInput.value = '';
  repairBusy.value = true;
  waitingSeconds.value = 0;
  repairView.value = 'output';
  modelOutput.value = '等待模型响应…';
  beforeRepair.value = source.value ? JSON.parse(JSON.stringify(source.value)) : null;
  afterRepair.value = null;
  chatMessages.value.push({ id: ++messageId, role: 'user', content: message });
  waitTimer = window.setInterval(() => { waitingSeconds.value += 1; }, 1000);
  try {
    const result = await tasks.chatRepair(id, message);
    if (disposed || taskRevision !== revision) return;
    afterRepair.value = result.task.sourceCandidate?.bookSource ? JSON.parse(JSON.stringify(result.task.sourceCandidate.bookSource)) : null;
    await streamModelOutput(displayOutput(result.modelRawOutput, result.reply));
    if (!disposed && taskRevision === revision) chatMessages.value.push({ id: ++messageId, role: 'assistant', content: result.reply, applied: result.applied });
  } catch (error) {
    if (!disposed && taskRevision === revision) {
      const message = tasks.error || (error instanceof Error ? error.message : '修复失败');
      modelOutput.value = message;
      chatMessages.value.push({ id: ++messageId, role: 'assistant', content: message, applied: false });
    }
  } finally {
    if (taskRevision === revision) {
      window.clearInterval(waitTimer);
      repairBusy.value = false;
    }
  }
}
function composerKey(event: KeyboardEvent) {
  if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); void sendRepair(); }
}
function startResize(event: MouseEvent) {
  resizeStartY = event.clientY;
  resizeStartHeight = repairHeight.value;
  window.addEventListener('mousemove', resizeMove);
  window.addEventListener('mouseup', stopResize);
}
function resizeMove(event: MouseEvent) { repairHeight.value = Math.max(220, Math.min(window.innerHeight * 0.5, resizeStartHeight + resizeStartY - event.clientY)); }
function stopResize() { window.removeEventListener('mousemove', resizeMove); window.removeEventListener('mouseup', stopResize); }
function resizeKey(event: KeyboardEvent) {
  if (!['ArrowUp', 'ArrowDown'].includes(event.key)) return;
  event.preventDefault();
  repairHeight.value = Math.max(220, Math.min(window.innerHeight * 0.5, repairHeight.value + (event.key === 'ArrowUp' ? 24 : -24)));
}
</script>

<template>
  <section class="detail-page">
    <div
      v-if="!tasks.current"
      class="panel empty-state"
    >
      <AppIcon
        name="loader"
        :class="{ spinning: tasks.loading }"
        :size="28"
      /><strong>{{ tasks.loading ? '读取任务中' : '未找到任务' }}</strong><p
        v-if="tasks.error"
        class="error-copy"
        role="alert"
      >
        {{ tasks.error }}
      </p><RouterLink
        to="/tasks"
        class="secondary-button"
      >
        返回任务台
      </RouterLink>
    </div>
    <template v-else>
      <header class="task-action-header">
        <div class="task-action-header__title">
          <RouterLink
            to="/tasks"
            class="icon-button"
            title="返回任务台"
            aria-label="返回任务台"
          >
            <AppIcon name="arrowLeft" />
          </RouterLink><div>
            <h1 :title="tasks.current.targetUrl">
              {{ tasks.current.targetUrl }}
            </h1><div class="detail-meta">
              <span>创建 {{ formatDate(tasks.current.createdAt) }}</span><span>更新 {{ formatDate(tasks.current.updatedAt) }}</span><span class="tiny-badge">{{ tasks.current.taskMode === 'agent' ? 'Agent' : 'Classic' }}</span>
            </div>
          </div>
        </div>
        <div class="toolbar-actions">
          <StatusBadge :status="tasks.current.status" />
          <span
            v-if="progress.total"
            class="progress-label"
            :class="progress.passed === progress.total ? 'text-success' : 'text-warning'"
          >{{ progress.passed }}/{{ progress.total }}</span>
          <button
            v-if="['draft', 'failed'].includes(tasks.current.status)"
            class="primary-button"
            type="button"
            :disabled="busy"
            @click="tasks.start(taskId)"
          >
            <AppIcon
              :name="busy ? 'loader' : 'play'"
              :class="{ spinning: busy }"
              :size="16"
            />{{ tasks.current.status === 'draft' ? '开始执行' : '继续执行' }}
          </button>
          <button
            v-if="tasks.current.sourceCandidate"
            class="secondary-button"
            type="button"
            :disabled="busy"
            @click="tasks.revalidate(taskId)"
          >
            <AppIcon
              :name="busy ? 'loader' : 'refresh'"
              :class="{ spinning: busy }"
              :size="16"
            />重新验证
          </button>
          <button
            class="icon-button"
            type="button"
            :disabled="!canExport"
            aria-label="复制书源 JSON"
            title="复制书源 JSON"
            @click="copySource"
          >
            <AppIcon name="copy" />
          </button>
          <button
            class="secondary-button"
            type="button"
            :disabled="!canExport || tasks.exporting"
            @click="saveSource"
          >
            <AppIcon
              name="download"
              :size="16"
            />导出书源
          </button>
          <button
            class="icon-button danger-button"
            type="button"
            :disabled="tasks.deleting || repairBusy"
            title="删除任务"
            aria-label="删除任务"
            @click="removeTask"
          >
            <AppIcon
              name="trash"
              :size="17"
            />
          </button>
        </div>
      </header>
      <div
        v-if="tasks.error || uiError || tasks.current.errorSummary || feedback"
        class="detail-feedback"
        :class="{ 'text-danger': tasks.error || uiError || tasks.current.errorSummary }"
        role="status"
      >
        {{ tasks.error || uiError || tasks.current.errorSummary || feedback }}
      </div>

      <div
        class="pipeline-track"
        aria-label="验证流水线"
      >
        <button
          v-for="(phase, index) in phases"
          :key="phase.key"
          type="button"
          :data-state="phase.result ? phase.result.passed ? 'passed' : 'failed' : 'pending'"
          :data-selected="selectedPhase === phase.key"
          @click="focusPhase(phase.key)"
        >
          <span class="pipeline-index">{{ String(index + 1).padStart(2, '0') }}</span><strong>{{ phase.label }}</strong><AppIcon
            :name="phase.result ? phase.result.passed ? 'check' : 'alert' : 'clock'"
            :size="16"
          /><span class="sr-only">{{ phase.result ? phase.result.passed ? '通过' : '失败' : '待验证' }}</span>
        </button>
      </div>

      <section class="studio-workspace">
        <header class="workspace-toolbar">
          <SegmentedControl
            id="workspace"
            v-model="workspace"
            :options="[...workspaceOptions]"
            label="任务工作区"
            tabs
          /><span
            class="workspace-id"
            :title="tasks.current.id"
          >#{{ tasks.current.id.slice(0, 8) }}</span>
        </header>

        <section
          id="workspace-panel-rules"
          class="workspace-panel"
          role="tabpanel"
          aria-labelledby="workspace-tab-rules"
          :hidden="workspace !== 'rules'"
        >
          <div
            v-if="workspace === 'rules'"
            class="rules-layout"
          >
            <section class="rules-pane">
              <div class="pane-heading">
                <h2>书源规则</h2><select
                  v-model="jsonView"
                  class="compact-select"
                  aria-label="JSON 视图"
                >
                  <option value="source">
                    书源 JSON
                  </option><option value="candidate">
                    候选记录
                  </option><option value="report">
                    验证报告
                  </option>
                </select>
              </div>
              <CodeViewer
                :title="jsonTitle"
                :code="shownJson"
                language="json"
              />
              <details
                v-if="ruleSnapshot.length"
                class="rule-peek"
              >
                <summary>关键规则</summary><dl>
                  <div
                    v-for="[label, value] in ruleSnapshot"
                    :key="label"
                  >
                    <dt>{{ label }}</dt><dd :title="value">
                      {{ value }}
                    </dd>
                  </div>
                </dl>
              </details>
            </section>
            <section class="validation-pane">
              <div class="pane-heading">
                <h2>验证结果</h2><span class="muted-copy">{{ progress.total ? `${progress.passed}/${progress.total} 阶段` : '待验证' }}</span>
              </div>
              <div class="validation-list">
                <details
                  v-for="phase in phases"
                  :key="phase.key"
                  class="phase-report"
                  :data-state="phase.result ? phase.result.passed ? 'passed' : 'failed' : 'pending'"
                  :data-selected="selectedPhase === phase.key"
                  :open="selectedPhase === phase.key"
                  @toggle="phaseToggle($event, phase.key)"
                >
                  <summary
                    :id="`phase-${phase.key}`"
                    tabindex="0"
                  >
                    <AppIcon
                      :name="phase.result ? phase.result.passed ? 'check' : 'alert' : 'clock'"
                      :size="17"
                    /><strong>{{ phase.label }}</strong><span>{{ phase.result ? phase.result.passed ? '通过' : '失败' : '待验证' }}</span><AppIcon
                      name="chevronDown"
                      :size="15"
                    />
                  </summary>
                  <div
                    v-if="phase.result"
                    class="phase-report__details"
                  >
                    <p>{{ phase.result.summary }}</p><ul>
                      <li
                        v-for="(detail, index) in phase.result.details"
                        :key="index"
                      >
                        {{ detail }}
                      </li>
                    </ul><div
                      v-if="phase.result.evidenceRefs.length"
                      class="phase-evidence"
                    >
                      <button
                        v-for="evidence in phase.result.evidenceRefs"
                        :key="evidence.artifactPath"
                        type="button"
                        class="text-button"
                        :title="evidence.artifactPath"
                        @click="openEvidence(evidence)"
                      >
                        <AppIcon
                          name="file"
                          :size="13"
                        />{{ evidence.artifactPath.split('/').pop() }}
                      </button>
                    </div>
                  </div>
                  <div
                    v-else
                    class="phase-report__details muted-copy"
                  >
                    暂无验证数据
                  </div>
                </details>
              </div>
            </section>
          </div>
        </section>

        <section
          id="workspace-panel-evidence"
          class="workspace-panel"
          role="tabpanel"
          aria-labelledby="workspace-tab-evidence"
          :hidden="workspace !== 'evidence'"
        >
          <div
            v-if="workspace === 'evidence'"
            class="evidence-workspace"
          >
            <div class="pane-heading">
              <SegmentedControl
                v-model="evidenceKind"
                :options="evidenceOptions"
                label="证据类型"
              /><span class="muted-copy">{{ evidenceKind === 'screenshots' ? tasks.artifacts?.screenshots.length ?? 0 : artifactFiles.length }} 个文件</span>
            </div>
            <EvidenceGallery
              v-if="evidenceKind === 'screenshots'"
              :artifacts="tasks.artifacts"
              :highlight-role="preferredRole"
              @select="previewImage = $event"
            />
            <div
              v-else-if="artifactFiles.length"
              class="file-workspace"
            >
              <nav
                class="file-list"
                aria-label="采样文件"
              >
                <button
                  v-for="file in artifactFiles"
                  :key="file.path"
                  type="button"
                  :data-selected="currentFile?.path === file.path"
                  :title="file.path"
                  @click="selectedFile[evidenceKind] = file.path"
                >
                  <AppIcon
                    name="file"
                    :size="15"
                  /><span>{{ file.label }}</span>
                </button>
              </nav><CodeViewer
                v-if="currentFile"
                :key="currentFile.path"
                :title="currentFile.label"
                :code="currentFile.contents"
                :language="currentFile.contentType.includes('json') || currentFile.label.endsWith('.json') ? 'json' : 'text'"
              />
            </div>
            <div
              v-else
              class="empty-state"
            >
              <AppIcon
                name="folder"
                :size="32"
              /><strong>暂无文件</strong>
            </div>
          </div>
        </section>

        <section
          id="workspace-panel-terminal"
          class="workspace-panel"
          role="tabpanel"
          aria-labelledby="workspace-tab-terminal"
          :hidden="workspace !== 'terminal'"
        >
          <div
            v-if="workspace === 'terminal'"
            class="terminal-workspace"
          >
            <div class="terminal-toolbar">
              <SegmentedControl
                v-model="logView"
                :options="logOptions"
                label="日志来源"
              />
              <label class="search-field"><AppIcon
                name="search"
                :size="15"
              /><input
                v-model="logQuery"
                type="search"
                aria-label="搜索日志"
                placeholder="搜索日志"
              ></label>
              <label class="checkbox-field"><input
                v-model="autoScroll"
                type="checkbox"
              >自动滚动</label>
              <button
                v-if="logOffset[logView]"
                class="text-button"
                type="button"
                @click="logOffset[logView] = 0"
              >
                恢复日志
              </button>
              <button
                class="icon-button"
                type="button"
                title="清屏，保留原始日志"
                aria-label="清屏"
                @click="clearScreen"
              >
                <AppIcon
                  name="x"
                  :size="16"
                />
              </button>
              <button
                class="icon-button"
                type="button"
                title="导出原始日志"
                aria-label="导出日志"
                @click="saveLog"
              >
                <AppIcon
                  name="download"
                  :size="16"
                />
              </button>
            </div>
            <CodeViewer
              :title="logView === 'worker' ? 'worker.log' : 'task.log'"
              :code="terminalCode"
              :auto-scroll="autoScroll"
            />
            <div class="terminal-footer">
              <span>{{ terminalRows.length }} 条记录</span><span v-if="logOffset[logView]">已清屏</span>
            </div>
          </div>
        </section>
      </section>

      <section
        class="repair-dock"
        :data-open="repairOpen"
      >
        <div
          v-if="repairOpen"
          class="repair-dock__resize"
          role="separator"
          tabindex="0"
          aria-label="调整修复面板高度"
          aria-orientation="horizontal"
          @mousedown="startResize"
          @keydown="resizeKey"
        />
        <header class="repair-dock__header">
          <button
            class="repair-dock__toggle"
            type="button"
            :aria-expanded="repairOpen"
            @click="repairOpen = !repairOpen"
          >
            <AppIcon
              name="sparkles"
              :size="17"
            /><strong>Agent 修复</strong><AppIcon
              :name="repairOpen ? 'chevronDown' : 'chevronUp'"
              :size="16"
            />
          </button><span
            v-if="repairBusy"
            class="repair-status"
          ><span
            class="status-dot"
            data-state="testing"
          />修复中 {{ waitingSeconds }}s</span><span
            v-else-if="!canRepair"
            class="muted-copy"
          >{{ tasks.runtimeRepairing ? '运行环境修复中' : tasks.chatting ? '任务修复中' : busy ? '任务执行中' : '生成规则后可修复' }}</span>
        </header>
        <div
          v-if="repairOpen"
          class="repair-dock__body"
          :style="{ height: `${repairHeight}px` }"
        >
          <div class="repair-conversation">
            <div class="repair-prompts">
              <button
                v-for="prompt in prompts"
                :key="prompt.label"
                class="text-button"
                type="button"
                :disabled="!canRepair"
                @click="repairInput = prompt.text"
              >
                {{ prompt.label }}
              </button><button
                v-if="lastPrompt && !repairBusy"
                class="text-button"
                type="button"
                :disabled="!canRepair"
                @click="repairInput = lastPrompt; sendRepair()"
              >
                <AppIcon
                  name="refresh"
                  :size="13"
                />重试
              </button>
            </div>
            <div
              class="chat-history"
              aria-live="polite"
            >
              <article
                v-for="message in chatMessages"
                :key="message.id"
                class="chat-message"
                :data-role="message.role"
              >
                <strong>{{ message.role === 'user' ? '你' : 'Agent' }}</strong><p>{{ message.content }}</p><span
                  v-if="message.role === 'assistant'"
                  class="tiny-badge"
                  :class="message.applied ? 'text-success' : 'text-warning'"
                >{{ message.applied ? '已应用修复' : '未应用' }}</span>
              </article><p
                v-if="!chatMessages.length"
                class="muted-copy"
              >
                暂无对话
              </p>
            </div>
            <div class="repair-composer">
              <textarea
                v-model="repairInput"
                rows="2"
                aria-label="修复需求"
                placeholder="描述需要修复的问题"
                title="Enter 发送，Shift+Enter 换行"
                :disabled="repairBusy"
                @keydown="composerKey"
              /><button
                class="primary-button"
                type="button"
                :disabled="!canRepair || !repairInput.trim()"
                @click="sendRepair"
              >
                <AppIcon
                  :name="repairBusy ? 'loader' : 'play'"
                  :class="{ spinning: repairBusy }"
                  :size="16"
                />{{ repairBusy ? '修复中' : '发送' }}
              </button>
            </div>
          </div>
          <div class="repair-output">
            <SegmentedControl
              v-model="repairView"
              :options="repairOptions"
              label="修复输出"
            /><CodeViewer
              v-if="repairView === 'output'"
              title="模型响应"
              :code="modelOutput"
            /><div
              v-else
              class="rule-diff"
            >
              <article
                v-for="change in changes"
                :key="change.path"
              >
                <strong>{{ change.path }}</strong><pre class="diff-before">− {{ change.before }}</pre><pre class="diff-after">+ {{ change.after }}</pre>
              </article><p
                v-if="!changes.length"
                class="muted-copy"
              >
                无规则变更
              </p>
            </div>
          </div>
        </div>
      </section>
      <ImageViewerModal
        :image="previewImage"
        @close="previewImage = null"
      />
    </template>
  </section>
</template>
