import { describe, expect, it } from 'vitest';
import { formatDate, maskSecret, statusLabel } from '@/lib/format';

describe('format utilities', () => {
  it('masks a secret without leaking the full value', () => {
    expect(maskSecret('sk-test-secret-1234')).toBe('sk-t••••234');
  });

  it('keeps invalid dates readable', () => {
    expect(formatDate('not-a-date')).toBe('not-a-date');
  });

  it('maps statuses to chinese labels', () => {
    expect(statusLabel('validating')).toBe('验证中');
  });
});
