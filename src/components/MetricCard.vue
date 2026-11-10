<script setup lang="ts">
import AppIcon from '@/components/icons/AppIcon.vue';
import type { IconName } from '@/components/icons/paths';

defineProps<{ label: string; value: number | string; icon: IconName }>();
</script>

<template>
  <article class="metric-card">
    <div>
      <span>{{ label }}</span>
      <strong>{{ value }}</strong>
    </div>
    <AppIcon
      :name="icon"
      :size="24"
    />
  </article>
</template>
