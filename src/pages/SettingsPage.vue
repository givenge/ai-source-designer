<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import AppIcon from '@/components/icons/AppIcon.vue';
import CodeViewer from '@/components/CodeViewer.vue';
import { maskSecret } from '@/lib/format';
import { getRuntimeDiagnostics, getRuntimeRepairLog, openDataDirectory, repairPlaywright } from '@/lib/tauri';
import { isTaskActive } from '@/lib/studio';
import type { RuntimeDiagnostics } from '@/types/models';
import { useSettingsStore } from '@/stores/settings';
import { useTasksStore } from '@/stores/tasks';

const settings = useSettingsStore();
const tasks = useTasksStore();
const showKey = ref(false);
const diagnostics = ref<RuntimeDiagnostics | null>(null);
const checkingRuntime = ref(false);
const runtimeError = ref('');
const repairLog = ref('');
const runtimeFeedback = ref('');
let repairLogTimer: number | undefined;
const activeTask = computed(() => tasks.running || tasks.chatting || tasks.records.some(isTaskActive));
const presets = [
  { name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', model: 'gpt-4.1-mini', letter: 'O' },
  { name: 'DeepSeek', baseUrl: 'https://api.deepseek.com/v1', model: 'deepseek-chat', letter: 'D' },
  { name: 'SiliconFlow', baseUrl: 'https://api.siliconflow.cn/v1', model: 'Qwen/Qwen3-8B', letter: 'S' },
  { name: 'Ollama', baseUrl: 'http://127.0.0.1:11434/v1', model: 'qwen3:8b', letter: 'L' },
  { name: 'Mock / Fixture', baseUrl: 'mock://fixture', model: 'mock-fixture', letter: 'M' },
];
const pendingPreset = ref<typeof presets[number] | null>(null);
const activePreset = computed(() => presets.find((preset) => settings.settings.baseUrl.replace(/\/$/, '') === preset.baseUrl)?.name);
const connectionFeedback = computed(() => {
  if (settings.connectionStatus === 'mock') return '本地模拟模式';
  if (settings.connectionStatus === 'connected') return `${settings.connection?.latencyMs} ms · 连接正常`;
  return '';
});
onMounted(async () => {
  if (!settings.hydrated && !settings.loading) await settings.hydrate();
  void refreshDiagnostics();
});
watch(() => tasks.runtimeRepairing, (repairing) => {
  window.clearInterval(repairLogTimer);
  if (repairing) {
    void refreshRepairLog();
    repairLogTimer = window.setInterval(() => { void refreshRepairLog(); }, 2000);
  } else {
    void refreshRepairLog();
    if (!diagnostics.value) void refreshDiagnostics(true);
  }
}, { immediate: true });
onUnmounted(() => window.clearInterval(repairLogTimer));
function choosePreset(preset: typeof presets[number]) {
  pendingPreset.value = preset;
}
function applyPreset() {
  const preset = pendingPreset.value;
  if (!preset || settings.loading || settings.saving || settings.testing) return;
  const sameProvider = settings.settings.baseUrl.replace(/\/$/, '') === preset.baseUrl;
  settings.settings = { baseUrl: preset.baseUrl, model: preset.model, apiKey: sameProvider ? settings.settings.apiKey : '' };
  pendingPreset.value = null;
  showKey.value = false;
}
function restoreSaved() {
  if (!settings.savedSettings) return;
  settings.settings = { ...settings.savedSettings };
  pendingPreset.value = null;
  showKey.value = false;
}
async function refreshDiagnostics(preserveError = false) {
  if (checkingRuntime.value || tasks.runtimeRepairing) return;
  checkingRuntime.value = true;
  if (!preserveError) runtimeError.value = '';
  try { diagnostics.value = await getRuntimeDiagnostics(); }
  catch (error) { if (!runtimeError.value) runtimeError.value = error instanceof Error ? error.message : String(error); }
  finally { checkingRuntime.value = false; }
}
async function refreshRepairLog() {
  try { repairLog.value = await getRuntimeRepairLog(); }
  catch (error) { runtimeError.value = error instanceof Error ? error.message : String(error); }
}
async function repairRuntime() {
  if (checkingRuntime.value || tasks.runtimeRepairing || activeTask.value) return;
  tasks.runtimeRepairing = true;
  runtimeError.value = '';
  runtimeFeedback.value = '';
  repairLog.value = '';
  try {
    diagnostics.value = await repairPlaywright();
    if (diagnostics.value.workerReady) runtimeFeedback.value = '修复完成 · Chromium 启动通过';
    else runtimeError.value = diagnostics.value.error || '运行环境仍不可用';
  } catch (error) { runtimeError.value = error instanceof Error ? error.message : String(error); }
  finally {
    await refreshRepairLog();
    tasks.runtimeRepairing = false;
  }
}
async function openDirectory() {
  try { await openDataDirectory(); }
  catch (error) { runtimeError.value = error instanceof Error ? error.message : String(error); }
}
</script>

<template>
  <section class="page settings-page">
    <header class="page-title">
      <h1>设置</h1>
    </header>
    <div class="settings-layout">
      <form
        class="panel provider-panel"
        @submit.prevent="settings.persist()"
      >
        <header class="panel-header">
          <h2><AppIcon name="sparkles" />AI 服务</h2>
        </header>
        <div
          class="provider-presets"
          role="group"
          aria-label="服务商预设"
        >
          <button
            v-for="preset in presets"
            :key="preset.name"
            class="provider-preset"
            :data-active="(pendingPreset?.name ?? activePreset) === preset.name"
            type="button"
            :disabled="settings.loading || settings.saving || settings.testing"
            :aria-pressed="(pendingPreset?.name ?? activePreset) === preset.name"
            @click="choosePreset(preset)"
          >
            <span>{{ preset.letter }}</span><strong>{{ preset.name }}</strong>
          </button>
        </div>
        <div
          v-if="pendingPreset"
          class="provider-preview"
        >
          <dl><div><dt>端点</dt><dd>{{ pendingPreset.baseUrl }}</dd></div><div><dt>模型</dt><dd>{{ pendingPreset.model }}</dd></div></dl>
          <div class="toolbar-actions">
            <button
              class="secondary-button"
              type="button"
              :disabled="settings.loading || settings.saving || settings.testing"
              @click="applyPreset"
            >
              应用预设
            </button>
            <button
              class="text-button"
              type="button"
              @click="pendingPreset = null"
            >
              取消
            </button>
          </div>
          <p
            v-if="settings.settings.apiKey && settings.settings.baseUrl.replace(/\/$/, '') !== pendingPreset.baseUrl"
            class="field-note"
          >
            切换端点后需重新填写 API Key。
          </p>
        </div>
        <label class="field"><span>Base URL（OpenAI 兼容）</span><input
          v-model="settings.settings.baseUrl"
          type="url"
          placeholder="https://api.example.com/v1"
          :disabled="settings.loading || settings.saving || settings.testing"
          required
        ></label>
        <label class="field"><span>模型</span><input
          v-model="settings.settings.model"
          type="text"
          placeholder="模型名称"
          :disabled="settings.loading || settings.saving || settings.testing"
          required
        ></label>
        <label class="field">
          <span>API Key</span>
          <div class="password-field"><input
            v-model="settings.settings.apiKey"
            :type="showKey ? 'text' : 'password'"
            placeholder="sk-..."
            autocomplete="off"
            :disabled="settings.loading || settings.saving || settings.testing"
          ><button
            class="icon-button"
            type="button"
            :aria-pressed="showKey"
            :aria-label="showKey ? '隐藏密钥' : '显示密钥'"
            :title="showKey ? '隐藏密钥' : '显示密钥'"
            @click="showKey = !showKey"
          ><AppIcon
            :name="showKey ? 'eyeOff' : 'eye'"
            :size="18"
          /></button></div>
          <span
            v-if="settings.settings.apiKey && !showKey"
            class="field-meta key-summary"
          >{{ maskSecret(settings.settings.apiKey) }}</span>
        </label>
        <p class="field-note">
          API Key 以明文保存在本机。
        </p>
        <div class="settings-actions">
          <button
            v-if="!settings.saved && settings.savedSettings"
            class="text-button"
            type="button"
            :disabled="settings.loading || settings.saving || settings.testing"
            @click="restoreSaved"
          >
            恢复已保存
          </button>
          <button
            class="primary-button"
            type="submit"
            :disabled="settings.saving || settings.testing || !settings.ready"
          >
            <AppIcon
              :name="settings.saving ? 'loader' : 'check'"
              :class="{ spinning: settings.saving }"
              :size="16"
            />{{ settings.saving ? '保存中' : '保存设置' }}
          </button>
          <button
            class="secondary-button"
            type="button"
            :disabled="settings.saving || settings.testing || !settings.ready"
            @click="settings.testConnection()"
          >
            <AppIcon
              :name="settings.testing ? 'loader' : 'refresh'"
              :class="{ spinning: settings.testing }"
              :size="16"
            />{{ settings.testing ? '测试中' : '测试连接' }}
          </button>
          <span
            class="save-state"
            aria-live="polite"
          ><AppIcon
            v-if="settings.saved"
            name="check"
            :size="14"
          />{{ settings.saved ? '已保存' : '未保存' }}</span>
        </div>
        <p
          v-if="connectionFeedback"
          class="connection-feedback"
          role="status"
        >
          <span
            class="status-dot"
            :data-state="settings.connectionStatus"
          />{{ connectionFeedback }}
        </p>
        <p
          v-if="settings.connectionStatus === 'failed'"
          class="error-copy"
          role="alert"
        >
          {{ settings.connectionError }}
        </p>
        <p
          v-if="settings.error"
          class="error-copy"
          role="alert"
        >
          {{ settings.error }}
        </p>
      </form>
      <section class="panel runtime-panel">
        <header class="panel-header panel-header--row">
          <h2><AppIcon name="cpu" />运行环境</h2><button
            class="icon-button"
            type="button"
            :disabled="checkingRuntime || tasks.runtimeRepairing"
            title="检测运行环境"
            aria-label="检测运行环境"
            @click="refreshDiagnostics()"
          >
            <AppIcon
              name="refresh"
              :class="{ spinning: checkingRuntime }"
              :size="16"
            />
          </button>
        </header>
        <div class="runtime-repair-actions">
          <button
            class="secondary-button"
            type="button"
            :disabled="checkingRuntime || tasks.runtimeRepairing || activeTask"
            :aria-busy="tasks.runtimeRepairing"
            @click="repairRuntime"
          >
            <AppIcon
              :name="tasks.runtimeRepairing ? 'loader' : 'refresh'"
              :class="{ spinning: tasks.runtimeRepairing }"
              :size="16"
            />{{ tasks.runtimeRepairing ? '修复中' : '一键修复' }}
          </button>
          <span
            v-if="activeTask"
            class="field-note"
          >任务执行中</span>
          <p class="field-note">
            修复需系统 Node.js/npm，并联网安装 Playwright 和 Chromium。
          </p>
        </div>
        <p
          v-if="runtimeFeedback"
          class="connection-feedback"
          role="status"
        >
          {{ runtimeFeedback }}
        </p>
        <p
          v-if="checkingRuntime"
          class="muted-copy"
          role="status"
        >
          检测中
        </p>
        <template v-if="diagnostics">
          <dl class="runtime-list">
            <div>
              <dt>Playwright Worker</dt><dd :class="diagnostics.workerReady ? 'text-success' : 'text-danger'">
                <AppIcon
                  :name="diagnostics.workerReady ? 'check' : 'alert'"
                  :size="16"
                />{{ diagnostics.workerReady ? '可用' : '不可用' }}
              </dd>
            </div>
            <div><dt>Node.js</dt><dd>{{ diagnostics.nodeVersion || '—' }}</dd></div>
            <div><dt>运行方式</dt><dd>系统 Node.js</dd></div>
            <div><dt>Playwright</dt><dd>{{ diagnostics.playwrightVersion || '—' }}</dd></div>
            <div><dt>Chromium</dt><dd>{{ diagnostics.browserVersion || '—' }}</dd></div>
            <div>
              <dt>SQLite</dt><dd :class="diagnostics.databaseReady ? 'text-success' : 'text-danger'">
                {{ diagnostics.databaseReady ? '可用' : '不可用' }}
              </dd>
            </div>
            <div><dt>应用版本</dt><dd>{{ diagnostics.version }}</dd></div>
          </dl>
          <div
            v-if="diagnostics.nodePath"
            class="runtime-path"
          >
            <span>Node.js 路径</span><code>{{ diagnostics.nodePath }}</code>
          </div>
          <div
            v-if="diagnostics.playwrightPath"
            class="runtime-path"
          >
            <span>Playwright 路径</span><code>{{ diagnostics.playwrightPath }}</code>
          </div>
          <div class="runtime-path">
            <span>数据目录</span><code>{{ diagnostics.dataDir }}</code><button
              class="secondary-button"
              type="button"
              @click="openDirectory"
            >
              <AppIcon
                name="folder"
                :size="16"
              />打开目录
            </button>
          </div>
          <div class="runtime-path">
            <span>数据库</span><code>{{ diagnostics.dbPath }}</code>
          </div>
          <div class="runtime-path">
            <span>配置文件</span><code>{{ diagnostics.settingsPath }}</code>
          </div>
          <p
            v-if="diagnostics.error"
            class="error-copy"
            role="alert"
          >
            {{ diagnostics.error }}
          </p>
        </template>
        <p
          v-if="runtimeError"
          class="error-copy"
          role="alert"
        >
          {{ runtimeError }}
        </p>
        <CodeViewer
          v-if="repairLog"
          class="runtime-repair-log"
          title="playwright-repair.log"
          :code="repairLog"
          :auto-scroll="tasks.runtimeRepairing"
        />
      </section>
    </div>
  </section>
</template>
