import { Clock, MapPin, Tag } from 'lucide-react';
import { Card } from '@/components/ui';
import { useAppStore } from '@/stores/app';

export function MarksPage() {
  const videos = useAppStore((state) => state.videos);
  const selectedVideoId = useAppStore((state) => state.selectedVideoId);
  const video = videos.find((v) => v.id === selectedVideoId) ?? videos[0] ?? null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-4 border-b border-divider px-8 py-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">标注</h1>
          <p className="mt-1 text-sm text-default-500">
            {video ? `${video.name} · 管理时间轴上的标记点与区间` : '请先在视频库选择一个视频'}
          </p>
        </div>
      </header>

      <main className="scrollbar-thin flex min-h-0 flex-1 items-center justify-center overflow-y-auto p-8">
        <Card className="flex w-full max-w-md flex-col items-center gap-4 rounded-2xl border-divider bg-default-50/50 p-10 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/15 text-primary">
            <MapPin className="h-7 w-7" aria-hidden="true" />
          </div>
          <div>
            <h2 className="text-base font-semibold">标注功能即将上线</h2>
            <p className="mt-2 text-sm leading-relaxed text-default-500">
              在视频播放时按 <kbd className="rounded-md border border-divider bg-background px-1.5 py-0.5 font-mono text-xs">M</kbd> 打点，
              或框选区间后按 <kbd className="rounded-md border border-divider bg-background px-1.5 py-0.5 font-mono text-xs">R</kbd> 标记区间。
              支持标签、置信度、来源插件与截图证据。
            </p>
          </div>
          <div className="mt-2 grid w-full grid-cols-3 gap-3 text-left">
            {[
              { icon: Clock, label: '时间点', desc: '精确到毫秒' },
              { icon: Tag, label: '区间', desc: '首尾范围' },
              { icon: MapPin, label: '来源', desc: '插件 / 手动' },
            ].map(({ icon: Icon, label, desc }) => (
              <div key={label} className="rounded-xl border border-divider bg-background/60 p-3">
                <Icon className="mb-1.5 h-4 w-4 text-primary" aria-hidden="true" />
                <p className="text-xs font-medium">{label}</p>
                <p className="mt-0.5 text-[11px] text-default-400">{desc}</p>
              </div>
            ))}
          </div>
        </Card>
      </main>
    </div>
  );
}
