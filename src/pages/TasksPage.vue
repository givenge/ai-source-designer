<script setup lang="ts">
import { onMounted, reactive } from 'vue';
import { useRouter } from 'vue-router';
import { formatDate } from '@/lib/format';
import StatusBadge from '@/components/StatusBadge.vue';
import type { TaskStatus } from '@/types/models';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';

const router = useRouter();
const settingsStore = useSettingsStore();
const tasksStore = useTasksStore();

const draft = reactive({
  targetUrl: 'fixture://novel-world',
  testKeyword: '星门',
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
  });
  await router.push(`/tasks/${result.id}`);
  void tasksStore.start(result.id);
}

function isActiveStatus(status: TaskStatus): boolean {
  return ['sampling', 'synthesizing', 'validating', 'repairing'].includes(status);
}

async function deleteTaskRecord(taskId: string) {
  if (!window.confirm('确定删除这个任务以及对应证据吗？')) {
    return;
  }

  await tasksStore.delete(taskId);
}
</script>

<template>
  <section class="page page-home">
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
    </header>

    <div class="dashboard-grid">
      <section class="panel form-panel">
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

        <button
          class="primary-button"
          :disabled="tasksStore.running || !settingsStore.ready"
          @click="launchTask"
        >
          {{ tasksStore.running ? '执行中...' : '开始分析' }}
        </button>

        <p class="muted-copy">
          当前模型：{{ settingsStore.settings.model }} / {{ settingsStore.settings.baseUrl }}
        </p>
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
                <p>{{ formatDate(task.updatedAt) }}</p>
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
