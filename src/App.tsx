import { Suspense, useEffect } from 'react';
import { Sidebar } from '@/components/Sidebar';
import { LibraryPage } from '@/features/library/LibraryPage';
import { PluginsPage } from '@/features/library/PluginsPage';
import { SettingsPage } from '@/features/settings/SettingsPage';
import { useAppStore } from '@/stores/app';
import { cn } from '@/lib/utils';

function PageWrapper({ children }: { children: React.ReactNode }) {
  return <div className="flex h-full min-h-0 flex-1">{children}</div>;
}

function Loading() {
  return (
    <div className="flex h-full min-h-0 items-center justify-center">
      <div className="flex flex-col items-center gap-3 text-muted-foreground">
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
  const { page, initialize, loading } = useAppStore();

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

  if (loading) return <Loading />;

  return (
    <div className={cn('flex h-screen min-h-0 overflow-hidden')}>
      <Sidebar />
      <ErrorBoundary>
        <PageWrapper>
          {page === 'library' ? <LibraryPage /> : page === 'plugins' ? <PluginsPage /> : <SettingsPage />}
        </PageWrapper>
      </ErrorBoundary>
    </div>
  );
}