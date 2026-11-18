import { getCurrentWebview } from '@tauri-apps/api/webview';
import { useCallback, useEffect, useRef, useState } from 'react';
import { api, isTauri } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { PluginImportSource } from '@/types';

const MAX_PACKAGE_BYTES = 128 * 1024 * 1024;

function validatePackageName(name: string): void {
  if (!/\.(tppkg|zip)$/i.test(name)) {
    throw new Error('仅支持 .tppkg 或 .zip 插件包');
  }
}

function readPackage(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onerror = () => reject(new Error('读取插件包失败，请重新选择文件'));
    reader.onabort = () => reject(new Error('插件包读取已取消'));
    reader.onload = () => {
      const result = reader.result;
      if (typeof result !== 'string' || !result.includes(';base64,')) {
        reject(new Error('读取插件包失败：无法取得 base64 数据'));
        return;
      }
      resolve(result.slice(result.indexOf(',') + 1));
    };
    reader.readAsDataURL(file);
  });
}

/** 文件选择、原生路径和 HTML File 共用一把锁，覆盖选择框、读取、IPC 及列表刷新。 */
export function usePluginImport(active: boolean, refreshPlugins: () => Promise<boolean>) {
  const [busy, setBusy] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [feedback, setFeedback] = useState<{ kind: 'success' | 'error' | 'warning'; message: string } | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(false);
  const activeRef = useRef(active);
  activeRef.current = active;

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const importSource = useCallback(async (getSource: () => Promise<PluginImportSource | null>) => {
    if (!activeRef.current || inFlight.current || !mounted.current) return;
    // Store 仅在保存成功后更新；不读取设置页尚未保存的 draft，也不回退到内置目录。
    if (!useAppStore.getState().settings.customPluginDirs?.[0]?.trim()) {
      setFeedback({ kind: 'error', message: '请先前往设置，添加并保存第一个自定义插件目录，再导入插件。' });
      return;
    }
    inFlight.current = true;
    setBusy(true);
    setFeedback(null);
    try {
      const source = await getSource();
      // 选择框或 FileReader 返回时可能已经离开本地 Tab，不再发起安装。
      if (source === null || !mounted.current || !activeRef.current) return;
      validatePackageName(typeof source === 'string' ? source : source.fileName);
      const result = await api.importPlugin(source);
      if (!mounted.current) return;
      const message = `已${result.replaced ? '替换同名插件' : '导入插件'} ${result.name} v${result.version}。`;
      // 后端已安装并 reload；列表读取失败不应被描述为安装失败，也不应重复 reload。
      try {
        const refreshed = await refreshPlugins();
        if (mounted.current) {
          setFeedback(refreshed
            ? { kind: 'success', message }
            : { kind: 'warning', message: `${message}但列表刷新失败，请点击「重新加载」重试。` });
        }
      } catch {
        if (mounted.current) setFeedback({ kind: 'warning', message: `${message}但列表刷新失败，请点击「重新加载」重试。` });
      }
    } catch (error) {
      if (mounted.current) setFeedback({ kind: 'error', message: `导入失败：${error instanceof Error ? error.message : String(error)}` });
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(false);
    }
  }, [refreshPlugins]);

  const pickPackage = useCallback(() => importSource(() => api.pickPluginPackage()), [importSource]);

  const importPaths = useCallback((paths: string[]) => importSource(async () => {
    if (paths.length !== 1) throw new Error('一次只能导入一个插件包，请选择一个文件');
    return paths[0] ?? null;
  }), [importSource]);

  const importFiles = useCallback((files: File[]) => importSource(async () => {
    if (files.length !== 1) throw new Error('一次只能导入一个插件包，请选择一个文件');
    const file = files[0];
    if (!file) return null;
    validatePackageName(file.name);
    // 在 FileReader / base64 分配之前拦截超大压缩包。
    if (file.size > MAX_PACKAGE_BYTES) throw new Error('插件包不能超过 128 MiB');
    return { base64: await readPackage(file), fileName: file.name };
  }), [importSource]);

  useEffect(() => {
    setDragging(false);
    if (!active || !isTauri()) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    // 注册是异步的：离开 Tab / 卸载后才拿到的清理函数也必须立即执行。
    void (async () => {
      try {
        const cleanup = await getCurrentWebview().onDragDropEvent(({ payload }) => {
          if (disposed || !activeRef.current) return;
          if (payload.type === 'enter') {
            setDragging(!inFlight.current);
          } else if (payload.type === 'leave') {
            setDragging(false);
          } else if (payload.type === 'drop') {
            setDragging(false);
            void importPaths(payload.paths);
          }
        });
        if (disposed) cleanup();
        else unlisten = cleanup;
      } catch {
        if (!disposed) setFeedback({ kind: 'warning', message: '原生拖放暂不可用，请使用「导入插件」按钮。' });
      }
    })();
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [active, importPaths]);

  return { busy, dragging, setDragging, feedback, pickPackage, importFiles };
}
