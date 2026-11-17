import {
  Badge,
  Button,
  Card,
  CardBody,
  CardHeader,
  Checkbox,
  FormField,
  FormSection,
  Input,
  Progress,
  SelectButtonGroup,
  Switch,
  Tab,
  Tabs,
} from '@/components/ui';
import { useEffect, useMemo, useState } from 'react';
import type { PluginInfo, PluginParameterSchema, VideoInfo } from '@/types';
import { useAppStore } from '@/stores/app';
import { useUpdateStore } from '@/stores/updater';
import { api } from '@/lib/ipc';
import type { OrchestrationMode, OrchestrationResult } from '@/types';
import { Loader2, Play, Square, RotateCcw, Trash2, RefreshCw, AlertTriangle, CheckCircle, XCircle, Code, Zap, Github } from 'lucide-react';
import { cn } from '@/lib/utils';
import { GitHubPluginDiscovery } from '@/features/library/GitHubPluginDiscovery';

/** 插件运行参数：由 parametersSchema 的 default 初始化，UI 编辑后覆盖。 */
type PluginParamValues = Record<string, string | number | boolean | null>;

function defaultsForSchema(schema: Record<string, PluginParameterSchema>): PluginParamValues {
  const values: PluginParamValues = {};
  for (const [key, s] of Object.entries(schema)) {
    values[key] = s.default ?? null;
  }
  return values;
}

export function PluginsPage() {
  const { settings } = useAppStore();
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [runningTasks, setRunningTasks] = useState<Record<string, { progress: number; message: string }>>({});
  const [logs, setLogs] = useState<string[]>([]);
  // M4：多选插件 + 批量编排
  const [selectedPlugins, setSelectedPlugins] = useState<Set<string>>(new Set());
  const [orchestrationMode, setOrchestrationMode] = useState<OrchestrationMode>('sequential');
  const [orchestrationRunning, setOrchestrationRunning] = useState(false);
  const [orchestrationProgress, setOrchestrationProgress] = useState<{ completed: number; total: number; stage: string } | null>(null);
  const [orchestrationResult, setOrchestrationResult] = useState<OrchestrationResult | null>(null);
  // 每个插件的参数草稿：name → {param: value}
  const [paramValues, setParamValues] = useState<Record<string, PluginParamValues>>({});
  const [activeTab, setActiveTab] = useState<string>('local');

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void setupEventListeners()
      .then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          cleanup = unlisten;
        }
      })
      .catch((error) => {
        if (!disposed) console.error('注册插件事件监听失败:', error);
      });
    void loadPlugins();

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  const loadPlugins = async () => {
    setLoading(true);
    try {
      const list = await api.getPlugins();
      setPlugins(list);
      // 为每个有 schema 的插件初始化参数默认值（不覆盖用户已改动的草稿）
      setParamValues((prev) => {
        const next = { ...prev };
        for (const plugin of list) {
          const schema = plugin.parametersSchema;
          if (!next[plugin.name] && schema && typeof schema === 'object' && Object.keys(schema).length > 0) {
            next[plugin.name] = defaultsForSchema(schema);
          }
        }
        return next;
      });
    } catch (e) {
      console.error('加载插件列表失败:', e);
    } finally {
      setLoading(false);
    }
  };

  const setupEventListeners = async (): Promise<() => void> => {
    const unlisteners: Array<() => void> = [];
    try {
      const unlistenProgress = await api.onPluginProgress((e) => {
        setRunningTasks((prev) => ({
          ...prev,
          [e.taskId]: { progress: e.progress, message: e.message || '' },
        }));
      });
      unlisteners.push(unlistenProgress);

      const unlistenStatus = await api.onPluginStatus((e) => {
        setPlugins((prev) =>
          prev.map((p) =>
            p.name === e.pluginName
              ? { ...p, status: e.status as PluginInfo['status'], statusDetail: e.detail || null }
              : p
          )
        );
      });
      unlisteners.push(unlistenStatus);

      const unlistenLog = await api.onPluginLog((e) => {
        setLogs((prev) => [...prev.slice(-99), `[${e.level.toUpperCase()}] ${e.pluginName}: ${e.message}`]);
      });
      unlisteners.push(unlistenLog);

      const unlistenResult = await api.onPluginResult((e) => {
        setRunningTasks((prev) => {
          const next = { ...prev };
          delete next[e.taskId];
          return next;
        });
        if (e.outcome === 'ok') {
          setLogs((prev) => [...prev.slice(-99), `[OK] ${e.pluginName}: 生成 ${e.marksCount} 个标注`]);
        } else if (e.outcome === 'crashed') {
          setLogs((prev) => [...prev.slice(-99), `[CRASHED] ${e.pluginName}: ${e.error}`]);
        } else if (e.outcome === 'error') {
          setLogs((prev) => [...prev.slice(-99), `[ERROR] ${e.pluginName}: ${e.error}`]);
        }
      });
      unlisteners.push(unlistenResult);

      const unlistenOrchestration = await api.onPluginOrchestration((e) => {
        setOrchestrationProgress({ completed: e.completed, total: e.total, stage: e.stage });
        if (e.stage === 'finished') {
          setOrchestrationRunning(false);
        }
      });
      unlisteners.push(unlistenOrchestration);

      return () => unlisteners.forEach((unlisten) => unlisten());
    } catch (error) {
      unlisteners.forEach((unlisten) => unlisten());
      throw error;
    }
  };

  const handleReload = async () => {
    setLoading(true);
    try {
      const list = await api.reloadPlugins();
      setPlugins(list);
    } finally {
      setLoading(false);
    }
  };

  const updatePluginParam = (pluginName: string, key: string, value: string | number | boolean | null) => {
    setParamValues((prev) => ({
      ...prev,
      [pluginName]: { ...(prev[pluginName] ?? {}), [key]: value },
    }));
  };

  /** 将 UI 草稿值按 schema 类型转换为传给插件的实际值。空字符串/未设置传 undefined（插件用默认）。 */
  const buildRunParams = (plugin: PluginInfo): Record<string, unknown> => {
    const schema = plugin.parametersSchema && typeof plugin.parametersSchema === 'object'
      ? plugin.parametersSchema
      : {};
    const draft = paramValues[plugin.name] ?? {};
    const params: Record<string, unknown> = {};
    for (const [key, s] of Object.entries(schema)) {
      const raw = draft[key];
      if (raw === null || raw === undefined || raw === '') continue;
      const type = s.type ?? 'string';
      if (type === 'boolean') {
        params[key] = Boolean(raw);
      } else if (type === 'number' || type === 'integer') {
        const num = typeof raw === 'number' ? raw : Number(raw);
        if (!Number.isNaN(num)) params[key] = num;
      } else {
        params[key] = String(raw);
      }
    }
    return params;
  };

  const handleRun = async (plugin: PluginInfo, video: VideoInfo | null) => {
    setLogs([]);
    if (!video) {
      setLogs(['[WARN] 请先在媒体库选择一个视频']);
      return;
    }
    const taskId = `task-${Date.now()}`;
    const outputDir = `${settings.importDirectory || '.'}/plugin-output/${plugin.name}/${taskId}`;
    const workToken = useUpdateStore.getState().beginWork('plugin');
    try {
      setRunningTasks((prev) => ({ ...prev, [taskId]: { progress: 0, message: '启动中...' } }));
      const result = await api.runPlugin(plugin.name, video, buildRunParams(plugin), outputDir);
      setLogs((prev) => [...prev, `[OK] 完成：生成 ${result.marks.length} 个标注`]);
      setRunningTasks((prev) => {
        const next = { ...prev };
        delete next[taskId];
        return next;
      });
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      setLogs((prev) => [...prev, `[ERROR] ${message}`]);
      setRunningTasks((prev) => {
        const next = { ...prev };
        delete next[taskId];
        return next;
      });
    } finally {
      useUpdateStore.getState().endWork(workToken);
    }
  };

  const handleCancel = async (pluginName: string, taskId: string) => {
    await api.cancelPluginRun(pluginName, taskId);
    setLogs((prev) => [...prev, `[INFO] 已请求取消`]);
  };

  const handleStopPlugin = async (pluginName: string) => {
    await api.stopPlugin(pluginName);
    loadPlugins();
  };

  // M4: 批量执行所有选中插件（在选中视频上）
  const handleRunSelected = async () => {
    if (selectedPlugins.size === 0) {
      setLogs(['[WARN] 请先勾选要运行的插件']);
      return;
    }
    if (!selectedVideo) {
      setLogs(['[WARN] 请先在媒体库选择一个视频']);
      return;
    }
    setOrchestrationRunning(true);
    const workToken = useUpdateStore.getState().beginWork('plugin');
    setOrchestrationProgress({ completed: 0, total: selectedPlugins.size, stage: 'started' });
    setOrchestrationResult(null);
    setLogs((prev) => [
      ...prev.slice(-99),
      `[BATCH] 开始编排: ${selectedPlugins.size} 个插件 × 1 个视频, 模式=${orchestrationMode}`,
    ]);
    try {
      const outputDir = `${settings.importDirectory || '.'}/plugin-output/batch-${Date.now()}`;
      // 为每个插件构建 per-plugin 参数，保证批量运行也使用 UI 里的配置。
      const perPluginParams: Record<string, Record<string, unknown>> = {};
      for (const name of selectedPlugins) {
        const plugin = plugins.find((p) => p.name === name);
        if (plugin) perPluginParams[name] = buildRunParams(plugin);
      }
      const result = await api.runPlugins({
        pluginNames: Array.from(selectedPlugins),
        videos: [selectedVideo],
        perPluginParams,
        outputDir,
        mode: orchestrationMode,
      });
      setOrchestrationResult(result);
      setLogs((prev) => [
        ...prev.slice(-99),
        `[BATCH OK] 完成：合并后 ${result.totalMarks} 个标注，失败 ${result.failedCount} 个`,
      ]);
      // 失败的细节
      for (const summary of result.videoSummaries) {
        for (const o of summary.pluginResults) {
          if (o.outcome !== 'ok') {
            setLogs((prev) => [
              ...prev.slice(-99),
              `[BATCH ${o.outcome.toUpperCase()}] ${o.pluginName}: ${o.error || '未知错误'}`,
            ]);
          }
        }
      }
    } catch (e: any) {
      setLogs((prev) => [...prev.slice(-99), `[BATCH ERROR] ${e?.message || e}`]);
    } finally {
      setOrchestrationRunning(false);
      useUpdateStore.getState().endWork(workToken);
    }
  };

  const togglePluginSelected = (name: string) => {
    setSelectedPlugins((prev) => {
      const next = new Set(prev);
      if (next.has(name)) next.delete(name);
      else next.add(name);
      return next;
    });
  };

  const toggleAllPlugins = () => {
    const runnable = plugins.filter((p) => p.status === 'ready' || p.status === 'crashed');
    if (selectedPlugins.size === runnable.length) {
      setSelectedPlugins(new Set());
    } else {
      setSelectedPlugins(new Set(runnable.map((p) => p.name)));
    }
  };

  // 找到当前选中的视频（从 store）
  const videos = useAppStore((state) => state.videos);
  const selectedVideoId = useAppStore((state) => state.selectedVideoId);
  const selectedVideo = videos.find((v) => v.id === selectedVideoId) || videos[0] || null;

  const tabItems = useMemo(
    () => [
      {
        key: 'local',
        title: (
          <span className="flex items-center gap-1.5">
            <Code className="h-4 w-4" aria-hidden="true" />
            本地插件 ({plugins.length})
          </span>
        ),
        content: (
          <LocalPluginsTab
            plugins={plugins}
            loading={loading}
            selectedVideo={selectedVideo}
            runningTasks={runningTasks}
            onRun={handleRun}
            onCancel={handleCancel}
            onStop={handleStopPlugin}
            selectedPlugins={selectedPlugins}
            onToggleSelected={togglePluginSelected}
            orchestrationMode={orchestrationMode}
            setOrchestrationMode={setOrchestrationMode}
            orchestrationRunning={orchestrationRunning}
            orchestrationProgress={orchestrationProgress}
            orchestrationResult={orchestrationResult}
            handleRunSelected={handleRunSelected}
            toggleAllPlugins={toggleAllPlugins}
            logs={logs}
            setLogs={setLogs}
            paramValues={paramValues}
            onParamChange={updatePluginParam}
          />
        ),
      },
      {
        key: 'discover',
        title: (
          <span className="flex items-center gap-1.5">
            <Github className="h-4 w-4" aria-hidden="true" />
            发现插件
          </span>
        ),
        content: <GitHubPluginDiscovery />,
      },
    ],
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [plugins, loading, selectedVideo, runningTasks, selectedPlugins, orchestrationMode, orchestrationRunning, orchestrationProgress, orchestrationResult, logs, paramValues],
  );

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-default-400">
          <Loader2 className="h-6 w-6 animate-spin" />
          <span className="text-sm">加载插件中…</span>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex flex-wrap items-center justify-between gap-4 border-b border-divider px-4 py-5 lg:px-8">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-xl font-semibold tracking-tight">插件</h1>
            {plugins.some((p) => p.status === 'crashed') && (
              <Badge className="border border-warning/40 bg-warning/10 text-warning">
                有插件已崩溃
              </Badge>
            )}
          </div>
          <p className="mt-1 text-sm text-default-500">
            管理自动打标插件，新插件放入 plugins/ 目录即可被识别
          </p>
        </div>
        <Button variant="secondary" onPress={handleReload} isDisabled={loading}>
          <RefreshCw className={cn('h-4 w-4', loading && 'animate-spin')} aria-hidden="true" />
          重新加载
        </Button>
      </header>

      <main className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-4 pb-8 lg:px-8">
        <Tabs value={activeTab} onValueChange={setActiveTab} aria-label="插件管理" className="pt-4">
          {tabItems.map((item) => (
            <Tab key={item.key} tabKey={item.key} title={item.title}>
              {item.content}
            </Tab>
          ))}
        </Tabs>
      </main>
    </div>
  );
}

interface LocalPluginsTabProps {
  plugins: PluginInfo[];
  loading: boolean;
  selectedVideo: VideoInfo | null;
  runningTasks: Record<string, { progress: number; message: string }>;
  onRun: (plugin: PluginInfo, video: VideoInfo | null) => void;
  onCancel: (pluginName: string, taskId: string) => void;
  onStop: (pluginName: string) => void;
  selectedPlugins: Set<string>;
  onToggleSelected: (name: string) => void;
  orchestrationMode: OrchestrationMode;
  setOrchestrationMode: (mode: OrchestrationMode) => void;
  orchestrationRunning: boolean;
  orchestrationProgress: { completed: number; total: number; stage: string } | null;
  orchestrationResult: OrchestrationResult | null;
  handleRunSelected: () => void;
  toggleAllPlugins: () => void;
  logs: string[];
  setLogs: React.Dispatch<React.SetStateAction<string[]>>;
  paramValues: Record<string, PluginParamValues>;
  onParamChange: (pluginName: string, key: string, value: string | number | boolean | null) => void;
}

function LocalPluginsTab({
  plugins,
  loading,
  selectedVideo,
  runningTasks,
  onRun,
  onCancel,
  onStop,
  selectedPlugins,
  onToggleSelected,
  orchestrationMode,
  setOrchestrationMode,
  orchestrationRunning,
  orchestrationProgress,
  orchestrationResult,
  handleRunSelected,
  toggleAllPlugins,
  logs,
  setLogs,
  paramValues,
  onParamChange,
}: LocalPluginsTabProps) {
  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="flex flex-col items-center gap-3 text-default-400">
          <Loader2 className="h-6 w-6 animate-spin" />
          <span className="text-sm">加载插件中…</span>
        </div>
      </div>
    );
  }

  const runnableCount = plugins.filter((p) => p.status === 'ready' || p.status === 'crashed').length;
  const allSelected = selectedPlugins.size > 0 && selectedPlugins.size === runnableCount;

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto">
      {/* M4 编排工具栏 */}
      {plugins.length > 0 && (
        <FormSection
          title="批量运行"
          description="勾选插件后在当前视频上批量执行，可切换串行/并行模式。"
        >
          <div className="flex flex-wrap items-center gap-4">
            <Checkbox
              checked={allSelected}
              onCheckedChange={toggleAllPlugins}
              label="全选可运行插件"
              aria-label="全选可运行插件"
            />
            <div className="h-4 w-px bg-divider" />
            <FormField label="模式" fieldId="orchestration-mode" labelAuto className="min-w-[220px]">
              <SelectButtonGroup
                value={orchestrationMode}
                onValueChange={setOrchestrationMode}
                isDisabled={orchestrationRunning}
                aria-label="批量运行模式"
                options={[
                  { value: 'sequential', label: '串行' },
                  { value: 'parallel', label: '并行' },
                ]}
              />
            </FormField>
            <Button
              size="sm"
              onPress={handleRunSelected}
              isDisabled={orchestrationRunning || selectedPlugins.size === 0 || !selectedVideo}
            >
              {orchestrationRunning ? (
                <>
                  <Loader2 className="mr-1 h-4 w-4 animate-spin" aria-hidden="true" />
                  执行中 ({orchestrationProgress?.completed}/{orchestrationProgress?.total})
                </>
              ) : (
                <>
                  <Zap className="mr-1 h-4 w-4" aria-hidden="true" />
                  批量运行 ({selectedPlugins.size})
                </>
              )}
            </Button>
            {orchestrationResult && (
              <div className="ml-auto text-xs text-default-500">
                上次结果：{orchestrationResult.totalMarks} 个标注，{orchestrationResult.failedCount} 个失败
              </div>
            )}
          </div>
        </FormSection>
      )}

      {/* 插件列表 */}
      <div className="flex-1 overflow-auto space-y-3">
        {plugins.length === 0 ? (
          <Card className="flex flex-col items-center justify-center gap-3 rounded-2xl border-divider bg-default-50/50 p-10 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/15 text-primary">
              <Code className="h-7 w-7" aria-hidden="true" />
            </div>
            <div>
              <p className="text-sm font-medium">未发现插件</p>
              <p className="mt-1 text-xs leading-relaxed text-default-400">
                在 <code className="rounded border border-divider bg-background px-1 font-mono">plugins/</code> 目录
                或设置的自定义插件目录下放入 <code className="rounded border border-divider bg-background px-1 font-mono">plugin.json</code> + 入口文件，
                点击「重新加载」即可发现。
              </p>
            </div>
          </Card>
        ) : (
          plugins.map((plugin) => (
            <PluginCard
              key={plugin.name}
              plugin={plugin}
              selectedVideo={selectedVideo}
              runningTasks={runningTasks}
              onRun={onRun}
              onCancel={onCancel}
              onStop={onStop}
              selected={selectedPlugins.has(plugin.name)}
              onToggleSelected={() => onToggleSelected(plugin.name)}
              selectable={plugin.status === 'ready' || plugin.status === 'crashed'}
              paramValues={paramValues[plugin.name] ?? {}}
              onParamChange={(key, value) => onParamChange(plugin.name, key, value)}
            />
          ))
        )}
      </div>

      {/* 日志面板 */}
      {logs.length > 0 && (
        <Card className="flex h-64 flex-col rounded-2xl border-divider bg-default-50/50">
          <CardHeader className="flex items-center justify-between p-4 pb-3">
            <h3 className="text-sm font-medium">插件日志</h3>
            <Button variant="ghost" size="icon" onPress={() => setLogs([])}>
              <Trash2 className="h-4 w-4" aria-hidden="true" />
            </Button>
          </CardHeader>
          <CardBody className="flex-1 overflow-auto rounded-xl bg-background/60 p-3 font-mono text-xs">
            <div className="space-y-1">
              {logs.map((log, i) => (
                <div
                  key={i}
                  className={cn(
                    'px-2 py-1 rounded',
                    log.includes('[ERROR]') && 'bg-danger-50 text-danger',
                    log.includes('[WARN]') && 'bg-warning-50 text-warning',
                    log.includes('[OK]') && 'bg-success-50 text-success'
                  )}
                >
                  {log}
                </div>
              ))}
            </div>
          </CardBody>
        </Card>
      )}
    </div>
  );
}

interface PluginCardProps {
  plugin: PluginInfo;
  selectedVideo: VideoInfo | null;
  runningTasks: Record<string, { progress: number; message: string }>;
  onRun: (plugin: PluginInfo, video: VideoInfo | null) => void;
  onCancel: (pluginName: string, taskId: string) => void;
  onStop: (pluginName: string) => void;
  selected: boolean;
  onToggleSelected: () => void;
  selectable: boolean;
  paramValues: PluginParamValues;
  onParamChange: (key: string, value: string | number | boolean | null) => void;
}

/** 根据参数 schema 类型渲染对应的输入控件。 */
function PluginParamControl({
  paramKey,
  schema,
  value,
  onChange,
  disabled,
}: {
  paramKey: string;
  schema: PluginParameterSchema;
  value: string | number | boolean | null;
  onChange: (value: string | number | boolean | null) => void;
  disabled?: boolean;
}) {
  const id = `plugin-param-${paramKey}`;
  const type = schema.type ?? 'string';
  const label = schema.title || paramKey;
  const hint = schema.description;

  if (type === 'boolean') {
    return (
      <FormField label={label} hint={hint} fieldId={id}>
        <Switch
          checked={Boolean(value)}
          onCheckedChange={onChange}
          isDisabled={disabled}
          aria-label={label}
        />
      </FormField>
    );
  }

  if (type === 'number' || type === 'integer') {
    return (
      <FormField label={label} hint={hint} fieldId={id}>
        <Input
          id={id}
          type="number"
          inputMode={type === 'integer' ? 'numeric' : 'decimal'}
          value={value === null || value === undefined ? '' : String(value)}
          onValueChange={(v) => {
            if (v === '') { onChange(null); return; }
            const n = type === 'integer' ? parseInt(v, 10) : parseFloat(v);
            onChange(Number.isNaN(n) ? null : n);
          }}
          isDisabled={disabled}
          aria-label={label}
          placeholder={hint}
        />
      </FormField>
    );
  }

  return (
    <FormField label={label} hint={hint} fieldId={id}>
      <Input
        id={id}
        type="text"
        value={value === null || value === undefined ? '' : String(value)}
        onValueChange={(v) => onChange(v === '' ? null : v)}
        isDisabled={disabled}
        aria-label={label}
        placeholder={hint}
      />
    </FormField>
  );
}

function PluginCard({
  plugin,
  selectedVideo,
  runningTasks,
  onRun,
  onCancel,
  onStop,
  selected,
  onToggleSelected,
  selectable,
  paramValues,
  onParamChange,
}: PluginCardProps) {
  const isRunning = plugin.status === 'running';
  const isReady = plugin.status === 'ready';
  const isCrashed = plugin.status === 'crashed';
  const isInvalid = plugin.status === 'invalid';

  const statusIcon = isRunning
    ? <Loader2 className="h-4 w-4 animate-spin text-primary" />
    : isReady
    ? <CheckCircle className="h-4 w-4 text-success" />
    : isCrashed
    ? <AlertTriangle className="h-4 w-4 text-danger" />
    : isInvalid
    ? <XCircle className="h-4 w-4 text-warning" />
    : <RotateCcw className="h-4 w-4 text-default-400" />;

  const statusText = isRunning
    ? '运行中'
    : isReady
    ? '就绪'
    : isCrashed
    ? '已崩溃'
    : isInvalid
    ? '无效'
    : '未知';

  // 查找该插件的运行中任务
  const runningTask = Object.entries(runningTasks).find(([_, v]) => v); // 简化：只要有进度就认为在跑

  const schema = plugin.parametersSchema && typeof plugin.parametersSchema === 'object'
    ? plugin.parametersSchema
    : null;
  const hasSchema = Boolean(schema && Object.keys(schema).length > 0);

  return (
    <Card className="overflow-hidden rounded-2xl border-divider bg-default-50/50">
      <CardHeader className="p-4">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            {selectable && (
              <Checkbox
                checked={selected}
                onCheckedChange={onToggleSelected}
                aria-label={`选择插件 ${plugin.name}`}
                className="shrink-0"
              />
            )}
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/15 text-primary">
              <Code className="h-5 w-5" aria-hidden="true" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h3 className="truncate font-medium">{plugin.name}</h3>
                <Badge className="border border-divider bg-background text-xs">{plugin.runtime}</Badge>
                {statusIcon}
                <span className="text-xs text-default-400">{statusText}</span>
              </div>
              <p className="mt-1 truncate text-xs text-default-400">{plugin.description}</p>
              <p className="mt-1 text-xs text-default-400">
                v{plugin.version} · {plugin.author} · {plugin.capabilities.join(', ') || '无声明能力'}
              </p>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {isRunning && runningTask && (
              <div className="flex items-center gap-2 w-48">
                <Progress
                  value={runningTask[1].progress * 100}
                  className="flex-1"
                  label={`${plugin.name} 运行进度`}
                />
                <Button
                  variant="ghost"
                  size="icon"
                  onPress={() => onCancel(plugin.name, runningTask[0])}
                  aria-label="取消任务"
                >
                  <Square className="h-4 w-4" />
                </Button>
              </div>
            )}
            {isReady && selectedVideo && (
              <Button size="sm" onPress={() => onRun(plugin, selectedVideo)}>
                <Play className="h-4 w-4 mr-1" aria-hidden="true" />
                运行
              </Button>
            )}
            {isCrashed && selectedVideo && (
              <Button variant="secondary" size="sm" onPress={() => onRun(plugin, selectedVideo)}>
                <RotateCcw className="h-4 w-4 mr-1" aria-hidden="true" />
                重试
              </Button>
            )}
            {(isRunning || isReady) && (
              <Button variant="ghost" size="icon" onPress={() => onStop(plugin.name)} aria-label="停止插件">
                <Square className="h-4 w-4" />
              </Button>
            )}
          </div>
        </div>
      </CardHeader>

      {hasSchema && schema && (
        <CardBody className="px-4 pb-4 pt-0">
          <div className="space-y-4 rounded-xl border border-divider bg-background p-4">
            <h4 className="text-xs font-semibold text-default-500">运行参数</h4>
            <div className="grid gap-4 text-sm sm:grid-cols-2">
              {Object.entries(schema).map(([key, s]) => (
                <PluginParamControl
                  key={key}
                  paramKey={key}
                  schema={s}
                  value={paramValues[key] ?? null}
                  onChange={(v) => onParamChange(key, v)}
                  disabled={isRunning}
                />
              ))}
            </div>
          </div>
        </CardBody>
      )}

      {plugin.statusDetail && (
        <CardBody className="px-4 pb-4 pt-0">
          <div className="rounded-xl border border-warning/30 bg-warning/10 p-3 text-xs text-warning">
            {plugin.statusDetail}
          </div>
        </CardBody>
      )}

      {plugin.pythonEnvironment && plugin.pythonEnvironment.stage !== 'ready' && (
        <CardBody className="px-4 pb-4 pt-0">
          <div className={`rounded-xl border p-3 text-xs ${
            plugin.pythonEnvironment.stage === 'failed'
              ? 'border-danger/30 bg-danger/10 text-danger'
              : plugin.pythonEnvironment.stage === 'absent'
              ? 'border-divider bg-default-100 text-default-500'
              : 'border-primary/30 bg-primary/10 text-primary'
          }`}>
            <p className="font-medium">
              Python 环境：
              {plugin.pythonEnvironment.stage === 'absent' && '未建立（首次运行插件时自动创建）'}
              {plugin.pythonEnvironment.stage === 'creating-venv' && '正在创建虚拟环境…'}
              {plugin.pythonEnvironment.stage === 'installing' && '正在安装依赖…'}
              {plugin.pythonEnvironment.stage === 'failed' && '建置失败'}
            </p>
            {plugin.pythonEnvironment.venvDir && (
              <p className="mt-1 break-all text-[10px] opacity-70">{plugin.pythonEnvironment.venvDir}</p>
            )}
            {plugin.pythonEnvironment.pythonVersion && (
              <p className="mt-1 text-[10px] opacity-70">Python {plugin.pythonEnvironment.pythonVersion}</p>
            )}
            {plugin.pythonEnvironment.error && (
              <pre className="mt-2 whitespace-pre-wrap break-words font-mono text-[10px]">{plugin.pythonEnvironment.error}</pre>
            )}
          </div>
        </CardBody>
      )}
    </Card>
  );
}
