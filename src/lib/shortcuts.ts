export type ShortcutAction = 'import' | 'openSettings' | 'library' | 'plugins';

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;

  return target.isContentEditable
    || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName)
    || target.closest('input, textarea, select, [contenteditable="true"]') !== null;
}

/** 将浏览器按键事件转换为应用命令，编辑表单内的按键不会触发全局命令。 */
export function getShortcutAction(event: KeyboardEvent): ShortcutAction | null {
  if (isEditableTarget(event.target) || event.altKey || event.shiftKey) return null;

  const key = event.key.toLowerCase();
  if (event.ctrlKey || event.metaKey) {
    if (key === 'o') return 'import';
    if (event.key === ',') return 'openSettings';
    return null;
  }

  if (event.key === '1') return 'library';
  if (event.key === '2') return 'plugins';
  if (event.key === '3') return 'openSettings';
  return null;
}
