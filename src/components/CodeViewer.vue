<script setup lang="ts">
import { computed, nextTick, onUnmounted, ref, watch } from 'vue';
import AppIcon from '@/components/icons/AppIcon.vue';
import { copyText } from '@/lib/clipboard';
import { tokenizeJson } from '@/lib/studio';

const props = withDefaults(defineProps<{
  title: string;
  code?: string;
  value?: unknown;
  language?: 'json' | 'text';
  autoScroll?: boolean;
}>(), { code: undefined, value: undefined, language: 'text', autoScroll: false });
const pretty = ref(true);
const copied = ref(false);
const error = ref('');
const body = ref<HTMLElement | null>(null);
let timer: number | undefined;

const rendered = computed(() => {
  if (props.value !== undefined) return JSON.stringify(props.value, null, pretty.value ? 2 : 0) ?? '';
  if (props.language === 'json' && props.code) {
    try { return JSON.stringify(JSON.parse(props.code), null, pretty.value ? 2 : 0); }
    catch { return props.code; }
  }
  return props.code ?? '';
});
const tokens = computed(() => props.language === 'json' ? tokenizeJson(rendered.value) : [{ text: rendered.value, kind: '' }]);
const lineNumbers = computed(() => Array.from({ length: rendered.value.split('\n').length }, (_, index) => index + 1).join('\n'));

watch(() => [rendered.value, props.autoScroll], async () => {
  if (!props.autoScroll) return;
  await nextTick();
  if (body.value) body.value.scrollTop = body.value.scrollHeight;
}, { immediate: true });
onUnmounted(() => window.clearTimeout(timer));

async function copy() {
  error.value = '';
  try {
    await copyText(rendered.value);
    copied.value = true;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => { copied.value = false; }, 2000);
  } catch (cause) {
    error.value = cause instanceof Error ? cause.message : '复制失败';
  }
}
</script>

<template>
  <section class="code-viewer">
    <header class="code-viewer__header">
      <span class="code-viewer__title"><AppIcon
        name="file"
        :size="16"
      />{{ title }}</span>
      <div class="toolbar-actions">
        <button
          v-if="language === 'json'"
          class="icon-button"
          type="button"
          :title="pretty ? '紧凑格式' : '格式化'"
          :aria-label="pretty ? '紧凑格式' : '格式化'"
          @click="pretty = !pretty"
        >
          <AppIcon
            name="code"
            :size="16"
          />
        </button>
        <button
          class="icon-button"
          type="button"
          :disabled="!rendered"
          :aria-label="copied ? '已复制' : '复制代码'"
          :title="copied ? '已复制' : '复制代码'"
          @click="copy"
        >
          <AppIcon
            :name="copied ? 'check' : 'copy'"
            :size="16"
          />
        </button>
      </div>
    </header>
    <p
      v-if="error"
      class="error-copy"
      role="alert"
    >
      {{ error }}
    </p>
    <span
      class="sr-only"
      aria-live="polite"
    >{{ copied ? '已复制' : '' }}</span>
    <div
      ref="body"
      class="code-viewer__body"
      tabindex="0"
      :aria-label="title"
    >
      <pre
        v-if="rendered"
        class="code-viewer__numbers"
        aria-hidden="true"
      >{{ lineNumbers }}</pre>
      <pre class="code-viewer__source"><code v-if="rendered"><span
        v-for="(token, index) in tokens"
        :key="index"
        :class="token.kind ? `syntax-${token.kind}` : undefined"
      >{{ token.text }}</span></code><span
v-else
                                                                                                                                                                             class="code-empty"
      >暂无内容</span></pre>
    </div>
  </section>
</template>
