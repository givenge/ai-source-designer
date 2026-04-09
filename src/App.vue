<script setup lang="ts">
import { computed } from 'vue';
import { useRoute } from 'vue-router';

const route = useRoute();
const showQuickNav = computed(() => !String(route.path).startsWith('/tasks/'));
</script>

<template>
  <div class="app-frame">
    <nav
      v-if="showQuickNav"
      class="quick-nav"
    >
      <RouterLink
        to="/tasks"
        class="quick-nav-link"
        active-class="is-active"
      >
        任务台
      </RouterLink>
      <RouterLink
        to="/settings"
        class="quick-nav-link"
        active-class="is-active"
      >
        设置
      </RouterLink>
    </nav>
    <main class="shell-main">
      <RouterView />
    </main>
  </div>
</template>
