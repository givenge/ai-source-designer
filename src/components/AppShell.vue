<script setup lang="ts">
import { computed, onMounted } from 'vue';
import { useRoute } from 'vue-router';
import AppIcon from '@/components/icons/AppIcon.vue';
import { themePreference, setTheme, type ThemePreference } from '@/lib/theme';
import { siteLabel } from '@/lib/studio';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';

const route = useRoute();
const settings = useSettingsStore();
const tasks = useTasksStore();
const themes: ThemePreference[] = ['dark', 'light', 'system'];
const themeLabels = { dark: '深色', light: '浅色', system: '系统' };
const themeIcons = { dark: 'moon', light: 'sun', system: 'monitor' } as const;
const connectionLabel = computed(() => ({
  unknown: '尚未测试连接', testing: '测试连接中', connected: '连接已验证', failed: '连接测试失败', mock: '本地模拟模式',
})[settings.connectionStatus]);
const breadcrumb = computed(() => {
  const task = tasks.current;
  return route.path.startsWith('/tasks/') && task?.id === route.params.taskId ? siteLabel(task.targetUrl) : '';
});
onMounted(() => {
  if (!settings.hydrated && !settings.loading) void settings.hydrate();
});
function cycleTheme() {
  setTheme(themes[(themes.indexOf(themePreference.value) + 1) % themes.length]);
}
</script>

<template>
  <div class="app-shell">
    <header
      class="app-header"
      data-tauri-drag-region
    >
      <RouterLink
        to="/tasks"
        class="app-brand"
      >
        <span class="app-brand__mark"><AppIcon
          name="source"
          :size="22"
        /></span>
        <span>AI Source Designer</span>
        <span class="version-tag">v0.1.1</span>
      </RouterLink>
      <nav
        class="app-nav"
        aria-label="主导航"
      >
        <RouterLink
          to="/tasks"
          :class="{ 'is-active': route.path.startsWith('/tasks') }"
        >
          <AppIcon
            name="layers"
            :size="16"
          />任务台
        </RouterLink>
        <RouterLink to="/settings">
          <AppIcon
            name="settings"
            :size="16"
          />设置
        </RouterLink>
      </nav>
      <div class="app-header__status">
        <RouterLink
          to="/settings"
          class="provider-status"
          :title="connectionLabel"
        >
          <span
            class="status-dot"
            :data-state="settings.connectionStatus"
          />
          <span class="model-pill">{{ settings.settings.model || '未设置模型' }}</span>
          <span class="sr-only">{{ connectionLabel }}</span>
        </RouterLink>
        <button
          class="theme-button"
          type="button"
          :title="`主题：${themeLabels[themePreference]}`"
          :aria-label="`切换主题，当前${themeLabels[themePreference]}`"
          @click="cycleTheme"
        >
          <AppIcon
            :name="themeIcons[themePreference]"
            :size="18"
          />
          <span>{{ themeLabels[themePreference] }}</span>
        </button>
      </div>
    </header>
    <main class="shell-main">
      <nav
        v-if="breadcrumb"
        class="breadcrumbs"
        aria-label="面包屑"
      >
        <RouterLink to="/tasks">
          任务台
        </RouterLink>
        <AppIcon
          name="chevronRight"
          :size="14"
        />
        <span>{{ breadcrumb }}</span>
      </nav>
      <slot />
    </main>
  </div>
</template>
