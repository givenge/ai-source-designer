import type { AiSettings } from '@/types/models';

const SETTINGS_KEY = 'ai-source-designer:settings';

export function loadFallbackSettings(): AiSettings | null {
  if (typeof window === 'undefined') {
    return null;
  }

  const raw = window.localStorage.getItem(SETTINGS_KEY);
  if (!raw) {
    return null;
  }

  try {
    return JSON.parse(raw) as AiSettings;
  } catch {
    return null;
  }
}

export function saveFallbackSettings(settings: AiSettings): void {
  if (typeof window === 'undefined') {
    return;
  }

  window.localStorage.setItem(SETTINGS_KEY, JSON.stringify(settings));
}
