import { describe, expect, it } from 'vitest';
import { getShortcutAction } from './shortcuts';

function keyboardEvent(key: string, init: KeyboardEventInit = {}): KeyboardEvent {
  return new KeyboardEvent('keydown', { key, ...init });
}

describe('getShortcutAction', () => {
  it('maps platform modifier shortcuts to import and navigation actions', () => {
    expect(getShortcutAction(keyboardEvent('o', { ctrlKey: true }))).toBe('import');
    expect(getShortcutAction(keyboardEvent(',', { metaKey: true }))).toBe('openSettings');
    expect(getShortcutAction(keyboardEvent('2'))).toBe('plugins');
  });

  it('ignores modified or editable-field key presses', () => {
    expect(getShortcutAction(keyboardEvent('1', { shiftKey: true }))).toBeNull();
    expect(getShortcutAction(keyboardEvent('1', { altKey: true }))).toBeNull();

    const input = document.createElement('input');
    document.body.append(input);
    expect(getShortcutAction(new KeyboardEvent('keydown', { key: '1', bubbles: true }))).toBe('library');
    const inputEvent = keyboardEvent('1');
    Object.defineProperty(inputEvent, 'target', { value: input });
    expect(getShortcutAction(inputEvent)).toBeNull();
    input.remove();
  });
});
