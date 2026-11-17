import { useCallback, useEffect, useRef, useState } from 'react';
import { api } from '@/lib/ipc';
import { useAppStore } from '@/stores/app';
import type { DependencyInstallProgress, InstalledDependencies } from '@/types';

interface Attempt {
  taskId: string;
  unlisten?: () => void;
}

function release(attempt: Attempt) {
  attempt.unlisten?.();
  attempt.unlisten = undefined;
}

/** 先注册监听再调用安装；离开页面时解绑，迟到的注册也必须立即回收。 */
export function useDependencyInstaller(onInstalled: (result: InstalledDependencies) => void) {
  const [installing, setInstalling] = useState(false);
  const [progress, setProgress] = useState<DependencyInstallProgress | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [installedResult, setInstalledResult] = useState<InstalledDependencies | null>(null);
  const mounted = useRef(false);
  const active = useRef<Attempt | null>(null);
  const onInstalledRef = useRef(onInstalled);
  onInstalledRef.current = onInstalled;

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      if (active.current) release(active.current);
    };
  }, []);

  const install = useCallback(async () => {
    if (active.current) return;
    const taskId = globalThis.crypto?.randomUUID?.() ?? `dependencies-${Date.now()}-${Math.random().toString(36).slice(2)}`;
    const attempt: Attempt = { taskId };
    active.current = attempt;
    setInstalling(true);
    setError(null);
    setProgress(null);
    setInstalledResult(null);
    try {
      attempt.unlisten = await api.onDependencyInstallProgress((event) => {
        if (!mounted.current || active.current !== attempt || event.taskId !== taskId) return;
        setProgress({ ...event, progress: Math.max(0, Math.min(1, event.progress)) });
        if (event.stage === 'failed') setError(event.error ?? event.message);
      });
      if (!mounted.current) return;
      const result = await api.installDependencies(taskId);
      // 即使安装期间离开页面，后端已保存的工具路径仍需同步到全局设置。
      useAppStore.getState().mergeInstalledToolPaths(result);
      if (mounted.current) {
        setProgress({ schemaVersion: '1.0.0', taskId, stage: 'completed', progress: 1, message: '依赖已安装、验证并保存路径', error: null });
        setError(null);
        setInstalledResult(result);
        onInstalledRef.current(result);
      }
    } catch (cause) {
      if (mounted.current) {
        const message = cause instanceof Error ? cause.message : String(cause);
        setError(message);
        setProgress((current) => ({ schemaVersion: '1.0.0', taskId, stage: 'failed', progress: current?.progress ?? 0, message: '安装失败，原有工具配置保持不变', error: message }));
      }
    } finally {
      release(attempt);
      if (active.current === attempt) active.current = null;
      if (mounted.current) setInstalling(false);
    }
  }, []);

  return { installing, progress, error, installedResult, install };
}
