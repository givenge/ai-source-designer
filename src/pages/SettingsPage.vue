<script setup lang="ts">
import { onMounted } from 'vue';
import { maskSecret } from '@/lib/format';
import { useSettingsStore } from '@/stores/settings';

const settingsStore = useSettingsStore();

onMounted(async () => {
  if (!settingsStore.hydrated && !settingsStore.loading) {
    await settingsStore.hydrate();
  }
});
</script>

<template>
  <section class="page">
    <header class="page-hero">
      <div class="hero-topline">
        <div class="hero-copy">
          <p class="eyebrow">
            Settings Contract
          </p>
          <h2>AI Provider 与安全存储</h2>
          <p class="hero-lead">
            所有配置统一写入 `~/.aisourcedesign/settings.json`，任务页只负责执行与导出。
          </p>
        </div>

        <div class="hero-actions">
          <RouterLink
            to="/tasks"
            class="secondary-button"
          >
            任务台
          </RouterLink>
          <RouterLink
            to="/settings"
            class="secondary-button is-active"
          >
            设置
          </RouterLink>
        </div>
      </div>
    </header>

    <section class="panel form-panel">
      <header class="panel-header">
        <h3>模型接入</h3>
        <p>当前默认走 OpenAI-compatible 接口，也可以继续使用 `mock://fixture` 进行本地演示。</p>
      </header>

      <label class="field">
        <span>Base URL</span>
        <input
          v-model="settingsStore.settings.baseUrl"
          type="url"
          placeholder="https://api.example.com/v1"
        >
      </label>

      <label class="field">
        <span>模型</span>
        <input
          v-model="settingsStore.settings.model"
          type="text"
          placeholder="gpt-4.1-mini"
        >
      </label>

      <label class="field">
        <span>API Key</span>
        <input
          v-model="settingsStore.settings.apiKey"
          type="password"
          placeholder="sk-..."
          autocomplete="off"
        >
      </label>

      <div class="settings-meta">
        <p>
          当前状态：
          <strong>{{
            settingsStore.settings.apiKey
              ? `已加载 ${maskSecret(settingsStore.settings.apiKey)}`
              : '未设置 API Key'
          }}</strong>
        </p>
      </div>

      <button
        class="primary-button"
        :disabled="settingsStore.saving"
        @click="settingsStore.persist()"
      >
        {{ settingsStore.saving ? '保存中...' : '保存设置' }}
      </button>

      <p
        v-if="settingsStore.error"
        class="error-copy"
      >
        {{ settingsStore.error }}
      </p>
    </section>
  </section>
</template>