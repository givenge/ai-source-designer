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
  <section class="page settings-page-polished">
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

    <section class="panel form-panel settings-panel">
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

      <div class="settings-hints">
        <article>
          <h4>配置作用域</h4>
          <p>配置写入本机 `~/.aisourcedesign/settings.json`，不会上传到远端。</p>
        </article>
        <article>
          <h4>推荐做法</h4>
          <p>先用 `mock://fixture` 联调流程，再切换真实模型接口。</p>
        </article>
        <article>
          <h4>故障排查</h4>
          <p>模型报错时先核对 Base URL、模型名和 API Key，再看任务日志。</p>
        </article>
      </div>

      <div class="settings-runtime">
        <article>
          <h4>连接状态</h4>
          <p>{{ settingsStore.settings.baseUrl ? '已配置 Provider 接口' : '未配置 Base URL' }}</p>
        </article>
        <article>
          <h4>模型状态</h4>
          <p>{{ settingsStore.settings.model || '未配置模型' }}</p>
        </article>
        <article>
          <h4>密钥状态</h4>
          <p>{{ settingsStore.settings.apiKey ? '已加载 API Key（已脱敏）' : '未配置 API Key' }}</p>
        </article>
      </div>

      <p
        v-if="settingsStore.error"
        class="error-copy"
      >
        {{ settingsStore.error }}
      </p>
    </section>
  </section>
</template>

<style scoped>
.settings-panel {
  background:
    linear-gradient(180deg, rgba(255, 255, 255, 0.8), rgba(255, 255, 255, 0.58)),
    rgba(248, 242, 229, 0.72);
}

.settings-panel .field input {
  width: 100%;
  padding: 0.95rem 1rem;
  border: 1px solid rgba(31, 38, 48, 0.14);
  border-radius: 14px;
  background: rgba(255, 255, 255, 0.88);
  transition: border-color 0.18s ease, box-shadow 0.18s ease;
}

.settings-panel .field input:focus {
  outline: none;
  border-color: rgba(188, 93, 46, 0.45);
  box-shadow: 0 0 0 3px rgba(188, 93, 46, 0.12);
}

.settings-meta {
  padding: 0.62rem 0.72rem;
  border: 1px solid rgba(31, 38, 48, 0.12);
  border-radius: 12px;
  background: rgba(255, 255, 255, 0.75);
  color: #4c5a6e;
}

.settings-meta p {
  margin: 0;
}

.settings-hints {
  margin-top: 0.9rem;
  display: grid;
  gap: 0.65rem;
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.settings-hints article {
  border: 1px solid rgba(31, 38, 48, 0.12);
  border-radius: 12px;
  padding: 0.72rem 0.78rem;
  background: rgba(255, 255, 255, 0.8);
}

.settings-hints h4 {
  margin: 0 0 0.22rem;
  font-size: 0.92rem;
  color: #2f3a47;
}

.settings-hints p {
  margin: 0;
  color: #5e6877;
  line-height: 1.48;
  font-size: 0.84rem;
}

.settings-runtime {
  margin-top: 0.85rem;
  display: grid;
  gap: 0.62rem;
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.settings-runtime article {
  border: 1px solid rgba(31, 38, 48, 0.12);
  border-radius: 12px;
  background: linear-gradient(150deg, rgba(255, 255, 255, 0.88), rgba(249, 243, 231, 0.72));
  padding: 0.66rem 0.75rem;
}

.settings-runtime h4 {
  margin: 0 0 0.26rem;
  font-size: 0.88rem;
  color: #314157;
}

.settings-runtime p {
  margin: 0;
  color: #5e6d81;
  line-height: 1.45;
  font-size: 0.84rem;
}

@media (max-width: 1080px) {
  .settings-hints {
    grid-template-columns: 1fr;
  }

  .settings-runtime {
    grid-template-columns: 1fr;
  }
}
</style>
