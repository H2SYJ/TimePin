import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { getShortcutAction } from '@/lib/shortcuts';
import { resolveMarksShortcut, useMarksShortcuts } from './useMarksShortcuts';
import { formatTime, normalizeInterval } from './format';

afterEach(cleanup);

function actionFrom(target: HTMLElement, init: KeyboardEventInit) {
  const event = new KeyboardEvent('keydown', { key: 'm', bubbles: true, cancelable: true, ...init });
  let action: ReturnType<typeof resolveMarksShortcut> = null;
  const listener = (received: KeyboardEvent) => { action = resolveMarksShortcut(received); };
  target.addEventListener('keydown', listener);
  target.dispatchEvent(event);
  target.removeEventListener('keydown', listener);
  return action;
}

describe('mark keyboard shortcuts', () => {
  it('resolves only unmodified M/R/Escape and ignores repeats/composition/claimed events', () => {
    const body = document.body;
    expect(actionFrom(body, { key: 'M' })).toBe('point');
    expect(actionFrom(body, { key: 'r' })).toBe('interval');
    expect(actionFrom(body, { key: 'Escape' })).toBe('cancel');
    for (const modifier of ['ctrlKey', 'metaKey', 'altKey', 'shiftKey', 'repeat', 'isComposing']) {
      expect(actionFrom(body, { [modifier]: true })).toBeNull();
    }
    for (const key of ['1', '2', '3', ',', 'o', ' ']) expect(actionFrom(body, { key })).toBeNull();
    const event = new KeyboardEvent('keydown', { key: 'm', cancelable: true });
    event.preventDefault();
    expect(resolveMarksShortcut(event)).toBeNull();
  });

  it('ignores input, textarea, select and all editable descendants', () => {
    const view = render(<div>
      <input aria-label="input" /><textarea aria-label="textarea" /><select aria-label="select"><option>x</option></select>
      <div contentEditable suppressContentEditableWarning><span>editable</span></div>
      <div contentEditable="plaintext-only" suppressContentEditableWarning><span>plain editable</span></div>
    </div>);
    for (const label of ['input', 'textarea', 'select']) expect(actionFrom(view.getByLabelText(label), {})).toBeNull();
    expect(actionFrom(view.getByText('editable'), {})).toBeNull();
    expect(actionFrom(view.getByText('plain editable'), {})).toBeNull();
  });

  it('leaves application shortcuts untouched and removes its listener on unmount', () => {
    const point = vi.fn();
    const interval = vi.fn();
    const cancel = vi.fn();
    const navigation = vi.fn();
    const globalHandler = (event: KeyboardEvent) => { const action = getShortcutAction(event); if (action) navigation(action); };
    window.addEventListener('keydown', globalHandler);
    function Harness() { useMarksShortcuts({ onPoint: point, onInterval: interval, onCancel: cancel }); return null; }
    const view = render(<Harness />);
    fireEvent.keyDown(window, { key: 'm' });
    fireEvent.keyDown(window, { key: 'r' });
    fireEvent.keyDown(window, { key: 'Escape' });
    fireEvent.keyDown(window, { key: '1' });
    fireEvent.keyDown(window, { key: 'o', ctrlKey: true });
    expect(point).toHaveBeenCalledTimes(1);
    expect(interval).toHaveBeenCalledTimes(1);
    expect(cancel).toHaveBeenCalledTimes(1);
    expect(navigation.mock.calls).toEqual([['library'], ['import']]);
    view.unmount();
    fireEvent.keyDown(window, { key: 'm' });
    expect(point).toHaveBeenCalledTimes(1);
    window.removeEventListener('keydown', globalHandler);
  });
});

describe('mark time helpers', () => {
  it('normalizes reverse seeks without rounding exact player times, and rejects equal/nonfinite endpoints', () => {
    expect(normalizeInterval(20.123456, 10.987654)).toEqual({ startTime: 10.987654, endTime: 20.123456 });
    expect(normalizeInterval(1, 1.0000001)).not.toBeNull();
    expect(normalizeInterval(1, 1)).toBeNull();
    expect(normalizeInterval(NaN, 2)).toBeNull();
    expect(normalizeInterval(1, Infinity)).toBeNull();
  });
  it('formats millisecond rollover without producing an invalid time', () => {
    expect(formatTime(59.9999)).toBe('00:01:00.000');
    expect(formatTime(3600.1234)).toBe('01:00:00.123');
    expect(formatTime(NaN)).toBe('00:00:00.000');
  });
});
