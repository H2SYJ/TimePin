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
      <header className="flex items-center justify-between gap-4 px-8 py-6">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-semibold tracking-tight">视频库</h1>
            <span className="rounded-full border border-divider bg-default-50 px-2.5 py-0.5 text-xs font-medium text-default-500">
              {videos.length} 个视频
            </span>
          </div>
          <p className="mt-1.5 text-sm text-default-500">
            导入本地视频，查看元数据并预览。原视频始终保持不变。
          </p>
        </div>
        <Button size="md" onPress={() => void pickFiles()} isDisabled={importing}>
          {importing ? (
            <LoaderCircle className="h-4 w-4 animate-spin" aria-hidden="true" />
          ) : (
            <FolderPlus className="h-4 w-4" aria-hidden="true" />
          )}
          {importing ? '正在导入…' : '导入视频'}
        </Button>
      </header>

      <div className="flex min-h-0 flex-1">
        <main className="scrollbar-thin min-w-0 flex-1 overflow-y-auto px-8 pb-8">
          <div
            className={cn(
              'mb-6 flex min-h-36 flex-col items-center justify-center gap-3 rounded-2xl border-2 border-dashed p-6 text-center transition-colors',
              dragging
                ? 'border-primary bg-primary/10'
                : 'border-divider/60 bg-default-50/50',
            )}
          >
            <div className="flex h-11 w-11 items-center justify-center rounded-xl bg-primary/15 text-primary">
              <UploadCloud className="h-5 w-5" aria-hidden="true" />
            </div>
            <div>
              <p className="text-sm font-medium">将视频拖放到这里</p>
              <p className="mt-1 text-xs text-default-400">
                支持 MP4、MKV、MOV、AVI 等格式；也可使用右上角按钮选择多个文件
              </p>
            </div>
          </div>

          {loading ? (
            <div className="flex items-center justify-center gap-2 py-16 text-sm text-default-400">
              <LoaderCircle className="h-4 w-4 animate-spin" /> 正在加载媒体库…
            </div>
          ) : videos.length === 0 ? (
            <Card className="flex flex-col items-center justify-center gap-3 rounded-2xl border-divider bg-default-50/50 py-16 text-center">
              <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/15 text-primary">
                <Film className="h-7 w-7" />
              </div>
              <div>
                <p className="text-sm font-medium">媒体库还是空的</p>
                <p className="mt-1 text-xs text-default-400">导入一个视频，开始整理你的时间轴</p>
              </div>
            </Card>
          ) : (
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-3 2xl:grid-cols-4">
              {videos.map((video) => <VideoCard key={video.id} video={video} />)}
            </div>
          )}
        </main>

        <aside className="scrollbar-thin hidden w-[340px] shrink-0 overflow-y-auto border-l border-divider bg-default-50/50 p-6 lg:block xl:w-[380px] 2xl:w-[420px]">
          <VideoDetail />
        </aside>
      </div>
    </div>
  );
}