<script setup lang="ts">
import { computed } from 'vue';
import { statusLabel, statusTone } from '@/lib/format';
import type { TaskStatus } from '@/types/models';

const props = defineProps<{
  status: TaskStatus;
}>();

const tone = computed(() => statusTone(props.status));
const active = computed(() => ['sampling', 'synthesizing', 'validating', 'repairing'].includes(props.status));
</script>

<template>
  <span
    class="status-badge"
    :data-tone="tone"
    :data-active="active"
  ><span
    class="status-dot"
    aria-hidden="true"
  />{{ statusLabel(status) }}</span>
</template>
