import { ref } from 'vue';

export type ThemePreference = 'dark' | 'light' | 'system';
const storageKey = 'aisd-theme';

export function normalizeTheme(value: string | null): ThemePreference {
  return value === 'light' || value === 'system' ? value : 'dark';
}

export function resolveTheme(preference: ThemePreference, systemDark: boolean): 'dark' | 'light' {
  return preference === 'system' ? (systemDark ? 'dark' : 'light') : preference;
}

function loadTheme(): ThemePreference {
  try {
    return normalizeTheme(window.localStorage.getItem(storageKey));
  } catch {
    return 'dark';
  }
}

export const themePreference = ref<ThemePreference>(loadTheme());

function applyTheme() {
  const systemDark = window.matchMedia?.('(prefers-color-scheme: dark)').matches ?? true;
  document.documentElement.dataset.theme = resolveTheme(themePreference.value, systemDark);
}

export function setTheme(preference: ThemePreference) {
  themePreference.value = preference;
  applyTheme();
  try {
    window.localStorage.setItem(storageKey, preference);
  } catch {
    // The theme still works when local storage is unavailable.
  }
}

export function initializeTheme() {
  applyTheme();
  window.matchMedia?.('(prefers-color-scheme: dark)').addEventListener('change', applyTheme);
}
