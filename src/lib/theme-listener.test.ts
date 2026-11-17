import { afterEach, describe, expect, it, vi } from 'vitest';
import { listenToSystemTheme, SYSTEM_THEME_QUERY } from './theme';

describe('listenToSystemTheme', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('registers a change listener and removes it on cleanup', () => {
    const addEventListener = vi.fn();
    const removeEventListener = vi.fn();
    const media = {
      addEventListener,
      removeEventListener,
    } as unknown as MediaQueryList;
    const matchMedia = vi.spyOn(window, 'matchMedia').mockReturnValue(media);
    const onChange = vi.fn();

    const cleanup = listenToSystemTheme(onChange);

    expect(matchMedia).toHaveBeenCalledWith(SYSTEM_THEME_QUERY);
    expect(addEventListener).toHaveBeenCalledWith('change', expect.any(Function));
    const handler = addEventListener.mock.calls[0]?.[1] as (event: MediaQueryListEvent) => void;
    handler({ matches: true } as MediaQueryListEvent);
    expect(onChange).toHaveBeenCalledWith(true);

    cleanup();
    expect(removeEventListener).toHaveBeenCalledWith('change', handler);
  });
});
