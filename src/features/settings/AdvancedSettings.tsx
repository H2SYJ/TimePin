import { Copy, Download, FolderOpen, Plus, RefreshCw, RotateCcw, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button, Card, Input } from '@/components/ui';
import { api, isTauri } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { AdvancedSettingsInfo, DependencyReport, Settings } from '@/types';
import type { SettingsErrors } from './settingsValidation';
import type { useDependencyInstaller } from './useDependencyInstaller';

type Tool = 'ffmpeg' | 'ffprobe';
const stages = { download: '下载', extract: '解压', validate: '验证', completed: '完成', failed: '失败' };

interface Props {
  draft: Settings;
  patch: <K extends keyof Settings>(key: K, value: Settings[K]) => void;
  errors: SettingsErrors;
  saving: boolean;
  installer: ReturnType<typeof useDependencyInstaller>;
}

export function AdvancedSettings({ draft, patch, errors, saving, installer }: Props) {
  const desktop = isTauri();
  const savedDirectories = useAppStore((state) => state.settings.customPluginDirs);
  const [info, setInfo] = useState<AdvancedSettingsInfo | null>(null);
  const [infoError, setInfoError] = useState<string | null>(null);
  const [report, setReport] = useState<DependencyReport | null>(null);
  const [checking, setChecking] = useState(false);
  const [operationError, setOperationError] = useState<string | null>(null);
  const [picking, setPicking] = useState(false);
  const [copyNotice, setCopyNotice] = useState<string | null>(null);
  const mounted = useRef(false);
  const checkId = useRef(0);
  const busy = saving || installer.installing || picking;

  const check = async (settings: Settings) => {
    const id = ++checkId.current;
    setChecking(true);
    setOperationError(null);
    try {
      const result = await api.validateDependencies(settings);
      if (mounted.current && id === checkId.current) setReport(result);
    } catch (cause) {
      if (mounted.current && id === checkId.current) setOperationError(String(cause));
    } finally {
      if (mounted.current && id === checkId.current) setChecking(false);
    }
  };

  // 路径变化后旧检测结果不再代表当前草稿，也不能让迟到的结果覆盖新路径。
  useEffect(() => {
    checkId.current += 1;
    setChecking(false);
    setReport(null);
  }, [draft.ffmpegPath, draft.ffprobePath]);

  useEffect(() => {
    mounted.current = true;
    if (desktop) {
      void api.getAdvancedSettingsInfo().then((result) => {
        if (mounted.current) setInfo(result);
      }).catch((cause: unknown) => {
        if (mounted.current) setInfoError(String(cause));
      });
      void check(draft);
    }
    return () => { mounted.current = false; checkId.current += 1; };
    // 初次进入时检查已保存设置；后续编辑仅在用户点击检测时运行。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [desktop]);

  useEffect(() => {
    const installed = installer.installedResult;
    if (!installed) return;
    // 优先用后端返回的报告：安装刚完成时 draft 可能尚未反映合并的路径，
    // 旧草稿会让检测结果停在「待检测」且不再重试。
    if (installed.report && installed.report.dependencies.length > 0) {
      setReport(installed.report);
    } else {
      void check({ ...draft, ffmpegPath: installed.ffmpegPath, ffprobePath: installed.ffprobePath });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installer.installedResult]);

  const perform = async (action: () => Promise<void>) => {
    setOperationError(null);
    try { await action(); } catch (cause) {
      if (mounted.current) setOperationError(String(cause));
    }
  };

  const browseDirectory = async (index?: number) => {
    setPicking(true);
    await perform(async () => {
      const path = await api.pickDirectory();
      if (!path || !mounted.current) return;
      if (index === undefined) patch('customPluginDirs', [...draft.customPluginDirs, path]);
      else patch('customPluginDirs', draft.customPluginDirs.map((value, i) => i === index ? path : value));
    });
    if (mounted.current) setPicking(false);
  };

  const browseTool = async (tool: Tool) => {
    setPicking(true);
    await perform(async () => {
      const path = await api.pickExecutable(tool);
      if (path && mounted.current) patch(tool === 'ffmpeg' ? 'ffmpegPath' : 'ffprobePath', path);
    });
    if (mounted.current) setPicking(false);
  };

  const copyError = async () => {
    if (!installer.error) return;
    try {
      await navigator.clipboard.writeText(installer.error);
      setCopyNotice('错误详情已复制');
    } catch {
      setCopyNotice('复制失败，请手动选择下方错误详情。');
    }
  };

  return (
    <Card className="rounded-2xl border-divider bg-default-50/50 p-6" aria-labelledby="advanced-settings-title">
      <div className="mb-5">
        <h2 id="advanced-settings-title" className="text-base font-semibold">高级设置</h2>
        <p className="mt-1 text-sm text-default-500">管理插件搜索目录、媒体依赖和本机安装。保存后立即生效，无需重启。</p>
      </div>
      {!desktop && <p role="status" className="mb-5 rounded-lg bg-default-100 p-3 text-sm text-default-600">当前为浏览器预览。目录选择、依赖检测、安装和保存设置仅在 TimePin 桌面应用中可用。</p>}
      {infoError && <p role="alert" className="mb-4 text-sm text-danger">无法读取应用目录：{infoError}</p>}
      <div className="space-y-6">
        <section aria-labelledby="plugin-directories-title" className="space-y-4">
          <h3 id="plugin-directories-title" className="text-sm font-semibold">插件目录</h3>
          <div className="flex flex-col gap-2 sm:flex-row sm:items-end">
            <Input label="默认用户插件目录（始终搜索）" value={info?.defaultPluginDirectory ?? ''} placeholder={desktop ? '正在读取应用目录…' : '请在桌面应用中查看'} isReadOnly />
            <Button variant="secondary" isDisabled={!desktop || !info || busy} onPress={() => void perform(() => api.openPluginDirectory(null))}><FolderOpen className="h-4 w-4" aria-hidden="true" />打开默认目录</Button>
          </div>
          <p className="text-xs text-default-500">内置插件和默认用户目录始终参与搜索。以下每一项都是额外的插件搜索根目录，必须是已存在的绝对目录。</p>
          {draft.customPluginDirs.length === 0 && <p className="text-sm text-default-500">未配置额外搜索目录，使用默认目录。</p>}
          <div className="space-y-3">
            {draft.customPluginDirs.map((directory, index) => {
              const saved = directory === savedDirectories[index];
              return (
                <div key={index} className="flex flex-col gap-2 rounded-lg border border-divider p-3 sm:flex-row sm:items-start">
                  <Input id={`plugin-${index}`} label={`插件搜索目录 ${index + 1}`} value={directory} placeholder="绝对目录路径" isDisabled={busy} isInvalid={Boolean(errors[`plugin-${index}`])} errorMessage={errors[`plugin-${index}`]} onValueChange={(value) => patch('customPluginDirs', draft.customPluginDirs.map((path, i) => i === index ? value : path))} />
                  <div className="flex gap-2">
                    <Button variant="secondary" isDisabled={!desktop || busy} aria-label={`浏览插件目录 ${index + 1}`} onPress={() => void browseDirectory(index)}>浏览</Button>
                    <Button variant="secondary" isDisabled={!desktop || busy || !saved} aria-label={`打开插件目录 ${index + 1}`} title={saved ? '打开已保存的目录' : '请先保存该目录'} onPress={() => void perform(() => api.openPluginDirectory(index))}><FolderOpen className="h-4 w-4" aria-hidden="true" />打开</Button>
                    <Button variant="danger" size="icon" isDisabled={busy} aria-label={`移除插件目录 ${index + 1}`} onPress={() => patch('customPluginDirs', draft.customPluginDirs.filter((_, i) => i !== index))}><Trash2 className="h-4 w-4" aria-hidden="true" /></Button>
                  </div>
                </div>
              );
            })}
          </div>
          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" isDisabled={!desktop || busy} onPress={() => void browseDirectory()}><FolderOpen className="h-4 w-4" aria-hidden="true" />浏览并添加目录</Button>
            <Button variant="ghost" isDisabled={busy} onPress={() => patch('customPluginDirs', [...draft.customPluginDirs, ''])}><Plus className="h-4 w-4" aria-hidden="true" />手动添加目录</Button>
            <Button variant="ghost" isDisabled={busy || draft.customPluginDirs.length === 0} onPress={() => patch('customPluginDirs', [])}><RotateCcw className="h-4 w-4" aria-hidden="true" />恢复默认插件目录</Button>
          </div>
          <p className="rounded-lg bg-default-100 p-3 text-sm text-default-600">更改目录不会移动或删除旧插件。需要迁移时，请手动复制插件到目标目录，再保存设置；恢复默认仅清空额外目录列表。</p>
        </section>

        <section aria-labelledby="media-dependencies-title" className="space-y-4 border-t border-divider pt-5">
          <div>
            <h3 id="media-dependencies-title" className="text-sm font-semibold">FFmpeg / FFprobe 依赖</h3>
            <p className="mt-1 text-xs text-default-500">两项均需版本 ≥ 6.0。留空时自动查找；明确配置的路径无效时不会静默回退。</p>
          </div>
          {(['ffmpeg', 'ffprobe'] as const).map((tool) => {
            const key = tool === 'ffmpeg' ? 'ffmpegPath' : 'ffprobePath';
            const status = report?.dependencies.find((dependency) => dependency.id === tool);
            const invalid = Boolean(draft[key] && status && !status.available);
            const name = tool === 'ffmpeg' ? 'FFmpeg' : 'FFprobe';
            return (
              <div key={tool} className="flex flex-col gap-2 sm:flex-row sm:items-start">
                <Input label={`${name} 路径`} value={draft[key] ?? ''} placeholder={tool === 'ffmpeg' ? '可执行文件或 bin 目录；留空使用 PATH' : '优先使用此文件；否则查找 FFmpeg 同目录，再查 PATH'} description={tool === 'ffmpeg' ? '支持可执行文件路径或包含工具的 bin 目录。' : '显式 FFprobe 路径优先于 FFmpeg 同目录和系统 PATH。'} isDisabled={busy || checking} isInvalid={invalid} errorMessage={invalid ? status?.error ?? '该工具不可用，请检查路径或版本' : undefined} onValueChange={(value) => patch(key, value || null)} />
                <Button variant="secondary" isDisabled={!desktop || busy || checking} aria-label={`浏览 ${name}`} onPress={() => void browseTool(tool)}>浏览文件</Button>
              </div>
            );
          })}
          <div className="flex flex-wrap gap-2">
            <Button variant="secondary" isLoading={checking} isDisabled={!desktop || busy || checking} onPress={() => void check(draft)}><RefreshCw className="h-4 w-4" aria-hidden="true" />{checking ? '检测中…' : '检测依赖'}</Button>
            <Button isLoading={installer.installing} isDisabled={!desktop || busy || checking} onPress={() => { setCopyNotice(null); void installer.install(); }}><Download className="h-4 w-4" aria-hidden="true" />{installer.installing ? '正在安装…' : installer.error ? '重试安装依赖' : '一键安装依赖'}</Button>
          </div>
          <ul aria-label="依赖状态" className="space-y-3 rounded-lg border border-divider p-4" aria-live="polite" aria-busy={checking}>
            {(['ffmpeg', 'ffprobe'] as const).map((tool) => {
              const status = report?.dependencies.find((dependency) => dependency.id === tool);
              return <li key={tool} className="text-sm">
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1"><span className="font-medium">{tool}</span><span className={status?.available ? 'text-success' : 'text-default-500'}>{checking ? '检测中' : status ? status.available ? '可用' : '不可用' : '待检测'}</span><span className="text-xs text-default-500">要求 ≥ {status?.minimumVersion ?? '6.0'}{status?.version ? ` · 当前 ${status.version}` : ''}</span></div>
                {status?.path && <p className="mt-1 break-all text-xs text-default-500">{status.path}</p>}
                {status?.error && <p className="mt-1 break-words text-xs text-danger">{status.error}</p>}
              </li>;
            })}
          </ul>
          <p className="text-xs text-default-500">安装固定版本 b6.1.1（FFmpeg 6.1.1），下载经 SHA-256 校验。写入软件安装目录下的 dependencies 子目录，不修改系统 PATH；若安装目录受管理员权限保护，请以管理员身份运行应用或使用默认 PATH 中的 FFmpeg。成功后自动保存两个工具路径，其它未保存设置保持不变。</p>
          <p className="break-all text-xs text-default-500">依赖安装目录：{info?.installDirectory ?? (desktop ? '正在读取…' : '仅桌面应用可用')}</p>
          {installer.progress && <div role="status" className="space-y-2 rounded-lg bg-default-100 p-3 text-sm" aria-live="polite">
            <p>{stages[installer.progress.stage]} · {Math.round(installer.progress.progress * 100)}% — {installer.progress.message}</p>
            <progress className="h-2 w-full accent-primary" aria-label="依赖安装进度" max={1} value={installer.progress.progress} />
          </div>}
          {installer.error && <div role="alert" className="space-y-2 rounded-lg border border-danger/30 bg-danger/5 p-3 text-sm">
            <p className="font-medium text-danger">依赖安装失败，可重试；原有工具配置未更改。</p>
            <pre className="whitespace-pre-wrap break-words font-sans text-danger" tabIndex={0}>{installer.error}</pre>
            <Button variant="secondary" size="sm" onPress={() => void copyError()}><Copy className="h-4 w-4" aria-hidden="true" />复制错误详情</Button>
          </div>}
          {copyNotice && <p role="status" className="text-xs text-default-500">{copyNotice}</p>}
          {operationError && <p role="alert" className="break-words text-sm text-danger">操作失败：{operationError}</p>}
        </section>
      </div>
    </Card>
  );
}
