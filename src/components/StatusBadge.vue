<script setup lang="ts">
import { computed } from 'vue';
import { statusLabel, statusTone } from '@/lib/format';
import type { TaskStatus } from '@/types/models';

const props = defineProps<{
  status: TaskStatus;
}>();

const tone = computed(() => statusTone(props.status));
</script>

<template>
  <span
    class="status-badge"
    :data-tone="tone"
  >{{ statusLabel(status) }}</span>
</template>
