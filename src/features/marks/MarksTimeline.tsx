import type { Mark } from '@/types';
import { formatTime } from './format';

interface MarksTimelineProps {
  marks: Mark[];
  duration: number;
  pendingStart: number | null;
  currentTime: number;
  canSeek: boolean;
  onSeek: (time: number) => void;
}

export function MarksTimeline({ marks, duration, pendingStart, currentTime, canSeek, onSeek }: MarksTimelineProps) {
  const percent = (time: number) => `${(Math.min(Math.max(time, 0), duration) / duration) * 100}%`;

  return (
    <div role="group" aria-label="标注轨道" className="rounded-xl border border-divider bg-background p-3">
      <p className="mb-3 text-xs text-default-500">上轨：时间点 · 下轨：区间 · 点击标注可跳转</p>
      {duration > 0 ? (
        <div className="relative h-24 rounded-lg border border-divider bg-default-50">
          {marks.map((mark) => {
            const start = Math.min(Math.max(mark.startTime, 0), duration);
            const end = Math.min(Math.max(mark.endTime ?? mark.startTime, start), duration);
            return (
              <button key={mark.id} type="button" disabled={!canSeek}
                aria-label={`时间轴：${mark.label}，${formatTime(mark.startTime)}${mark.endTime !== null ? ` 至 ${formatTime(mark.endTime)}` : ''}`}
                title={`${mark.label} · ${formatTime(mark.startTime)}`}
                className={mark.endTime === null
                  ? 'absolute top-2 h-8 w-6 -translate-x-1/2 rounded text-primary focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary'
                  : 'absolute top-12 h-8 min-w-1 rounded border border-primary bg-primary/20 focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary'}
                style={{ left: percent(start), ...(mark.endTime !== null ? { width: percent(end - start) } : {}) }}
                onClick={() => onSeek(mark.startTime)}>
                {mark.endTime === null ? <span aria-hidden="true" className="mx-auto block h-6 w-1 rounded bg-primary" /> : null}
              </button>
            );
          })}
          {pendingStart !== null ? (
            <div aria-label={`待完成区间起点 ${formatTime(pendingStart)}`} className="pointer-events-none absolute inset-y-0 w-0 border-l-2 border-dashed border-warning" style={{ left: percent(pendingStart) }} />
          ) : null}
          <div aria-hidden="true" className="pointer-events-none absolute inset-y-0 w-0.5 bg-foreground" style={{ left: percent(currentTime) }} />
        </div>
      ) : <p className="text-sm text-default-500">视频时长不可用，时间轴尚未就绪。</p>}
      <div className="mt-2 flex justify-between font-mono text-xs text-default-500"><span>{formatTime(0)}</span><span>{formatTime(duration)}</span></div>
    </div>
  );
}
