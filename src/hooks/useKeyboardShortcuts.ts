import { useEffect } from 'react';
import { getShortcutAction, type ShortcutAction } from '@/lib/shortcuts';

interface KeyboardShortcutsOptions {
  onImport: () => void | Promise<void>;
  onNavigate: (action: Exclude<ShortcutAction, 'import'>) => void;
}

export function useKeyboardShortcuts({ onImport, onNavigate }: KeyboardShortcutsOptions): void {
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const action = getShortcutAction(event);
      if (!action) return;

      event.preventDefault();
      if (action === 'import') {
        void onImport();
      } else {
        onNavigate(action);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [onImport, onNavigate]);
}
