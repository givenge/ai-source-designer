<script setup lang="ts">
import { ref } from 'vue';
import AppIcon from '@/components/icons/AppIcon.vue';
import type { IconName } from '@/components/icons/paths';

const props = withDefaults(defineProps<{
  modelValue: string;
  options: Array<{ value: string; label: string; icon?: IconName; disabled?: boolean }>;
  label: string;
  id?: string;
  tabs?: boolean;
}>(), { id: undefined, tabs: false });
const emit = defineEmits<{ 'update:modelValue': [value: string] }>();
const root = ref<HTMLElement | null>(null);

function navigate(event: KeyboardEvent) {
  if (!props.tabs || !['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  event.preventDefault();
  const options = props.options.filter((option) => !option.disabled);
  if (!options.length) return;
  const current = options.findIndex((option) => option.value === props.modelValue);
  const index = event.key === 'Home' ? 0 : event.key === 'End' ? options.length - 1
    : (current + (event.key === 'ArrowRight' ? 1 : -1) + options.length) % options.length;
  const option = options[index];
  emit('update:modelValue', option.value);
  root.value?.querySelector<HTMLButtonElement>(`button[data-value="${option.value}"]`)?.focus();
}
</script>

<template>
  <div
    ref="root"
    class="segmented-control"
    :role="tabs ? 'tablist' : 'group'"
    :aria-label="label"
    @keydown="navigate"
  >
    <button
      v-for="option in options"
      :id="id ? `${id}-tab-${option.value}` : undefined"
      :key="option.value"
      type="button"
      :role="tabs ? 'tab' : undefined"
      :aria-controls="tabs && id ? `${id}-panel-${option.value}` : undefined"
      :aria-selected="tabs ? option.value === modelValue : undefined"
      :aria-pressed="tabs ? undefined : option.value === modelValue"
      :tabindex="tabs && option.value !== modelValue ? -1 : 0"
      :disabled="option.disabled"
      :data-value="option.value"
      :data-active="option.value === modelValue"
      @click="emit('update:modelValue', option.value)"
    >
      <AppIcon
        v-if="option.icon"
        :name="option.icon"
        :size="16"
      />
      {{ option.label }}
    </button>
  </div>
</template>
