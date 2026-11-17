import { lazy, Suspense, useCallback, useEffect } from 'react';
import { Sidebar } from '@/components/Sidebar';
import { api } from '@/lib/ipc';
import { listenToSystemTheme } from '@/lib/theme';
import { type ShortcutAction } from '@/lib/shortcuts';
import { useKeyboardShortcuts } from '@/hooks/useKeyboardShortcuts';
import { useAppStore } from '@/stores/app';
import { cn } from '@/lib/utils';

const LibraryPage = lazy(() => import('@/features/library/LibraryPage').then(({ LibraryPage }) => ({ default: LibraryPage })));
const MarksPage = lazy(() => import('@/features/marks/MarksPage').then(({ MarksPage }) => ({ default: MarksPage })));
const PluginsPage = lazy(() => import('@/features/library/PluginsPage').then(({ PluginsPage }) => ({ default: PluginsPage })));
const SettingsPage = lazy(() => import('@/features/settings/SettingsPage').then(({ SettingsPage }) => ({ default: SettingsPage })));

function PageWrapper({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex h-full min-h-0 min-w-0 flex-1">
      <div className="h-full min-h-0 min-w-0 flex-1">{children}</div>
    </div>
  );
}

function Loading() {
  return (
    <div className="flex h-full min-h-0 items-center justify-center">
      <div className="flex flex-col items-center gap-3 text-default-400">
        <div className="h-6 w-6 animate-spin rounded-full border-2 border-primary border-t-transparent" />
        <span className="text-sm">正在初始化…</span>
      </div>
    </div>
  );
}

function ErrorBoundary({ children }: { children: React.ReactNode }) {
  return (
    <Suspense fallback={<Loading />}>
      {children}
    </Suspense>
  );
}

export default function App() {
  const page = useAppStore((state) => state.page);
  const initialize = useAppStore((state) => state.initialize);
  const loading = useAppStore((state) => state.loading);
  const importPaths = useAppStore((state) => state.importPaths);
  const setPage = useAppStore((state) => state.setPage);

  const openVideoPicker = useCallback(async () => {
    try {
      await importPaths(await api.pickVideoFiles());
    } catch (error) {
      useAppStore.setState({ error: String(error) });
    }
  }, [importPaths]);

  const navigateFromShortcut = useCallback((action: Exclude<ShortcutAction, 'import'>) => {
    if (action === 'openSettings') {
      setPage('settings');
    } else {
      setPage(action);
    }
  }, [setPage]);

  useKeyboardShortcuts({ onImport: openVideoPicker, onNavigate: navigateFromShortcut });

  useEffect(() => {
    let mounted = true;
    initialize().then(() => {
      if (mounted) {
        const state = useAppStore.getState();
        if (state.applyTheme) state.applyTheme();
      }
    });
    return () => {
      mounted = false;
    };
  }, [initialize]);

  useEffect(() => listenToSystemTheme(() => {
    const state = useAppStore.getState();
    if (state.settings.theme === 'system') state.applyTheme();
  }), []);

  if (loading) return <Loading />;

  return (
    <div className={cn('flex h-screen min-h-0 overflow-hidden')}>
      <Sidebar />
      <ErrorBoundary>
        <PageWrapper>
          {page === 'library' ? (
            <LibraryPage />
          ) : page === 'marks' ? (
            <MarksPage />
          ) : page === 'plugins' ? (
            <PluginsPage />
          ) : (
            <SettingsPage />
          )}
        </PageWrapper>
      </ErrorBoundary>
    </div>
  );
}
