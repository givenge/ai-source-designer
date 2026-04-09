<script setup lang="ts">
import { computed } from 'vue';

const props = defineProps<{
  title: string;
  value: unknown;
}>();

const rendered = computed(() => JSON.stringify(props.value, null, 2));
</script>

<template>
  <section class="panel">
    <header class="panel-header">
      <h3>{{ title }}</h3>
    </header>
    <pre class="code-block">{{ rendered }}</pre>
  </section>
</template>
