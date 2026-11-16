import {
  Film,
  FolderOpen,
  MapPin,
  Puzzle,
  Settings as SettingsIcon,
} from 'lucide-react';
import { cn } from '@/lib/utils';
import { useAppStore } from '@/stores/app';

type Page = 'library' | 'marks' | 'plugins' | 'settings';

const navigation: Array<{ page: Page; label: string; icon: typeof Film }> = [
  { page: 'library', label: '视频库', icon: FolderOpen },
  { page: 'marks', label: '标注', icon: MapPin },
  { page: 'plugins', label: '插件', icon: Puzzle },
];

const bottomNavigation: Array<{ page: Page; label: string; icon: typeof Film }> = [
  { page: 'settings', label: '设置', icon: SettingsIcon },
];

function RailButton({
  item,
  active,
  badge,
  onPress,
}: {
  item: { page: Page; label: string; icon: typeof Film };
  active: boolean;
  badge?: number;
  onPress: () => void;
}) {
  const Icon = item.icon;
  return (
    <button
      type="button"
      onClick={onPress}
      aria-current={active ? 'page' : undefined}
      title={item.label}
      className={cn(
        'group relative flex h-14 w-14 flex-col items-center justify-center gap-1 rounded-xl transition-colors',
        active
          ? 'bg-primary/15 text-primary'
          : 'text-default-400 hover:bg-default-100 hover:text-foreground',
      )}
    >
      <Icon className="h-5 w-5" aria-hidden="true" />
      <span className="text-[10px] font-medium leading-none">{item.label}</span>
      {badge != null && badge > 0 && (
        <span className="absolute -right-0.5 -top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-primary px-1 text-[10px] font-semibold text-primary-foreground">
          {badge > 99 ? '99+' : badge}
        </span>
      )}
    </button>
  );
}

export function Sidebar() {
  const page = useAppStore((state) => state.page);
  const videos = useAppStore((state) => state.videos);
  const setPage = useAppStore((state) => state.setPage);
  const videoCount = videos.length;

  return (
    <aside className="flex h-full w-[72px] shrink-0 flex-col items-center border-r border-divider bg-default-50/50 py-4">
      {/* Logo */}
      <div className="mb-6 flex h-10 w-10 items-center justify-center rounded-xl bg-primary text-primary-foreground">
        <Film className="h-5 w-5" aria-hidden="true" />
      </div>

      {/* 主导航 */}
      <nav className="flex flex-col items-center gap-1" aria-label="主导航">
        {navigation.map((item) => (
          <RailButton
            key={item.page}
            item={item}
            active={page === item.page}
            badge={item.page === 'library' ? videoCount : undefined}
            onPress={() => setPage(item.page)}
          />
        ))}
      </nav>

      {/* 底部：设置 */}
      <div className="mt-auto flex flex-col items-center gap-1">
        {bottomNavigation.map((item) => (
          <RailButton
            key={item.page}
            item={item}
            active={page === item.page}
            onPress={() => setPage(item.page)}
          />
        ))}
      </div>
    </aside>
  );
}
