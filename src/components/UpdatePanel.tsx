import { AlertTriangle, CheckCircle2, Info, Loader2, RefreshCw } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';
import { Button, Card, Modal, ModalBody, ModalContent, ModalFooter, ModalHeader, Progress } from '@/components/ui';
import { formatFileSize } from '@/lib/utils';
import { isWindows, useUpdateStore } from '@/stores/updater';
import { useAppStore } from '@/stores/app';

const BUSY_LABELS: Record<string, string> = {
  import: '视频导入',
  export: '片段导出',
  plugin: '插件任务',
  recognition: '识别任务',
};

function statusTone(phase: string) {
  if (phase === 'error') return 'border-danger/30 bg-danger-50 text-danger';
  if (phase === 'notConfigured') return 'border-warning/30 bg-warning-50 text-warning';
  if (phase === 'upToDate' || phase === 'installedNeedsRelaunch') return 'border-success/30 bg-success-50 text-success';
  return 'border-divider bg-background text-default-500';
}

export function UpdatePanel() {
  const phase = useUpdateStore((state) => state.phase);
  const currentVersion = useUpdateStore((state) => state.currentVersion);
  const available = useUpdateStore((state) => state.available);
  const progress = useUpdateStore((state) => state.progress);
  const error = useUpdateStore((state) => state.error);
  const busyWorkKinds = useUpdateStore((state) => state.busyWorkKinds);
  const checkForUpdate = useUpdateStore((state) => state.checkForUpdate);
  const requestInstallConfirmation = useUpdateStore((state) => state.requestInstallConfirmation);
  const cancelInstallConfirmation = useUpdateStore((state) => state.cancelInstallConfirmation);
  const downloadAndInstall = useUpdateStore((state) => state.downloadAndInstall);
  const restartNow = useUpdateStore((state) => state.restartNow);
  const resetToAvailable = useUpdateStore((state) => state.resetToAvailable);
  const clearError = useUpdateStore((state) => state.clearError);
  const importing = useAppStore((state) => state.importing);

  const [restarting, setRestarting] = useState(false);
  const statusRef = useRef<HTMLDivElement>(null);
  const confirmButtonRef = useRef<HTMLButtonElement>(null);

  const effectiveBusy = useMemo(() => {
    const kinds = new Set(busyWorkKinds);
    if (importing) kinds.add('import');
    return Array.from(kinds);
  }, [busyWorkKinds, importing]);

  const busyText = useMemo(
    () => effectiveBusy.map((kind) => BUSY_LABELS[kind] ?? kind).join('、'),
    [effectiveBusy],
  );

  const downloadedText = formatFileSize(progress.downloadedBytes);
  const totalText = progress.totalBytes == null ? '未知大小' : formatFileSize(progress.totalBytes);
  const percent = progress.totalBytes && progress.totalBytes > 0
    ? Math.round((progress.downloadedBytes / progress.totalBytes) * 100)
    : null;

  const busy = phase === 'checking' || phase === 'downloading' || phase === 'installing';
  const canConfirmInstall = phase === 'confirming' && effectiveBusy.length === 0;
  const windowsExitNotice = isWindows();

  useEffect(() => {
    if (phase === 'error' || phase === 'notConfigured' || phase === 'desktopOnly') {
      statusRef.current?.focus();
    }
  }, [phase, error]);

  useEffect(() => {
    if (phase === 'confirming') {
      confirmButtonRef.current?.focus();
    }
  }, [phase]);

  const statusMessage = useMemo(() => {
    if (phase === 'desktopOnly') return '应用内更新仅在 TimePin 桌面应用中可用。';
    if (phase === 'notConfigured') return '此构建未配置应用内更新。';
    if (phase === 'upToDate') return `当前已是最新版本 v${currentVersion || '未知'}。`;
    if (phase === 'available' && available) return `发现新版本 v${available.version}，当前版本 v${available.currentVersion}。`;
    if (phase === 'downloading') return `正在下载更新包：${downloadedText} / ${totalText}`;
    if (phase === 'installing') return '正在启动安装程序，请勿关闭窗口。';
    if (phase === 'installedNeedsRelaunch') {
      return windowsExitNotice
        ? '安装程序已启动，应用即将退出并完成安装。'
        : '更新已安装，重启后生效。';
    }
    if (phase === 'checking') return '正在检查更新…';
    return '尚未检查更新。';
  }, [phase, currentVersion, available, downloadedText, totalText, windowsExitNotice]);

  const showRelease = phase === 'available' || phase === 'confirming' || phase === 'downloading' || phase === 'installing' || phase === 'installedNeedsRelaunch';

  return (
    <Card className="rounded-2xl border-divider bg-default-50/50 p-6" aria-labelledby="update-title">
      <div className="mb-5 flex items-start justify-between gap-4">
        <div>
          <h2 id="update-title" className="text-base font-semibold">应用更新</h2>
          <p className="mt-1 text-sm text-default-500">
            仅在桌面应用中检查并安装官方更新包；不会在后台自动下载或重启。
          </p>
        </div>
        <Button
          variant="secondary"
          onPress={() => void checkForUpdate()}
          isDisabled={busy}
          aria-busy={phase === 'checking'}
        >
          {phase === 'checking' ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" /> : <RefreshCw className="h-4 w-4" aria-hidden="true" />}
          {phase === 'checking' ? '检查中…' : '检查更新'}
        </Button>
      </div>

      <div
        ref={statusRef}
        tabIndex={-1}
        role={phase === 'error' || phase === 'notConfigured' ? 'alert' : 'status'}
        aria-live={phase === 'error' || phase === 'notConfigured' ? 'assertive' : 'polite'}
        className={`rounded-xl border px-4 py-3 text-sm outline-none ${statusTone(phase)}`}
      >
        <div className="flex items-start gap-2">
          {phase === 'error' || phase === 'notConfigured' ? (
            <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          ) : phase === 'upToDate' || phase === 'installedNeedsRelaunch' ? (
            <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          ) : (
            <Info className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
          )}
          <div className="min-w-0 flex-1">
            <p className="font-medium text-foreground">{statusMessage}</p>
            {error && <p className="mt-1 text-sm">{error.message}</p>}
            {busyText && phase === 'available' && (
              <p className="mt-1 text-sm">检测到 {busyText} 正在进行，安装前请先保存并等待完成。</p>
            )}
          </div>
        </div>
      </div>

      {showRelease && available && (
        <div className="mt-4 space-y-3 rounded-xl border border-divider bg-background p-4">
          <div className="flex flex-wrap items-center gap-2 text-sm">
            <span className="font-medium text-foreground">v{available.version}</span>
            <span className="text-default-400">当前 v{available.currentVersion}</span>
            {available.date && <span className="text-default-400">发布于 {available.date}</span>}
          </div>
          {available.body && (
            <div className="rounded-lg bg-default-50/70 p-3 text-sm text-default-600">
              <p className="mb-1 text-xs font-medium uppercase tracking-wide text-default-400">更新说明</p>
              <p className="whitespace-pre-wrap">{available.body}</p>
            </div>
          )}
          {(phase === 'downloading' || phase === 'installing') && (
            <div className="space-y-1" aria-live="polite">
              <Progress value={percent} label="更新下载进度" />
              <p className="text-xs text-default-400">
                {percent == null ? `已下载 ${downloadedText}，总大小未知` : `已下载 ${downloadedText} / ${totalText}（${percent}%）`}
              </p>
            </div>
          )}
          {phase === 'installedNeedsRelaunch' && !windowsExitNotice && (
            <Button
              variant="primary"
              onPress={() => {
                setRestarting(true);
                void restartNow().catch(() => setRestarting(false));
              }}
              isDisabled={restarting}
            >
              {restarting ? '正在重启…' : '立即重启完成更新'}
            </Button>
          )}
          {phase === 'available' && (
            <div className="flex flex-wrap items-center gap-2">
              <Button onPress={requestInstallConfirmation}>下载并安装更新</Button>
              <Button variant="ghost" onPress={() => void resetToAvailable()}>稍后提醒</Button>
            </div>
          )}
        </div>
      )}

      {phase === 'error' && (
        <div className="mt-4 flex flex-wrap gap-2">
          <Button variant="secondary" onPress={() => void checkForUpdate()} isDisabled={busy}>
            重试检查
          </Button>
          <Button variant="ghost" onPress={clearError}>清除错误</Button>
        </div>
      )}

      <Modal
        isOpen={phase === 'confirming'}
        onOpenChange={(open) => {
          if (!open) void cancelInstallConfirmation();
        }}
        aria-labelledby="update-confirm-title"
      >
        <ModalContent>
          {() => (
            <>
              <ModalHeader id="update-confirm-title" className="flex flex-col gap-1">
                确认下载并安装更新
              </ModalHeader>
              <ModalBody>
                <p className="text-sm text-default-600">
                  将下载并安装版本 v{available?.version ?? '未知'}。安装不会修改媒体库、标注或伴随文件。
                </p>
                {busyText ? (
                  <div className="rounded-lg border border-warning/30 bg-warning-50 p-3 text-sm text-warning" role="alert">
                    当前有 {busyText} 正在运行。请先保存并等待完成，再确认更新。
                  </div>
                ) : (
                  <div className="rounded-lg border border-primary/25 bg-primary/[0.08] p-3 text-sm text-default-600">
                    {windowsExitNotice
                      ? '确认后将下载更新包并启动 Windows 安装程序；应用会随即退出，请在安装器完成后按提示重新打开。'
                      : '确认后将下载并安装更新；安装完成后可手动重启使新版本生效。'}
                  </div>
                )}
              </ModalBody>
              <ModalFooter>
                <Button variant="ghost" onPress={() => void cancelInstallConfirmation()}>
                  取消
                </Button>
                <Button
                  ref={confirmButtonRef}
                  variant="danger"
                  onPress={() => void downloadAndInstall()}
                  isDisabled={!canConfirmInstall}
                >
                  下载并安装
                </Button>
              </ModalFooter>
            </>
          )}
        </ModalContent>
      </Modal>
    </Card>
  );
}
