import { describe, it, expect } from 'vitest';
import { formatDuration, formatFileSize, formatDate } from './utils';

describe('utils', () => {
  describe('formatDuration', () => {
    it('formats seconds correctly', () => {
      expect(formatDuration(0)).toBe('00:00');
      expect(formatDuration(65)).toBe('01:05');
      expect(formatDuration(3661)).toBe('1:01:01');
    });
  });

  describe('formatFileSize', () => {
    it('formats bytes correctly', () => {
      expect(formatFileSize(0)).toBe('0 B');
      expect(formatFileSize(1024)).toBe('1.0 KB');
      expect(formatFileSize(1024 * 1024)).toBe('1.0 MB');
      expect(formatFileSize(1024 * 1024 * 1024)).toBe('1.0 GB');
    });
  });

  describe('formatDate', () => {
    it('formats ISO date string', () => {
      const date = '2024-01-15T10:30:00.000Z';
      const result = formatDate(date);
      // Chinese locale outputs "MM/DD HH:MM"
      expect(result).toContain('01');
      expect(result).toContain('15');
    });
  });
});