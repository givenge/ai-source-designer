<script setup lang="ts">
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue';
import { useRouter } from 'vue-router';
import AppIcon from '@/components/icons/AppIcon.vue';
import MetricCard from '@/components/MetricCard.vue';
import SegmentedControl from '@/components/SegmentedControl.vue';
import StatusBadge from '@/components/StatusBadge.vue';
import { formatDate } from '@/lib/format';
import { filterTasks, isTaskActive, siteLabel, taskDuration, taskProgress } from '@/lib/studio';
import { confirmAction } from '@/lib/tauri';
import type { TaskMode, TaskRecord } from '@/types/models';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';

const router = useRouter();
const settings = useSettingsStore();
const tasks = useTasksStore();
const draft = reactive({ targetUrl: '', testKeyword: '星门', taskMode: 'agent' as TaskMode });
const targetInput = ref<HTMLInputElement | null>(null);
const query = ref('');
const filter = ref('all');
const advanced = ref(false);
const launching = ref(false);
const launchError = ref('');
let refreshTimer: number | undefined;
const filterOptions = [
  { value: 'all', label: '全部' }, { value: 'running', label: '运行中' },
  { value: 'completed', label: '已完成' }, { value: 'failed', label: '失败' },
];
const modeOptions = [{ value: 'agent', label: 'Agent 自动分析' }, { value: 'classic', label: 'Classic 调试' }];
const examples = [
  { label: 'novel-world', url: 'fixture://novel-world', keyword: '星门' },
  { label: 'Gutenberg', url: 'https://www.gutenberg.org/', keyword: 'sherlock' },
];
const stats = computed(() => ({
  total: tasks.records.length,
  completed: tasks.records.filter((task) => task.status === 'completed').length,
  running: tasks.records.filter(isTaskActive).length,
  failed: tasks.records.filter((task) => task.status === 'failed').length,
}));
const visibleTasks = computed(() => filterTasks(tasks.records, filter.value, query.value));
const canLaunch = computed(() => Boolean(draft.targetUrl.trim() && settings.hydrated && !settings.loading && settings.ready && !launching.value && !tasks.running && !tasks.runtimeRepairing));

onMounted(async () => {
  if (!settings.hydrated && !settings.loading) await settings.hydrate();
  await tasks.refresh();
  refreshTimer = window.setInterval(() => {
    if (tasks.records.some(isTaskActive) && !tasks.loading) void tasks.refresh();
  }, 2500);
});
onUnmounted(() => window.clearInterval(refreshTimer));

async function launchTask() {
  if (!canLaunch.value) return;
  launching.value = true;
  launchError.value = '';
  try {
    const task = await tasks.create({ ...draft, targetUrl: draft.targetUrl.trim() });
    await router.push(`/tasks/${task.id}`);
    void tasks.start(task.id);
  } catch (error) {
    launchError.value = error instanceof Error ? error.message : String(error);
  } finally { launching.value = false; }
}

function shortcut(event: KeyboardEvent) {
  if (event.key === 'Enter' && (event.metaKey || event.ctrlKey)) {
    event.preventDefault();
    void launchTask();
  }
}

async function restart(task: TaskRecord) {
  await router.push(`/tasks/${task.id}`);
  if (task.status === 'completed') void tasks.revalidate(task.id);
  else void tasks.start(task.id);
}

async function remove(task: TaskRecord) {
  if (await confirmAction('删除此任务及全部采样证据？此操作无法撤销。')) await tasks.delete(task.id);
}

function resetFilters() { query.value = ''; filter.value = 'all'; }
</script>

<template>
  <section class="page tasks-page">
    <div class="metrics-bar">
      <MetricCard
        label="任务总数"
        :value="stats.total"
        icon="layers"
      />
      <MetricCard
        label="已完成"
        :value="stats.completed"
        icon="check"
      />
      <MetricCard
        label="运行中"
        :value="stats.running"
        icon="loader"
      />
      <MetricCard
        label="失败"
        :value="stats.failed"
        icon="alert"
      />
    </div>
    <div class="task-dashboard">
      <form
        class="panel task-launcher"
        @submit.prevent="launchTask"
        @keydown="shortcut"
      >
        <header class="panel-header">
          <h2><AppIcon name="sparkles" />创建任务</h2>
        </header>
        <label class="field"><span>目标站点 URL</span><input
          ref="targetInput"
          v-model="draft.targetUrl"
          type="text"
          placeholder="https://example.com"
          required
        ></label>
        <div
          class="example-links"
          aria-label="示例站点"
        >
          <button
            v-for="example in examples"
            :key="example.url"
            type="button"
            :title="example.url"
            @click="draft.targetUrl = example.url; draft.testKeyword = example.keyword"
          >
            <AppIcon
              :name="example.url.startsWith('fixture:') ? 'cpu' : 'source'"
              :size="14"
            />{{ example.label }}
          </button>
        </div>
        <label class="field"><span>测试关键词</span><input
          v-model="draft.testKeyword"
          type="text"
          placeholder="书名或作者"
        ></label>
        <div class="field mode-field">
          <div class="field-heading">
            <span>任务模式</span><button
              type="button"
              class="text-button"
              :aria-expanded="advanced"
              @click="advanced = !advanced; !advanced && (draft.taskMode = 'agent')"
            >
              {{ advanced ? '收起' : '高级' }}
            </button>
          </div>
          <SegmentedControl
            v-if="advanced"
            v-model="draft.taskMode"
            :options="modeOptions"
            label="任务模式"
          />
          <div
            v-else
            class="mode-current"
          >
            <AppIcon
              name="sparkles"
              :size="16"
            /><strong>Agent 自动分析</strong><span class="tiny-badge">默认</span>
          </div>
        </div>
        <button
          class="primary-button launch-button"
          :disabled="!canLaunch"
          :aria-busy="launching || tasks.running"
          type="submit"
          title="Cmd / Ctrl + Enter"
        >
          <AppIcon
            :name="launching || tasks.running ? 'loader' : 'play'"
            :class="{ spinning: launching || tasks.running }"
            :size="17"
          />{{ launching ? '创建中' : '开始分析' }}<kbd>⌘ ↵</kbd>
        </button>
        <RouterLink
          v-if="!settings.ready"
          class="error-copy"
          to="/settings"
        >
          请先配置 AI 服务
        </RouterLink>
        <p
          v-if="launchError"
          class="error-copy"
          role="alert"
        >
          {{ launchError }}
        </p>
      </form>
      <section class="panel task-hub">
        <header class="panel-header panel-header--row">
          <h2>任务中心</h2><button
            class="icon-button"
            type="button"
            :disabled="tasks.loading"
            title="刷新任务"
            aria-label="刷新任务"
            @click="tasks.refresh()"
          >
            <AppIcon
              name="refresh"
              :class="{ spinning: tasks.loading }"
              :size="17"
            />
          </button>
        </header>
        <div class="task-hub__toolbar">
          <SegmentedControl
            v-model="filter"
            :options="filterOptions"
            label="任务筛选"
          />
          <label class="search-field"><AppIcon
            name="search"
            :size="16"
          /><input
            v-model="query"
            type="search"
            aria-label="搜索任务"
            placeholder="搜索站点或关键词"
          ></label>
        </div>
        <p
          v-if="tasks.error"
          class="error-copy"
          role="alert"
        >
          {{ tasks.error }}
        </p>
        <div
          v-if="visibleTasks.length"
          class="task-list"
        >
          <article
            v-for="task in visibleTasks"
            :key="task.id"
            class="task-row"
            :data-status="task.status"
          >
            <RouterLink
              :to="`/tasks/${task.id}`"
              class="task-row__main"
            >
              <span class="site-avatar">{{ siteLabel(task.targetUrl).slice(0, 2).toUpperCase() }}</span>
              <div class="task-row__copy">
                <strong :title="task.targetUrl">{{ siteLabel(task.targetUrl) }}</strong>
                <div class="task-row__meta">
                  <span>{{ task.taskMode === 'agent' ? 'Agent' : 'Classic' }}</span><span v-if="task.testKeyword">{{ task.testKeyword }}</span><span>{{ formatDate(task.updatedAt) }}</span><span title="最近一次执行耗时"><AppIcon
                    name="clock"
                    :size="13"
                  />{{ taskDuration(task) }}</span><span
                    v-if="task.validationReport"
                    :class="{ 'text-warning': taskProgress(task).passed < taskProgress(task).total }"
                  >{{ taskProgress(task).passed }}/{{ taskProgress(task).total }} 阶段</span>
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
              <RouterLink
                :to="`/tasks/${task.id}`"
                class="icon-button"
                title="查看任务"
                :aria-label="`查看 ${siteLabel(task.targetUrl)}`"
              >
                <AppIcon
                  name="chevronRight"
                  :size="17"
                />
              </RouterLink>
              <button
                v-if="['completed', 'failed'].includes(task.status)"
                class="icon-button"
                type="button"
                :disabled="tasks.running"
                :title="task.status === 'completed' ? '重新验证' : '继续执行'"
                :aria-label="task.status === 'completed' ? '重新验证' : '继续执行'"
                @click="restart(task)"
              >
                <AppIcon
                  name="refresh"
                  :size="16"
                />
              </button>
              <button
                class="icon-button danger-button"
                type="button"
                :disabled="tasks.deleting"
                title="删除任务"
                :aria-label="`删除 ${siteLabel(task.targetUrl)}`"
                @click="remove(task)"
              >
                <AppIcon
                  name="trash"
                  :size="16"
                />
              </button>
            </div>
          </article>
        </div>
        <div
          v-else
          class="empty-state"
        >
          <svg
            class="empty-state__art"
            width="88"
            height="68"
            viewBox="0 0 88 68"
            fill="none"
            aria-hidden="true"
          ><rect
            x="14"
            y="10"
            width="60"
            height="46"
            rx="8"
            stroke="currentColor"
            stroke-width="1.5"
          /><path
            d="M14 24h60 M25 17h.01 M32 17h.01 M39 17h.01 M31 38h26 M31 45h16"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          /><path
            d="M6 34h8 M74 34h8"
            stroke="currentColor"
            stroke-dasharray="2 3"
          /></svg>
          <strong>{{ tasks.loading ? '读取任务中' : tasks.records.length ? '无匹配任务' : '暂无任务' }}</strong>
          <button
            v-if="!tasks.loading"
            class="secondary-button"
            type="button"
            @click="tasks.records.length ? resetFilters() : targetInput?.focus()"
          >
            {{ tasks.records.length ? '重置筛选' : '创建任务' }}
          </button>
        </div>
      </section>
    </div>
  </section>
</template>
