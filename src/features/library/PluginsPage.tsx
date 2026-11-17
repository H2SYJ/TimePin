import { Card, CardBody, CardHeader, Button, Badge, Section, Field, Input, Select, SelectItem, Switch } from '@/components/ui';
import { useEffect, useState } from 'react';
import { Tab, Tabs } from '@heroui/react';
import { GitHubPluginDiscovery } from './GitHubPluginDiscovery';
import { useAppStore } from '@/stores/app';
import { api } from '@/lib/ipc';
import { Loader2, Play, Square, RotateCcw, Trash2, RefreshCw, AlertTriangle, CheckCircle, XCircle, Code } from 'lucide-react';
import { cn } from '@/lib/utils';
import { inferParamKind, type PluginParamSchema } from '@/types';

export function PluginsPage() {
  const { settings } = useAppStore();
  const [activeTab, setActiveTab] = useState<'local' | 'discover'>('local');
  const [discoveryVisited, setDiscoveryVisited] = useState(false);
  const [plugins, setPlugins] = useState<any[]>([]);
  const [loading, setLoading] = useState(true);
  const [runningTasks, setRunningTasks] = useState<Record<string, { progress: number; message: string }>>({});
  const [logs, setLogs] = useState<string[]>([]);
  const [paramValues, setParamValues] = useState<Record<string, Record<string, unknown>>>({});

  useEffect(() => {
    loadPlugins();
    setupEventListeners();
  }, []);

  const loadPlugins = async () => {
    setLoading(true);
    try {
      const list = await api.getPlugins();
      setPlugins(list);
      // 初始化参数默认值
      setParamValues((prev) => {
        const next = { ...prev };
        for (const p of list) {
          next[p.name] ??= Object.fromEntries(
            Object.entries(p.parametersSchema ?? {}).map(([k, s]: [string, any]) => [k, s.default ?? ''])
          );
        }
        return next;
      });
    } catch (e) {
      console.error('加载插件列表失败:', e);
    } finally {
      setLoading(false);
    }
  };

  const setupEventListeners = async () => {
    const unlistenProgress = await api.onPluginProgress((e) => {
      setRunningTasks((prev) => ({
        ...prev,
        [e.taskId]: { progress: e.progress, message: e.message || '' },
      }));
    });
    const unlistenStatus = await api.onPluginStatus((e) => {
      setPlugins((prev) =>
        prev.map((p) =>
          p.name === e.pluginName
            ? { ...p, status: e.status, statusDetail: e.detail || null }
            : p
        )
      );
    });
    const unlistenLog = await api.onPluginLog((e) => {
      setLogs((prev) => [...prev.slice(-99), `[${e.level.toUpperCase()}] ${e.pluginName}: ${e.message}`]);
    });
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
    return () => {
      unlistenProgress();
      unlistenStatus();
      unlistenLog();
      unlistenResult();
    };
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

  const handleParamChange = (pluginName: string, key: string, value: unknown) => {
    setParamValues((prev) => ({
      ...prev,
      [pluginName]: { ...prev[pluginName], [key]: value },
    }));
  };

  const handleRun = async (plugin: any, video: any) => {
    setLogs([]);
    if (!video) {
      setLogs(['[WARN] 请先在媒体库选择一个视频']);
      return;
    }
    const taskId = `task-${Date.now()}`;
    const outputDir = `${settings.importDirectory || '.'}/plugin-output/${plugin.name}/${taskId}`;
    try {
      setRunningTasks((prev) => ({ ...prev, [taskId]: { progress: 0, message: '启动中...' } }));
      const result = await api.runPlugin(plugin.name, video, paramValues[plugin.name] ?? {}, outputDir);
      setLogs((prev) => [...prev, `[OK] 完成：生成 ${result.marks.length} 个标注`]);
      setRunningTasks((prev) => {
        const next = { ...prev };
        delete next[taskId];
        return next;
      });
    } catch (e: any) {
      setLogs((prev) => [...prev, `[ERROR] ${e.message}`]);
      setRunningTasks((prev) => {
        const next = { ...prev };
        delete next[taskId];
        return next;
      });
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

  // 找到当前选中的视频（从 store）
  const videos = useAppStore((state) => state.videos);
  const selectedVideoId = useAppStore((state) => state.selectedVideoId);
  const selectedVideo = videos.find((v) => v.id === selectedVideoId) || videos[0] || null;

  const handleTabChange = (key: React.Key) => {
    if (key !== 'local' && key !== 'discover') return;
    setActiveTab(key);
    if (key === 'discover') {
      setDiscoveryVisited(true);
    } else {
      // 安装已由后端注册；返回本地时读取最新列表，不重启正在运行的插件。
      void loadPlugins();
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-4 px-8 py-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">插件管理</h1>
          <p className="mt-1.5 text-sm text-default-500">
            通过 stdio/JSON-RPC 运行的 Sidecar 插件，崩溃不影响主程序。
          </p>
        </div>
        {activeTab === 'local' && (
          <Button variant="secondary" onPress={handleReload} isDisabled={loading}>
            <RefreshCw className="h-4 w-4 mr-1" aria-hidden="true" />
            重新加载
          </Button>
        )}
      </header>

      <main className="flex min-h-0 flex-1 flex-col">
        <Tabs
          aria-label="插件管理导航"
          selectedKey={activeTab}
          onSelectionChange={handleTabChange}
          variant="underlined"
          destroyInactiveTabPanel={false}
          classNames={{
            base: 'px-8',
            panel: 'min-h-0 flex-1 overflow-hidden p-0 data-[inert=true]:hidden',
          }}
        >
          <Tab key="local" title="本地插件">
            <div className="scrollbar-thin h-full overflow-y-auto px-8 py-6">
              <div className="w-full min-w-0 space-y-5 pb-8">
                {loading ? (
                  <div className="flex flex-col items-center gap-3 py-16 text-default-400" role="status">
                    <Loader2 className="h-6 w-6 animate-spin" aria-hidden="true" />
                    <span className="text-sm">加载插件中…</span>
                  </div>
                ) : plugins.length === 0 ? (
                  <Section title="未发现插件" description="在 plugins/ 目录或设置的自定义插件目录下放入 plugin.json + 入口文件，点击「重新加载」即可发现。">
                    <div className="flex flex-col items-center gap-3 text-default-400 py-8">
                      <Code className="h-12 w-12 opacity-30" aria-hidden="true" />
                      <p className="text-sm">未发现插件</p>
                    </div>
                  </Section>
                ) : (
                  <>
                    {plugins.map((plugin) => (
                      <PluginSection
                        key={plugin.name}
                        plugin={plugin}
                        selectedVideo={selectedVideo}
                        runningTasks={runningTasks}
                        paramValues={paramValues[plugin.name] ?? {}}
                        onParamChange={handleParamChange}
                        onRun={handleRun}
                        onCancel={handleCancel}
                        onStop={handleStopPlugin}
                      />
                    ))}

                    {/* 运行日志 Section */}
                    {logs.length > 0 && (
                      <Section title="运行日志" description="最近 100 条插件输出">
                        <Card className="h-64 flex flex-col">
                          <CardHeader className="flex items-center justify-between p-3">
                            <h3 className="text-sm font-medium">插件日志</h3>
                            <Button variant="ghost" size="icon" onClick={() => setLogs([])}>
                              <Trash2 className="h-4 w-4" aria-hidden="true" />
                            </Button>
                          </CardHeader>
                          <CardBody className="flex-1 overflow-auto p-3 font-mono text-xs bg-default-50 rounded-lg">
                            <div className="space-y-1">
                              {logs.map((log, i) => (
                                <div
                                  key={i}
                                  className={cn(
                                    'px-2 py-1 rounded',
                                    log.includes('[ERROR]') && 'bg-danger-50 text-danger',
                                    log.includes('[CRASHED]') && 'bg-danger-50 text-danger',
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
                      </Section>
                    )}
                  </>
                )}
              </div>
            </div>
          </Tab>
          <Tab key="discover" title="发现插件">
            {/* 首次进入才联网；切换标签不卸载，保留限流等待与安装状态。 */}
            {discoveryVisited && <GitHubPluginDiscovery />}
          </Tab>
        </Tabs>
      </main>
    </div>
  );
}

interface PluginSectionProps {
  plugin: any;
  selectedVideo: any;
  runningTasks: Record<string, { progress: number; message: string }>;
  paramValues: Record<string, unknown>;
  onParamChange: (pluginName: string, key: string, value: unknown) => void;
  onRun: (plugin: any, video: any) => void;
  onCancel: (pluginName: string, taskId: string) => void;
  onStop: (pluginName: string) => void;
}

function PluginSection({
  plugin,
  selectedVideo,
  runningTasks,
  paramValues,
  onParamChange,
  onRun,
  onCancel,
  onStop,
}: PluginSectionProps) {
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
  const runningTask = Object.entries(runningTasks).find(([_, v]) => v);

  return (
    <Section
      title={plugin.name}
      description={`${plugin.description} · v${plugin.version} · ${plugin.author}`}
      trailing={
        <div className="flex items-center gap-2 shrink-0">
          <Badge
            className={cn(
              'text-xs',
              isReady && 'bg-success-50 text-success',
              isRunning && 'bg-primary-50 text-primary',
              isCrashed && 'bg-danger-50 text-danger',
              isInvalid && 'bg-warning-50 text-warning'
            )}
          >
            {statusIcon} {statusText}
          </Badge>
          {isRunning && runningTask && (
            <div className="flex items-center gap-2 w-48">
              <div className="flex-1 h-1.5 bg-default-100 rounded-full overflow-hidden">
                <div
                  className="h-full bg-primary transition-all"
                  style={{ width: `${runningTask[1].progress * 100}%` }}
                />
              </div>
              <Button
                variant="ghost"
                size="icon"
                onClick={() => onCancel(plugin.name, runningTask[0])}
                aria-label="取消任务"
              >
                <Square className="h-4 w-4" />
              </Button>
            </div>
          )}
          {isReady && selectedVideo && (
            <Button size="sm" onClick={() => onRun(plugin, selectedVideo)}>
              <Play className="h-4 w-4 mr-1" aria-hidden="true" />
              运行
            </Button>
          )}
          {isCrashed && (
            <Button variant="secondary" size="sm" onClick={() => onRun(plugin, selectedVideo)}>
              <RotateCcw className="h-4 w-4 mr-1" aria-hidden="true" />
              重试
            </Button>
          )}
          {(isRunning || isReady) && (
            <Button variant="ghost" size="icon" onClick={() => onStop(plugin.name)} aria-label="停止插件">
              <Square className="h-4 w-4" />
            </Button>
          )}
        </div>
      }
    >
      {Object.entries(plugin.parametersSchema ?? {}).map(([key, schema]: [string, any]) => (
        <PluginParamField
          key={key}
          pluginName={plugin.name}
          paramKey={key}
          schema={schema}
          value={paramValues[key] ?? schema.default ?? ''}
          onChange={(value) => onParamChange(plugin.name, key, value)}
        />
      ))}
    </Section>
  );
}

interface PluginParamFieldProps {
  pluginName: string;
  paramKey: string;
  schema: PluginParamSchema;
  value: unknown;
  onChange: (value: unknown) => void;
}

function PluginParamField({ paramKey, schema, value, onChange }: PluginParamFieldProps) {
  const kind = inferParamKind(schema);

  const handleNumberChange = (v: string) => {
    const num = Number(v);
    onChange(Number.isNaN(num) ? '' : num);
  };

  return (
    <Field label={schema.title || paramKey} hint={schema.description}>
      {kind === 'boolean' && (
        <Switch
          checked={Boolean(value)}
          onCheckedChange={(checked) => onChange(checked)}
          label={schema.title || paramKey}
        />
      )}
      {kind === 'number' && (
        <Input
          type="number"
          min={schema.minimum}
          max={schema.maximum}
          step={schema.type === 'integer' ? 1 : undefined}
          value={value === '' ? '' : String(value)}
          onValueChange={handleNumberChange}
        />
      )}
      {kind === 'select' && (
        <Select
          selectedKeys={value !== '' && value !== undefined ? [String(value)] : []}
          onSelectionChange={(keys) => {
            const val = Array.from(keys)[0];
            if (val !== undefined) onChange(val);
          }}
        >
          {(schema.enum ?? []).map((o) => (
            <SelectItem key={String(o)} textValue={String(o)}>
              {String(o)}
            </SelectItem>
          ))}
        </Select>
      )}
      {kind === 'text' && (
        <Input
          value={value === undefined ? '' : String(value)}
          onValueChange={onChange}
        />
      )}
    </Field>
  );
}