import { beforeEach, describe, expect, it } from 'vitest';
import { applyTheme } from './theme';

describe('applyTheme', () => {
  beforeEach(() => {
    document.documentElement.className = '';
    document.documentElement.style.colorScheme = '';
    document.head.innerHTML = '<meta name="theme-color" content="">';
  });

  it('applies dark mode to the document root and theme color', () => {
    applyTheme('dark', document);

    expect(document.documentElement.classList.contains('dark')).toBe(true);
    expect(document.documentElement.style.colorScheme).toBe('dark');
    expect(document.querySelector('meta[name="theme-color"]')?.getAttribute('content')).toBe('#121722');
  });

  it('removes dark mode when switching to light mode', () => {
    document.documentElement.classList.add('dark');
    applyTheme('light', document);

    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(document.documentElement.style.colorScheme).toBe('light');
    expect(document.querySelector('meta[name="theme-color"]')?.getAttribute('content')).toBe('#f8fafc');
  });
});
