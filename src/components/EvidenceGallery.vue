<script setup lang="ts">
import { computed } from 'vue';
import AppIcon from '@/components/icons/AppIcon.vue';
import { artifactImageUrl } from '@/lib/studio';
import type { ArtifactBundle, ArtifactFile } from '@/types/models';

const props = withDefaults(defineProps<{ artifacts: ArtifactBundle | null; highlightRole?: string }>(), { highlightRole: '' });
const emit = defineEmits<{ select: [file: ArtifactFile] }>();
const screenshots = computed(() => props.artifacts?.screenshots ?? []);
</script>

<template>
  <div
    v-if="screenshots.length"
    class="evidence-gallery"
  >
    <figure
      v-for="shot in screenshots"
      :key="shot.path"
      class="shot-card"
      :data-highlight="Boolean(highlightRole && shot.label === `${highlightRole}.png`)"
    >
      <button
        type="button"
        :aria-label="`预览 ${shot.label}`"
        :title="shot.path"
        @click="emit('select', shot)"
      >
        <img
          :src="artifactImageUrl(shot)"
          :alt="shot.label"
          loading="lazy"
        >
        <span class="shot-card__zoom"><AppIcon
          name="search"
          :size="18"
        /></span>
      </button>
      <figcaption>
        <AppIcon
          name="image"
          :size="14"
        /><strong>{{ shot.label }}</strong>
      </figcaption>
    </figure>
  </div>
  <div
    v-else
    class="empty-state"
  >
    <AppIcon
      name="image"
      :size="32"
    /><strong>暂无截图</strong>
  </div>
</template>
