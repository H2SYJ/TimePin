import type { Theme } from '@/types';

export const SYSTEM_THEME_QUERY = '(prefers-color-scheme: dark)';
export const LIGHT_THEME_COLOR = '#f8fafc';
export const DARK_THEME_COLOR = '#121722';

export function systemPrefersDark(): boolean {
  return typeof window !== 'undefined'
    && typeof window.matchMedia === 'function'
    && window.matchMedia(SYSTEM_THEME_QUERY).matches;
}

export function applyTheme(theme: Theme, doc?: Document): void {
  const target = doc ?? (typeof document !== 'undefined' ? document : null);
  if (!target) return;

  const isDark = theme === 'dark' || (theme === 'system' && systemPrefersDark());
  const root = target.documentElement;

  root.classList.toggle('dark', isDark);
  root.style.colorScheme = isDark ? 'dark' : 'light';

  const themeColor = target.querySelector<HTMLMetaElement>('meta[name="theme-color"]');
  if (themeColor) {
    themeColor.content = isDark ? DARK_THEME_COLOR : LIGHT_THEME_COLOR;
  }
}

/** 监听系统主题变化；固定浅色/深色模式不会触发回调。 */
export function listenToSystemTheme(onChange: (isDark: boolean) => void): () => void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') {
    return () => {};
  }

  const media = window.matchMedia(SYSTEM_THEME_QUERY);
  const handleChange = (event: MediaQueryListEvent) => onChange(event.matches);

  if (typeof media.addEventListener === 'function') {
    media.addEventListener('change', handleChange);
    return () => media.removeEventListener('change', handleChange);
  }

  if (typeof media.addListener === 'function') {
    media.addListener(handleChange);
    return () => media.removeListener(handleChange);
  }

  return () => {};
}
