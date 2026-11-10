import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { copyText } from '@/lib/clipboard';
import { normalizeTheme, resolveTheme, setTheme, themePreference } from '@/lib/theme';

beforeEach(() => vi.stubGlobal('localStorage', { getItem: vi.fn(), setItem: vi.fn() }));
afterEach(() => { vi.restoreAllMocks(); vi.unstubAllGlobals(); document.body.innerHTML = ''; });

describe('theme', () => {
  it('defaults to dark and resolves the system preference', () => {
    expect(normalizeTheme(null)).toBe('dark');
    expect(normalizeTheme('invalid')).toBe('dark');
    expect(resolveTheme('system', false)).toBe('light');
    expect(resolveTheme('system', true)).toBe('dark');
    expect(resolveTheme('light', true)).toBe('light');
  });
  it('applies the theme even if persistence fails', () => {
    vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => { throw new Error('unavailable'); });
    setTheme('light');
    expect(document.documentElement.dataset.theme).toBe('light');
    expect(themePreference.value).toBe('light');
  });
});

describe('clipboard', () => {
  it('copies through the Clipboard API when available', async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal('navigator', { clipboard: { writeText } });
    await copyText('原始 JSON');
    expect(writeText).toHaveBeenCalledWith('原始 JSON');
    expect(document.querySelector('textarea')).toBeNull();
  });
  it.each([true, false])('cleans up the selection fallback and restores focus (success: %s)', async (success) => {
    vi.stubGlobal('navigator', { clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    document.body.innerHTML = '<button>复制</button>';
    const button = document.querySelector('button')!;
    button.focus();
    const execCommand = vi.fn(() => success);
    Object.defineProperty(document, 'execCommand', { configurable: true, value: execCommand });
    const result = copyText('literal <html>');
    if (success) await expect(result).resolves.toBeUndefined();
    else await expect(result).rejects.toThrow('复制失败');
    expect(execCommand).toHaveBeenCalledWith('copy');
    expect(document.querySelector('textarea')).toBeNull();
    expect(document.activeElement).toBe(button);
  });
});
