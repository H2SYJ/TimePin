import { useEffect } from 'react';
import { getShortcutAction } from '@/lib/shortcuts';

interface MarksHandlers {
  onPoint: () => void;
  onInterval: () => void;
  onCancel: () => void;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable
    || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
    || target.closest('input, textarea, select, [contenteditable]:not([contenteditable="false"])') !== null;
}

export function resolveMarksShortcut(event: KeyboardEvent): 'point' | 'interval' | 'cancel' | null {
  // Editable fields, repeat, and modifier chords never fire marking actions.
  if (event.defaultPrevented || event.isComposing || event.repeat || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey
    || isEditableTarget(event.target)) return null;
  const key = event.key.toLowerCase();
  if (key === 'm') return 'point';
  if (key === 'r') return 'interval';
  if (key === 'escape') return 'cancel';
  return null;
}

/**
 * Marking keys claim only their own events; digits and chords keep flowing to the
 * global navigation/import shortcuts handled by useKeyboardShortcuts.
 */
export function useMarksShortcuts({ onPoint, onInterval, onCancel }: MarksHandlers): void {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const action = resolveMarksShortcut(event);
      if (action === null || getShortcutAction(event) !== null) return;
      event.preventDefault();
      if (action === 'point') onPoint();
      else if (action === 'interval') onInterval();
      else onCancel();
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onPoint, onInterval, onCancel]);
}
