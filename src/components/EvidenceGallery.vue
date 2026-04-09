<script setup lang="ts">
import { computed } from 'vue';
import type { ArtifactBundle } from '@/types/models';

const props = defineProps<{
  artifacts: ArtifactBundle | null;
}>();

const screenshots = computed(() => props.artifacts?.screenshots ?? []);

function toDataUrl(contents: string, contentType: string): string {
  return contentType === 'image/svg+xml'
    ? `data:image/svg+xml;charset=utf-8,${encodeURIComponent(contents)}`
    : `data:${contentType};base64,${contents}`;
}
</script>

<template>
  <section class="panel">
    <header class="panel-header">
      <h3>截图证据</h3>
      <p>保存任务关键页面的可视证据，便于人工回看。</p>
    </header>

    <div
      v-if="screenshots.length"
      class="shot-grid"
    >
      <figure
        v-for="shot in screenshots"
        :key="shot.path"
        class="shot-card"
      >
        <img
          :src="toDataUrl(shot.contents, shot.contentType)"
          :alt="shot.label"
        >
        <figcaption>
          <strong>{{ shot.label }}</strong>
          <span>{{ shot.path }}</span>
        </figcaption>
      </figure>
    </div>

    <p
      v-else
      class="muted-copy"
    >
      当前任务还没有截图产物。
    </p>
  </section>
</template>
