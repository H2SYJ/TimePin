import { Film, FolderOpen, Settings as SettingsIcon, Sparkles } from 'lucide-react';
import { cn } from '@/lib/utils';
import { useAppStore } from '@/stores/app';

type Page = 'library' | 'settings';

const navigation: Array<{ page: Page; label: string; icon: typeof Film }> = [
  { page: 'library', label: '媒体库', icon: FolderOpen },
  { page: 'settings', label: '软件设置', icon: SettingsIcon },
];

export function Sidebar() {
  const page = useAppStore((state) => state.page);
  const videos = useAppStore((state) => state.videos);
  const setPage = useAppStore((state) => state.setPage);
  const videoCount = videos.length;

  return (
    <aside className="flex h-full w-60 shrink-0 flex-col border-r border-border bg-secondary/40 px-3 py-5">
      <div className="flex items-center gap-3 px-3 pb-7">
        <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-primary text-primary-foreground">
          <Film className="h-5 w-5" aria-hidden="true" />
        </div>
        <div className="min-w-0">
          <p className="text-base font-semibold tracking-tight">TimePin</p>
          <p className="truncate text-xs text-muted-foreground">视频标注工作台</p>
        </div>
      </div>

      <p className="px-3 pb-2 text-xs font-medium uppercase tracking-wider text-muted-foreground">
        工作区
      </p>
      <nav className="space-y-1" aria-label="主导航">
        {navigation.map((item) => {
          const Icon = item.icon;
          const active = page === item.page;
          return (
            <button
              key={item.page}
              type="button"
              onClick={() => setPage(item.page)}
              aria-current={active ? 'page' : undefined}
              className={cn(
                'flex h-10 w-full items-center gap-3 rounded-lg px-3 text-sm font-medium transition-colors',
                active
                  ? 'bg-background text-primary'
                  : 'text-muted-foreground hover:bg-background/70 hover:text-foreground',
              )}
            >
              <Icon className="h-4 w-4" aria-hidden="true" />
              <span>{item.label}</span>
              {item.page === 'library' && videoCount > 0 && (
                <span className="ml-auto rounded-md bg-secondary px-1.5 py-0.5 text-xs tabular-nums text-muted-foreground">
                  {videoCount}
                </span>
              )}
            </button>
          );
        })}
      </nav>

      <div className="mt-auto rounded-xl border border-border bg-background/70 p-4">
        <div className="mb-2 flex items-center gap-2 text-xs font-medium text-foreground">
          <Sparkles className="h-3.5 w-3.5 text-primary" aria-hidden="true" />
          M1 原型
        </div>
        <p className="text-xs leading-relaxed text-muted-foreground">
          当前版本聚焦设置持久化、视频导入与媒体库。时间轴与插件将在后续里程碑开放。
        </p>
      </div>
    </aside>
  );
}