import { Film, ImageOff, Trash2 } from 'lucide-react';
import { useState } from 'react';
import { Badge, Button, Card, CardBody } from '@/components/ui';
import { fileSrc } from '@/lib/ipc';
import { cn, formatDuration, formatFileSize } from '@/lib/utils';
import { useAppStore } from '@/stores/app';
import type { VideoInfo } from '@/types';

function Thumbnail({ video }: { video: VideoInfo }) {
  const [failed, setFailed] = useState(false);
  const src = video.thumbnailPath && !failed ? fileSrc(video.thumbnailPath) : null;

  if (src) {
    return (
      <img
        src={src}
        alt={`${video.name} 缩略图`}
        onError={() => setFailed(true)}
        className="h-full w-full object-cover"
      />
    );
  }

  return (
    <div className="flex h-full w-full flex-col items-center justify-center gap-2 bg-default-100 text-default-400">
      {failed ? <ImageOff className="h-7 w-7" /> : <Film className="h-7 w-7" />}
      <span className="text-xs">暂无预览</span>
    </div>
  );
}

export function VideoCard({ video }: { video: VideoInfo }) {
  const selected = useAppStore((state) => state.selectedVideoId === video.id);
  const selectVideo = useAppStore((state) => state.selectVideo);
  const deleteVideo = useAppStore((state) => state.deleteVideo);
  const [confirmDelete, setConfirmDelete] = useState(false);

  return (
    <Card
      interactive
      isHoverable
      role="button"
      tabIndex={0}
      aria-pressed={selected}
      onClick={() => selectVideo(video.id)}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault();
          selectVideo(video.id);
        }
      }}
      className={cn(
        'group cursor-pointer overflow-hidden rounded-2xl',
        selected
          ? 'border-primary bg-primary/[0.06] ring-2 ring-primary/30'
          : 'border-divider bg-default-50/50 hover:border-default-300 hover:bg-default-100/60',
      )}
    >
      <div className="relative aspect-video overflow-hidden bg-default-100">
        <Thumbnail video={video} />
        <span className="absolute bottom-2 right-2 rounded-md bg-black/70 px-2 py-1 text-xs font-medium tabular-nums text-white backdrop-blur-sm">
          {formatDuration(video.duration)}
        </span>
      </div>

      <CardBody className="space-y-3 p-4">
        <div className="flex items-start justify-between gap-2">
          <h3 className="min-w-0 flex-1 truncate text-sm font-semibold" title={video.name}>
            {video.name}
          </h3>
          <Badge className="shrink-0 uppercase">{video.codec || 'UNKNOWN'}</Badge>
        </div>

        <div className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-default-400">
          <span>{video.width > 0 && video.height > 0 ? `${video.width} × ${video.height}` : '未知分辨率'}</span>
          <span>{Number.isFinite(video.fps) && video.fps > 0 ? `${video.fps.toFixed(2)} fps` : '-- fps'}</span>
          <span>{formatFileSize(video.fileSize)}</span>
        </div>

        <div className="flex items-center justify-between border-t border-divider/60 pt-3">
          <span
            className="truncate text-xs text-default-400"
            title={video.sourcePath || video.path}
          >
            {video.sourcePath || video.path}
          </span>
          {confirmDelete ? (
            <div className="flex shrink-0 items-center gap-1">
              <Button
                size="sm"
                variant="danger"
                onPress={() => {
                  void deleteVideo(video.id);
                  setConfirmDelete(false);
                }}
                onClick={(event) => event.stopPropagation()}
              >
                确认
              </Button>
              <Button
                size="sm"
                variant="ghost"
                onPress={() => setConfirmDelete(false)}
                onClick={(event) => event.stopPropagation()}
              >
                取消
              </Button>
            </div>
          ) : (
            <Button
              size="icon"
              variant="ghost"
              aria-label={`从媒体库移除 ${video.name}（不会删除原文件）`}
              title="从媒体库移除，不会删除原文件"
              className="opacity-0 transition-opacity group-hover:opacity-100 focus-visible:opacity-100"
              onPress={() => setConfirmDelete(true)}
              onClick={(event) => event.stopPropagation()}
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          )}
        </div>
      </CardBody>
    </Card>
  );
}
