import * as React from 'react';
import { Monitor, Moon, Sun } from 'lucide-react';
import { Check } from 'lucide-react';
import { useState } from 'react';
import { Button } from '@/components/ui';
import { useAppStore } from '@/stores/app';
import type { Theme } from '@/types';

const themes: Array<{ value: Theme; label: string; icon: typeof Sun }> = [
  { value: 'system', label: '跟随系统', icon: Monitor },
  { value: 'light', label: '浅色', icon: Sun },
  { value: 'dark', label: '深色', icon: Moon },
];

export function ThemeToggle() {
  const theme = useAppStore((state) => state.settings.theme);
  const updateSettings = useAppStore((state) => state.updateSettings);
  const [open, setOpen] = useState(false);

  const current = themes.find((t) => t.value === theme);

  return (
    <div className="relative">
      <Button
        variant="ghost"
        size="icon"
        onPress={() => setOpen(!open)}
        aria-label={`当前主题：${current?.label ?? '系统'}`}
        title={`当前主题：${current?.label ?? '系统'}`}
        aria-expanded={open}
        aria-haspopup="listbox"
      >
        {current ? React.createElement(current.icon, { className: 'h-4 w-4', 'aria-hidden': 'true' }) : <Monitor className="h-4 w-4" />}
      </Button>

      {open && (
        <>
          <div
            className="fixed inset-0 z-40"
            onClick={() => setOpen(false)}
            aria-hidden="true"
          />
          <div className="fixed bottom-20 left-4 z-50 rounded-xl border border-divider bg-background p-2 shadow-lg min-w-[140px]">
            <ul role="listbox" aria-label="选择主题">
              {themes.map((t) => (
                <li key={t.value}>
                  <button
                    role="option"
                    aria-selected={theme === t.value}
                    onClick={() => {
                      void updateSettings({ ...useAppStore.getState().settings, theme: t.value });
                      setOpen(false);
                    }}
                    className={`flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors ${
                      theme === t.value
                        ? 'bg-primary/15 text-primary'
                        : 'text-default-500 hover:bg-default-100 hover:text-foreground'
                    }`}
                  >
                    {React.createElement(t.icon, { className: 'h-4 w-4 shrink-0', 'aria-hidden': 'true' })}
                    {t.label}
                    {theme === t.value && (
                      <Check className="h-4 w-4 text-primary" aria-hidden="true" />
                    )}
                  </button>
                </li>
              ))}
            </ul>
          </div>
        </>
      )}
    </div>
  );
}