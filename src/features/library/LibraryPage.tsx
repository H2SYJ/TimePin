import { getCurrentWebview } from '@tauri-apps/api/webview';
import { Film, FolderPlus, LoaderCircle, UploadCloud } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Button, Card } from '@/components/ui';
import { VideoCard } from '@/features/library/VideoCard';
import { VideoDetail } from '@/features/library/VideoDetail';
import { api, isTauri } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { useAppStore } from '@/stores/app';

const SUPPORTED_EXTENSIONS = new Set([
  'mp4', 'mkv', 'mov', 'avi', 'flv', 'webm', 'm4v', 'ts', 'mts', 'm2ts',
]);

function isSupportedVideoPath(path: string): boolean {
  const ext = path.split(/[\\/]/).pop()?.split('.').pop()?.toLowerCase() ?? '';
  return SUPPORTED_EXTENSIONS.has(ext);
}

function filterSupportedPaths(paths: string[]): string[] {
  return paths.filter(isSupportedVideoPath);
}

export function LibraryPage() {
  const videos = useAppStore((state) => state.videos);
  const loading = useAppStore((state) => state.loading);
  const importing = useAppStore((state) => state.importing);
  const importPaths = useAppStore((state) => state.importPaths);
  const [dragging, setDragging] = useState(false);

  const pickFiles = useCallback(async () => {
    try {
      const paths = await api.pickVideoFiles();
      await importPaths(paths);
    } catch (error) {
      useAppStore.setState({ error: String(error) });
    }
  }, [importPaths]);

  // 使用 Tauri 2 的 Webview drag-drop 事件，正确获取文件路径
  useEffect(() => {
    if (!isTauri()) return;

    let unlisten: (() => void) | null = null;

    (async () => {
      const webview = getCurrentWebview();
      unlisten = await webview.onDragDropEvent((event) => {
        const { type } = event.payload;

        if (type === 'enter' || type === 'drop') {
          const { paths: droppedPaths } = event.payload;
          const supported = filterSupportedPaths(droppedPaths).length > 0;
          if (type === 'enter') {
            setDragging(supported);
          }
        }

        if (type === 'drop') {
          setDragging(false);
          const { paths: droppedPaths } = event.payload;
          const supportedPaths = filterSupportedPaths(droppedPaths);
          if (supportedPaths.length > 0) {
            void importPaths(supportedPaths);
          } else {
            useAppStore.setState({ error: '请拖入受支持的视频文件，或使用「选择视频」按钮' });
          }
        } else if (type === 'leave') {
          setDragging(false);
        }
      });
    })();

    return () => {
      if (unlisten) unlisten();
    };
  }, [importPaths]);

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-4 border-b border-border px-8 py-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">媒体库</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            导入本地视频，查看元数据并预览。原视频始终保持不变。
          </p>
        </div>
        <Button onClick={() => void pickFiles()} disabled={importing}>
          {importing ? (
            <LoaderCircle className="h-4 w-4 animate-spin" aria-hidden="true" />
          ) : (
            <FolderPlus className="h-4 w-4" aria-hidden="true" />
          )}
          {importing ? '正在导入…' : '选择视频'}
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <main className="scrollbar-thin min-w-0 flex-1 overflow-y-auto p-8">
          <div
            className={cn(
              'mb-6 flex min-h-32 flex-col items-center justify-center gap-3 rounded-xl border-2 border-dashed p-6 text-center transition-colors',
              dragging ? 'border-primary bg-accent/60' : 'border-border bg-secondary/30',
            )}
          >
            <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-background text-primary">
              <UploadCloud className="h-5 w-5" aria-hidden="true" />
            </div>
            <div>
              <p className="text-sm font-medium">将视频拖放到这里</p>
              <p className="mt-1 text-xs text-muted-foreground">
                支持 MP4、MKV、MOV、AVI 等格式；也可使用右上角按钮选择多个文件
              </p>
            </div>
          </div>

          {loading ? (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-muted-foreground">
              <LoaderCircle className="h-4 w-4 animate-spin" /> 正在加载媒体库…
            </div>
          ) : videos.length === 0 ? (
            <Card className="flex flex-col items-center justify-center gap-3 py-16 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-xl bg-secondary text-muted-foreground">
                <Film className="h-6 w-6" />
              </div>
              <div>
                <p className="text-sm font-medium">媒体库还是空的</p>
                <p className="mt-1 text-xs text-muted-foreground">导入一个视频，开始整理你的时间轴</p>
              </div>
            </Card>
          ) : (
            <>
              <div className="mb-4 flex items-center justify-between">
                <h2 className="text-sm font-medium">全部视频</h2>
                <span className="text-xs text-muted-foreground">{videos.length} 项</span>
              </div>
              <div className="grid grid-cols-1 gap-4 md:grid-cols-2 xl:grid-cols-3">
                {videos.map((video) => <VideoCard key={video.id} video={video} />)}
              </div>
            </>
          )}
        </main>

        <aside className="scrollbar-thin w-[360px] shrink-0 overflow-y-auto border-l border-border bg-secondary/20 p-5">
          <VideoDetail />
        </aside>
      </div>
    </div>
  );
}