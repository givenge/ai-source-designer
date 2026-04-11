<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue';
import { useRouter } from 'vue-router';
import { formatDate } from '@/lib/format';
import { confirmAction } from '@/lib/tauri';
import StatusBadge from '@/components/StatusBadge.vue';
import type { TaskMode } from '@/types/models';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';

const router = useRouter();
const settingsStore = useSettingsStore();
const tasksStore = useTasksStore();

const draft = reactive({
  targetUrl: '',
  testKeyword: '诡秘之主',
  taskMode: 'agent' as TaskMode,
});
const advancedModeOpen = ref(false);

const taskStats = computed(() => {
  const records = tasksStore.records;
  return {
    total: records.length,
    completed: records.filter((task) => task.status === 'completed').length,
    failed: records.filter((task) => task.status === 'failed').length,
    running: records.filter((task) =>
      ['sampling', 'synthesizing', 'validating', 'repairing'].includes(task.status),
    ).length,
  };
});

const activeProviderLabel = computed(() => {
  const baseUrl = settingsStore.settings.baseUrl.trim();
  if (!baseUrl) {
    return '未配置';
  }
  if (baseUrl.startsWith('mock://')) {
    return 'mock://fixture';
  }
  try {
    return new URL(baseUrl).host || baseUrl;
  } catch {
    return baseUrl;
  }
});

const latestTaskLabel = computed(() => {
  const latest = tasksStore.records[0];
  return latest ? formatDate(latest.updatedAt) : '暂无任务';
});

onMounted(async () => {
  if (!settingsStore.hydrated && !settingsStore.loading) {
    await settingsStore.hydrate();
  }
  await tasksStore.refresh();
});

async function launchTask() {
  const result = await tasksStore.create({
    targetUrl: draft.targetUrl,
    testKeyword: draft.testKeyword,
    taskMode: draft.taskMode,
  });
  await router.push(`/tasks/${result.id}`);
  void tasksStore.start(result.id);
}

async function deleteTaskRecord(taskId: string) {
  if (!await confirmAction('确定删除这个任务以及对应证据吗？')) {
    return;
  }

  await tasksStore.delete(taskId);
}

function toggleAdvancedMode() {
  advancedModeOpen.value = !advancedModeOpen.value;
  if (!advancedModeOpen.value && draft.taskMode === 'classic') {
    draft.taskMode = 'agent';
  }
}
</script>

<template>
  <section class="page page-home page-home-polished">
    <header class="page-hero home-hero">
      <div class="hero-topline">
        <div class="hero-copy">
          <p class="eyebrow">
            Reader-Rust Compatible
          </p>
          <h2>AI Source Designer</h2>
          <p class="hero-lead">
            采样真实页面，收集证据，再把模型输出压回可验证、可导出的书源 JSON。
          </p>
        </div>

        <div class="hero-actions">
          <RouterLink
            to="/tasks"
            class="secondary-button is-active"
          >
            任务台
          </RouterLink>
          <RouterLink
            to="/settings"
            class="secondary-button"
          >
            设置
          </RouterLink>
        </div>
      </div>

      <div class="hero-notes">
        <span>桌面本地运行</span>
        <span>Playwright 内置采样</span>
        <span>失败原因可追踪</span>
      </div>

      <div class="hero-stats">
        <article class="hero-stat">
          <span>任务总数</span>
          <strong>{{ taskStats.total }}</strong>
          <p>最近更新：{{ latestTaskLabel }}</p>
        </article>
        <article class="hero-stat">
          <span>已完成</span>
          <strong>{{ taskStats.completed }}</strong>
          <p>当前失败 {{ taskStats.failed }} 个</p>
        </article>
        <article class="hero-stat">
          <span>运行中</span>
          <strong>{{ taskStats.running }}</strong>
          <p>{{ settingsStore.ready ? '配置已就绪' : '请先检查设置' }}</p>
        </article>
        <article class="hero-stat">
          <span>当前 Provider</span>
          <strong>{{ settingsStore.settings.model || '未设置模型' }}</strong>
          <p>{{ activeProviderLabel }}</p>
        </article>
      </div>

      <div class="hero-workflow">
        <article>
          <span>01</span>
          <strong>采样证据</strong>
          <p>Agent 驱动 Playwright 采集页面、DOM 与网络样本。</p>
        </article>
        <article>
          <span>02</span>
          <strong>规则合成</strong>
          <p>基于证据生成兼容 Reader-Rust 的规则候选。</p>
        </article>
        <article>
          <span>03</span>
          <strong>验证与修复</strong>
          <p>本地验证失败后继续修复，直到可导出。</p>
        </article>
      </div>
    </header>

    <div class="dashboard-grid task-home-grid">
      <section class="panel form-panel task-create-panel">
        <header class="panel-header">
          <h3>创建任务</h3>
          <p>任务输入只保留站点 URL 与测试关键词，AI 配置统一走设置页。</p>
        </header>

        <label class="field">
          <span>目标站点 URL</span>
          <input
            v-model="draft.targetUrl"
            type="text"
            placeholder="fixture://novel-world 或 https://example.com"
          >
        </label>

        <label class="field">
          <span>测试关键词</span>
          <input
            v-model="draft.testKeyword"
            type="text"
            placeholder="星门"
          >
        </label>

        <div class="mode-panel">
          <div class="mode-panel__hero">
            <div>
              <span class="mode-panel__label">默认入口</span>
              <strong>Agent 全流程</strong>
              <p>推荐默认使用模型主导的 Playwright 采样、修复和验证闭环。</p>
            </div>
            <button
              class="secondary-button mode-panel__toggle"
              type="button"
              :data-open="advancedModeOpen"
              @click="toggleAdvancedMode"
            >
              {{ advancedModeOpen ? '收起高级模式' : '高级 / 调试模式' }}
            </button>
          </div>

          <div
            v-if="advancedModeOpen"
            class="mode-panel__advanced"
          >
            <label class="mode-choice">
              <input
                v-model="draft.taskMode"
                type="radio"
                name="task-mode"
                value="agent"
              >
              <div>
                <strong>Agent 全流程</strong>
                <p>默认主流程，优先用于真实站点生成与跨站点验证。</p>
              </div>
            </label>

            <label class="mode-choice mode-choice--debug">
              <input
                v-model="draft.taskMode"
                type="radio"
                name="task-mode"
                value="classic"
              >
              <div>
                <strong>Classic 固定采样</strong>
                <p>仅用于回归测试、采样排障和执行链路调试，不作为默认入口。</p>
              </div>
            </label>
          </div>
        </div>

        <button
          class="primary-button"
          :disabled="tasksStore.running || !settingsStore.ready"
          @click="launchTask"
        >
          {{ tasksStore.running ? '执行中...' : '开始分析' }}
        </button>

        <div class="quick-metrics">
          <p class="metric-chip">
            当前模式：{{ draft.taskMode === 'agent' ? 'Agent 全流程' : 'Classic 调试模式' }}
          </p>
          <p class="metric-chip">
            当前模型：{{ settingsStore.settings.model }}
          </p>
          <p class="metric-chip metric-chip--soft">
            {{ settingsStore.settings.baseUrl }}
          </p>
        </div>
        <p
          v-if="tasksStore.error"
          class="error-copy"
        >
          {{ tasksStore.error }}
        </p>
      </section>

      <section class="panel">
        <header class="panel-header">
          <h3>最近任务</h3>
          <p>最新任务优先，点击后进入详情页查看证据、日志和导出结果。</p>
        </header>

        <div
          v-if="tasksStore.records.length"
          class="task-list"
        >
          <article
            v-for="task in tasksStore.records"
            :key="task.id"
            class="task-row"
          >
            <RouterLink
              :to="`/tasks/${task.id}`"
              class="task-row__main"
            >
              <div class="task-row__copy">
                <strong>{{ task.targetUrl }}</strong>
                <p>{{ formatDate(task.updatedAt) }} / {{ task.taskMode === 'agent' ? 'Agent' : 'Classic' }}</p>
                <div class="task-row__meta">
                  <span v-if="task.testKeyword">关键词：{{ task.testKeyword }}</span>
                  <span>ID：{{ task.id.slice(0, 8) }}</span>
                </div>
                <p
                  v-if="task.errorSummary"
                  class="task-row__error"
                >
                  {{ task.errorSummary }}
                </p>
              </div>
            </RouterLink>

            <div class="task-row__actions">
              <StatusBadge :status="task.status" />
              <button
                class="danger-button"
                :disabled="tasksStore.deleting"
                @click.stop="deleteTaskRecord(task.id)"
              >
                {{ tasksStore.deleting ? '删除中...' : '删除' }}
              </button>
            </div>
          </article>
        </div>

        <p
          v-else
          class="muted-copy"
        >
          还没有任务记录。
        </p>
      </section>
    </div>
  </section>
</template>

<style scoped>
.page-home-polished .home-hero {
  border-radius: 30px;
}

.hero-stats {
  display: grid;
  gap: 0.85rem;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin-top: 1.35rem;
}

.hero-workflow {
  margin-top: 1rem;
  display: grid;
  gap: 0.75rem;
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.hero-workflow article {
  border: 1px solid rgba(19, 32, 51, 0.1);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.64);
  padding: 0.75rem 0.78rem;
}

.hero-workflow span {
  display: inline-flex;
  min-width: 1.8rem;
  height: 1.8rem;
  align-items: center;
  justify-content: center;
  border-radius: 8px;
  margin-bottom: 0.34rem;
  background: linear-gradient(135deg, rgba(176, 99, 42, 0.2), rgba(33, 71, 111, 0.2));
  color: #223754;
  font-size: 0.82rem;
  font-weight: 700;
}

.hero-workflow strong {
  display: block;
  color: #1f2a37;
  margin-bottom: 0.2rem;
}

.hero-workflow p {
  margin: 0;
  color: #607083;
  font-size: 0.84rem;
  line-height: 1.45;
}

.hero-stat {
  padding: 0.95rem 1rem;
  border: 1px solid rgba(31, 38, 48, 0.08);
  border-radius: 18px;
  background: rgba(255, 255, 255, 0.52);
  min-width: 0;
}

.hero-stat span {
  display: block;
  margin-bottom: 0.35rem;
  font-size: 0.78rem;
  letter-spacing: 0.08em;
  text-transform: uppercase;
  color: #8b6b4d;
}

.hero-stat strong {
  display: block;
  font-size: 1.3rem;
  line-height: 1.1;
  color: #1f2a37;
  overflow-wrap: anywhere;
}

.hero-stat p {
  margin: 0.38rem 0 0;
  color: #657182;
  line-height: 1.4;
  overflow-wrap: anywhere;
}

.task-home-grid {
  align-items: start;
}

.task-create-panel {
  position: sticky;
  top: 1rem;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.72), rgba(255, 255, 255, 0.5)),
    rgba(248, 242, 230, 0.72);
}

.task-create-panel .field input,
.task-create-panel .field select {
  width: 100%;
  padding: 0.95rem 1rem;
  border: 1px solid rgba(31, 38, 48, 0.14);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.86);
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.task-create-panel .field input:focus,
.task-create-panel .field select:focus {
  outline: none;
  border-color: rgba(188, 93, 46, 0.45);
  box-shadow: 0 0 0 3px rgba(188, 93, 46, 0.12);
}

.mode-panel {
  margin-bottom: 1rem;
  border: 1px solid rgba(31, 38, 48, 0.1);
  border-radius: 18px;
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.88), rgba(255, 255, 255, 0.62)),
    rgba(244, 236, 221, 0.55);
  overflow: hidden;
}

.mode-panel__hero {
  display: grid;
  gap: 0.75rem;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  padding: 0.95rem 1rem;
}

.mode-panel__label {
  display: block;
  margin-bottom: 0.28rem;
  color: #8b5d2a;
  font-size: 0.76rem;
  font-weight: 700;
  letter-spacing: 0.12em;
  text-transform: uppercase;
}

.mode-panel__hero strong {
  display: block;
  font-size: 1.02rem;
  color: #1f2a37;
}

.mode-panel__hero p {
  margin: 0.35rem 0 0;
  color: #5b6776;
  line-height: 1.5;
}

.mode-panel__toggle[data-open='true'] {
  background: rgba(168, 90, 31, 0.12);
  border-color: rgba(168, 90, 31, 0.22);
}

.mode-panel__advanced {
  display: grid;
  gap: 0.7rem;
  padding: 0 1rem 1rem;
  border-top: 1px solid rgba(31, 38, 48, 0.08);
}

.mode-choice {
  display: grid;
  grid-template-columns: auto minmax(0, 1fr);
  gap: 0.8rem;
  align-items: start;
  padding: 0.9rem;
  border: 1px solid rgba(31, 38, 48, 0.1);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.72);
  cursor: pointer;
}

.mode-choice input {
  margin-top: 0.2rem;
}

.mode-choice strong {
  display: block;
  color: #1f2a37;
}

.mode-choice p {
  margin: 0.26rem 0 0;
  color: #5c6877;
  line-height: 1.45;
}

.mode-choice--debug {
  background: rgba(247, 244, 238, 0.82);
}

.quick-metrics {
  margin-top: 0.75rem;
  display: grid;
  gap: 0.45rem;
}

.metric-chip {
  margin: 0;
  padding: 0.5rem 0.65rem;
  border-radius: 10px;
  border: 1px solid rgba(31, 38, 48, 0.1);
  background: rgba(255, 255, 255, 0.66);
  color: #4d596b;
  font-size: 0.86rem;
  line-height: 1.45;
  overflow-wrap: anywhere;
}

.metric-chip--soft {
  color: #6d7785;
  font-size: 0.82rem;
}

.task-row {
  transition: transform 0.16s ease, box-shadow 0.16s ease, border-color 0.16s ease;
}

.task-row:hover {
  transform: translateY(-1px);
  border-color: rgba(31, 38, 48, 0.2);
  box-shadow: 0 10px 26px rgba(52, 39, 24, 0.08);
}

.task-row__meta {
  display: flex;
  flex-wrap: wrap;
  gap: 0.45rem 0.7rem;
  margin-top: 0.45rem;
  color: #6f7a89;
  font-size: 0.82rem;
}

.task-row__meta span {
  display: inline-flex;
  align-items: center;
  padding: 0.24rem 0.52rem;
  border-radius: 999px;
  background: rgba(31, 38, 48, 0.06);
}

.task-row__error {
  margin-top: 0.55rem;
  color: #a24d3d;
  font-size: 0.86rem;
  line-height: 1.45;
}

@media (max-width: 1080px) {
  .task-create-panel {
    position: static;
  }

  .mode-panel__hero {
    grid-template-columns: 1fr;
  }

  .hero-stats {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }

  .hero-workflow {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 720px) {
  .hero-stats {
    grid-template-columns: 1fr;
  }
}
</style>
