import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { MapPin, Plus, RotateCcw, Scissors } from 'lucide-react';
import { Button, Card } from '@/components/ui';
import { fileSrc } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import { useMarksStore } from '@/stores/marks';
import type { VideoInfo } from '@/types';
import { formatTime, manualMark, normalizeInterval } from './format';
import { MarksList } from './MarksList';
import { MarksTimeline } from './MarksTimeline';
import { useMarksShortcuts } from './useMarksShortcuts';

function MarksWorkspace({ video }: { video: VideoInfo }) {
  const player = useRef<HTMLVideoElement>(null);
  const intervalStart = useRef<number | null>(null);
  const [pendingStart, setPendingStart] = useState<number | null>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [metadataReady, setMetadataReady] = useState(false);
  const [playbackError, setPlaybackError] = useState<string | null>(null);
  const [feedback, setFeedback] = useState('');
  const entry = useMarksStore((state) => state.entries[video.id]);
  const load = useMarksStore((state) => state.load);
  const retrySave = useMarksStore((state) => state.retrySave);
  const canSeek = metadataReady && duration > 0 && !playbackError;
  const canEdit = canSeek && entry?.loadStatus === 'ready';
  const marks = useMemo(() => [...(entry?.marks ?? [])].sort((a, b) => a.startTime - b.startTime), [entry?.marks]);

  useEffect(() => { void load(video.id); }, [load, video.id]);

  const readPlayerTime = useCallback(() => {
    const time = player.current?.currentTime;
    return canEdit && time !== undefined && Number.isFinite(time) && time >= 0 && time <= duration ? time : null;
  }, [canEdit, duration]);

  const addPoint = useCallback(() => {
    const time = readPlayerTime();
    if (time === null) return;
    useMarksStore.getState().add(video.id, manualMark(time, null));
    setFeedback(`已添加时间点 ${formatTime(time)}`);
  }, [readPlayerTime, video.id]);

  const toggleInterval = useCallback(() => {
    const time = readPlayerTime();
    if (time === null) return;
    if (intervalStart.current === null) {
      intervalStart.current = time;
      setPendingStart(time);
      setFeedback(`区间起点 ${formatTime(time)}，再次按 R 完成，Esc 取消`);
      return;
    }
    const interval = normalizeInterval(intervalStart.current, time);
    if (!interval) {
      setFeedback('区间首尾不能相同，请移动播放位置后再次按 R');
      return;
    }
    useMarksStore.getState().add(video.id, manualMark(interval.startTime, interval.endTime));
    intervalStart.current = null;
    setPendingStart(null);
    setFeedback(`已添加区间 ${formatTime(interval.startTime)} – ${formatTime(interval.endTime)}`);
  }, [readPlayerTime, video.id]);

  const cancelInterval = useCallback(() => {
    if (intervalStart.current === null) return;
    intervalStart.current = null;
    setPendingStart(null);
    setFeedback('已取消未完成区间');
  }, []);

  useMarksShortcuts({ onPoint: addPoint, onInterval: toggleInterval, onCancel: cancelInterval });

  const seek = (time: number) => {
    if (!canSeek || !player.current || !Number.isFinite(time)) return;
    try {
      const next = Math.min(Math.max(time, 0), duration);
      player.current.currentTime = next;
      setCurrentTime(next);
    } catch (error) {
      setFeedback(`无法跳转：${String(error)}`);
    }
  };

  const syncMetadata = () => {
    const element = player.current;
    if (!element) return;
    const valid = Number.isFinite(element.duration) && element.duration > 0;
    setDuration(valid ? element.duration : 0);
    if (Number.isFinite(element.currentTime)) setCurrentTime(element.currentTime);
  };

  const reloadPlayer = () => {
    setPlaybackError(null);
    setMetadataReady(false);
    setDuration(0);
    setCurrentTime(0);
    cancelInterval();
    try { player.current?.load(); } catch (error) { setPlaybackError(String(error)); }
  };

  const status = !entry || entry.loadStatus === 'loading' ? '正在读取标注…'
    : entry.loadStatus === 'error' ? '读取失败，编辑已锁定'
    : entry.saveStatus === 'error' ? '保存失败，仍有未保存更改'
    : entry.saveStatus === 'saving' ? '正在保存到本地…'
    : entry.revision > 0 ? '所有更改已保存到本地' : '标注已加载';

  return (
    <main className="scrollbar-thin min-h-0 flex-1 overflow-y-auto p-4 lg:p-8">
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <p role="status" aria-live="polite" className="text-sm text-default-500">{status}</p>
        <p className="text-xs text-default-500">标注单独保存，不修改原视频</p>
      </div>
      {entry?.loadStatus === 'error' ? (
        <div role="alert" className="mb-4 flex flex-wrap items-center gap-3 rounded-xl border border-danger/30 bg-danger/10 p-4">
          <p className="min-w-0 flex-1 break-words text-sm text-danger">无法读取已有标注：{entry.loadError}。为保护原标注，读取成功前不能编辑。</p>
          <Button variant="secondary" onPress={() => { void load(video.id); }}>重试读取</Button>
        </div>
      ) : null}
      {entry?.saveStatus === 'error' ? (
        <div role="alert" className="mb-4 flex flex-wrap items-center gap-3 rounded-xl border border-danger/30 bg-danger/10 p-4">
          <p className="min-w-0 flex-1 break-words text-sm text-danger">{entry.saveError}。更改仅保留在本次应用会话中，请重试后再退出应用。</p>
          <Button variant="secondary" onPress={() => retrySave(video.id)}>重试保存</Button>
        </div>
      ) : null}
      <div className="grid min-w-0 gap-6 xl:grid-cols-[minmax(0,1fr)_minmax(300px,360px)]">
        <section aria-label="视频与时间轴" className="min-w-0 space-y-4">
          <Card className="overflow-hidden rounded-2xl border-divider bg-default-50/50 p-3">
            <video ref={player} controls preload="metadata" src={fileSrc(video.path)} aria-label={`${video.name} 标注播放器`}
              className="aspect-video max-h-[55vh] w-full rounded-xl bg-default-100"
              onLoadedMetadata={() => { syncMetadata(); setMetadataReady(true); }} onDurationChange={syncMetadata}
              onTimeUpdate={() => {
                const time = player.current?.currentTime;
                if (time !== undefined && Number.isFinite(time)) setCurrentTime(time);
              }}
              onSeeked={() => {
                const time = player.current?.currentTime;
                if (time !== undefined && Number.isFinite(time)) setCurrentTime(time);
              }}
              onError={() => { setPlaybackError('无法播放视频。请检查文件是否可访问，以及当前系统是否支持其编码。'); cancelInterval(); }}>
              您的运行环境不支持视频播放。
            </video>
          </Card>
          {playbackError ? (
            <div role="alert" className="flex flex-wrap items-center gap-3 rounded-xl border border-danger/30 bg-danger/10 p-4">
              <p className="flex-1 text-sm text-danger">{playbackError}</p>
              <Button variant="secondary" onPress={reloadPlayer}>重新加载视频</Button>
            </div>
          ) : !metadataReady ? <p className="text-sm text-default-500">正在读取视频时长，播放器就绪后可打标…</p>
            : duration <= 0 ? <p role="alert" className="text-sm text-warning">视频时长不可用，无法打标或跳转。请检查文件。</p> : null}
          <div className="space-y-2">
            <div className="flex items-center justify-between gap-4 text-sm">
              <label htmlFor={`seek-${video.id}`} className="font-medium">播放位置</label>
              <output htmlFor={`seek-${video.id}`} className="font-mono text-xs tabular-nums text-default-500">{formatTime(currentTime)} / {formatTime(duration)}</output>
            </div>
            <input id={`seek-${video.id}`} type="range" min={0} max={duration || 1} step="0.001"
              value={Math.min(Math.max(currentTime, 0), duration || 1)} disabled={!canSeek}
              aria-valuetext={formatTime(currentTime)} onChange={(event) => seek(Number(event.target.value))}
              className="h-8 w-full cursor-pointer accent-primary disabled:cursor-not-allowed" />
          </div>
          <MarksTimeline marks={marks} duration={duration} currentTime={currentTime} pendingStart={pendingStart} canSeek={canSeek} onSeek={seek} />
          <div className="flex flex-wrap gap-3">
            <Button isDisabled={!canEdit} onPress={addPoint}><Plus className="h-4 w-4" aria-hidden="true" />添加时间点（M）</Button>
            <Button variant="secondary" isDisabled={!canEdit} onPress={toggleInterval}><Scissors className="h-4 w-4" aria-hidden="true" />{pendingStart === null ? '开始区间（R）' : '完成区间（R）'}</Button>
            {pendingStart !== null ? <Button variant="ghost" onPress={cancelInterval}><RotateCcw className="h-4 w-4" aria-hidden="true" />取消区间（Esc）</Button> : null}
          </div>
          <p className="text-xs leading-relaxed text-default-500">M：以播放器当前时间打点 · R：开始/结束区间（支持反向定位）· Esc：取消区间。输入文字时不触发快捷键。</p>
          <p role="status" aria-live="polite" className="min-h-5 text-sm text-primary">{feedback}</p>
        </section>
        <section aria-labelledby="marks-list-title" className="min-w-0">
          <h2 id="marks-list-title" className="mb-4 flex items-center gap-2 text-base font-semibold"><MapPin className="h-4 w-4 text-primary" aria-hidden="true" />标注列表 <span className="text-sm font-normal text-default-500">{marks.length} 项</span></h2>
          {entry?.loadStatus === 'ready' ? (
            <MarksList marks={marks} readOnly={false} canSeek={canSeek} onSeek={seek}
              onRename={(mark, label) => useMarksStore.getState().rename(video.id, mark.id, label)}
              onDelete={(mark) => useMarksStore.getState().remove(video.id, mark.id)} />
          ) : <p className="text-sm text-default-500">{entry?.loadStatus === 'error' ? '标注读取失败' : '正在读取标注…'}</p>}
        </section>
      </div>
    </main>
  );
}

export function MarksPage() {
  const videos = useAppStore((state) => state.videos);
  const selectedVideoId = useAppStore((state) => state.selectedVideoId);
  const selectVideo = useAppStore((state) => state.selectVideo);
  const video = videos.find((item) => item.id === selectedVideoId) ?? videos[0] ?? null;

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-4 px-8 py-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">标注工作台</h1>
          <p className="mt-1.5 text-sm text-default-500">{video ? '播放、定位并管理时间点与区间' : '请先导入并选择一个视频'}</p>
        </div>
        {video ? <div className="flex max-w-full items-center gap-3">
          <label htmlFor="marks-video" className="shrink-0 text-sm font-medium">当前视频</label>
          <select
            id="marks-video"
            value={video.id}
            onChange={(event) => selectVideo(event.target.value)}
            aria-label="当前视频"
            className="min-h-10 min-w-0 max-w-xs rounded-lg border border-divider bg-background px-3 text-sm text-foreground transition-colors hover:border-default-300 focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary"
          >
            {videos.map((item) => (
              <option key={item.id} value={item.id}>{item.name}</option>
            ))}
          </select>
        </div> : null}
      </header>
      {video ? <MarksWorkspace key={`${video.id}:${video.path}`} video={video} /> : (
        <main className="flex flex-1 items-center justify-center p-8">
          <Card className="flex max-w-md flex-col items-center gap-4 rounded-2xl border-divider bg-default-50/50 p-8 text-center">
            <MapPin className="h-10 w-10 text-primary" aria-hidden="true" />
            <h2 className="text-base font-semibold">还没有可标注的视频</h2>
            <p className="text-sm text-default-500">先从视频库导入视频，再进入工作台添加标记。</p>
            <Button onPress={() => useAppStore.getState().setPage('library')}>前往视频库</Button>
          </Card>
        </main>
      )}
    </div>
  );
}
