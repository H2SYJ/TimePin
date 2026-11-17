import { Check, Clock, Copy, FileVideo, Film, HardDrive, Monitor } from 'lucide-react';
import { useState } from 'react';
import { Badge, Button, Card, CardBody, CardFooter, CardHeader } from '@/components/ui';
import { fileSrc } from '@/lib/ipc';
import { formatDate, formatDuration, formatFileSize } from '@/lib/utils';
import { useAppStore } from '@/stores/app';

function MetaRow({ icon: Icon, label, value }: {
  icon: typeof Clock;
  label: string;
  value: string;
}) {
  return (
    <div className="flex items-center justify-between gap-4 border-b border-divider py-3 last:border-b-0">
      <span className="flex items-center gap-2 text-sm text-default-500">
        <Icon className="h-4 w-4" aria-hidden="true" />
        {label}
      </span>
      <span className="truncate text-right text-sm font-medium tabular-nums" title={value}>
        {value}
      </span>
    </div>
  );
}

export function VideoDetail() {
  const videos = useAppStore((state) => state.videos);
  const selectedVideoId = useAppStore((state) => state.selectedVideoId);
  const [copied, setCopied] = useState(false);
  const video = videos.find((item) => item.id === selectedVideoId) ?? null;

  if (!video) {
    return (
      <Card className="flex min-h-[280px] flex-col items-center justify-center gap-4 rounded-2xl border-divider bg-default-50/50 p-8 text-center">
        <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/15 text-primary">
          <Film className="h-7 w-7" />
        </div>
        <div>
          <p className="text-sm font-medium">尚未选择视频</p>
          <p className="mt-1 text-xs text-default-400">从媒体库中选择一项查看详情与预览</p>
        </div>
      </Card>
    );
  }

  const videoSrc = fileSrc(video.path);
  const sourcePath = video.sourcePath || video.path;

  return (
    <Card className="overflow-hidden rounded-2xl border-divider bg-default-50/50">
      <CardHeader className="p-4 pb-0">
        <div className="flex w-full items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-[11px] font-medium uppercase tracking-wider text-default-400">视频详情</p>
            <h2 className="mt-1 truncate text-base font-semibold" title={video.name}>{video.name}</h2>
          </div>
          <Badge className="shrink-0 border border-primary/30 bg-primary/10 text-primary">已导入</Badge>
        </div>
      </CardHeader>

      <CardBody className="p-4 pt-3">
        <div className="aspect-video w-full overflow-hidden rounded-xl bg-black">
          <video
            key={video.id}
            controls
            preload="metadata"
            src={videoSrc}
            className="h-full w-full"
            aria-label={`${video.name} 预览`}
          >
            您的运行环境不支持视频预览。
          </video>
        </div>
      </CardBody>

      <CardBody className="px-4 pb-2 pt-0">
        <MetaRow icon={Clock} label="时长" value={formatDuration(video.duration)} />
        <MetaRow
          icon={Monitor}
          label="分辨率"
          value={video.width > 0 && video.height > 0 ? `${video.width} × ${video.height}` : '未知'}
        />
        <MetaRow
          icon={Film}
          label="帧率 / 编码"
          value={`${video.fps > 0 ? `${video.fps.toFixed(2)} fps` : '-- fps'} · ${video.codec || 'unknown'}`}
        />
        <MetaRow icon={HardDrive} label="文件大小" value={formatFileSize(video.fileSize)} />
        <MetaRow icon={FileVideo} label="导入时间" value={formatDate(video.importedAt)} />
      </CardBody>

      <CardFooter className="border-t border-divider/60 bg-background/40 p-4">
        <div className="w-full">
          <Button className="mb-4 w-full" onPress={() => useAppStore.getState().setPage('marks')}>
            进入标注工作台
          </Button>
          <p className="mb-2 text-[11px] font-medium uppercase tracking-wider text-default-400">源文件路径</p>
          <div className="flex items-center gap-2">
            <code
              className="min-w-0 flex-1 truncate rounded-lg border border-divider bg-background px-2.5 py-1.5 font-mono text-xs"
              title={sourcePath}
            >
              {sourcePath}
            </code>
            <Button
              size="sm"
              variant="secondary"
              title="复制源文件路径"
              onPress={() => {
                if (!navigator.clipboard) return;
                void navigator.clipboard
                  .writeText(sourcePath)
                  .then(() => {
                    setCopied(true);
                    window.setTimeout(() => setCopied(false), 1500);
                  })
                  .catch(() => setCopied(false));
              }}
            >
              {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
              {copied ? '已复制' : '复制'}
            </Button>
          </div>
        </div>
      </CardFooter>
    </Card>
  );
}
