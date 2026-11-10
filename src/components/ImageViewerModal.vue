<script setup lang="ts">
import { nextTick, ref, watch } from 'vue';
import type { ArtifactFile } from '@/types/models';
import { artifactImageUrl } from '@/lib/studio';
import AppIcon from '@/components/icons/AppIcon.vue';

const props = defineProps<{ image: ArtifactFile | null }>();
const emit = defineEmits<{ close: [] }>();
const dialog = ref<HTMLDialogElement | null>(null);
watch(() => props.image, async (image) => {
  await nextTick();
  if (image && !dialog.value?.open) dialog.value?.showModal();
  else if (!image) dialog.value?.close();
}, { immediate: true });
</script>

<template>
  <dialog
    ref="dialog"
    class="image-viewer"
    aria-label="截图预览"
    @cancel.prevent="emit('close')"
    @click="($event.target === dialog) && emit('close')"
  >
    <div
      v-if="image"
      class="image-viewer__content"
    >
      <header>
        <strong>{{ image.label }}</strong>
        <button
          class="icon-button"
          type="button"
          title="关闭预览"
          aria-label="关闭预览"
          @click="emit('close')"
        >
          <AppIcon name="x" />
        </button>
      </header>
      <img
        :src="artifactImageUrl(image)"
        :alt="image.label"
      >
    </div>
  </dialog>
</template>
