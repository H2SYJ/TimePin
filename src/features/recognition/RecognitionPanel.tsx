import { useCallback, useEffect, useState } from 'react';
import {
  Play,
  RotateCcw,
  Loader2,
  FileText,
  Image as ImageIcon,
  Settings,
  Zap,
  Download,
} from 'lucide-react';
import {
  Button,
  Card,
  CardBody,
  CardHeader,
  Input,
  Label,
  Select,
  SelectItem,
  Tabs,
  Tab,
  Badge,
} from '@/components/ui';
import { RoiSelector } from '@/features/recognition/RoiSelector';
import { RecognitionPreview } from '@/features/recognition/RecognitionPreview';
import { api } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import { useUpdateStore } from '@/stores/updater';
import type { NormalizedRect } from '@/types';

interface TemplateMatchConfig {
  templateName: string;
  threshold: number;
  scales: number[];
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
}

interface OcrConfig {
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
  lang?: string;
}

interface DetectConfig {
  roi?: NormalizedRect;
  frameTimestamps?: number[];
  maxFrames: number;
}

export interface RecognitionResult {
  type: 'template' | 'ocr' | 'detect';
  matches?: unknown[];
  results?: unknown[];
  logs: string[];
}

interface Overlay {
  type: 'rect' | 'text' | 'point';
  rect?: { x: number; y: number; width: number; height: number };
  text?: string;
  label?: string;
  confidence?: number;
  color?: string;
}

interface PreviewFrameShape {
  frameIndex: number;
  timestamp: number;
  imageBase64: string;
  width: number;
  height: number;
}

type TabKey = 'template' | 'ocr' | 'detect' | 'preview';
type TaskKind = 'template' | 'ocr' | 'detect';

export function RecognitionPanel({
  videoPath,
  videoDuration,
  onResultsChange,
}: {
  videoPath: string;
  videoDuration: number;
  onResultsChange?: (results: RecognitionResult) => void;
}) {
  const [activeTab, setActiveTab] = useState<TabKey>('template');
  const [running, setRunning] = useState(false);
  const [currentTask, setCurrentTask] = useState<TaskKind | null>(null);
  const [progress, setProgress] = useState(0);
  const [logs, setLogs] = useState<string[]>([]);
  const [previewFrame, setPreviewFrame] = useState<PreviewFrameShape | null>(null);
  const [previewFrames, setPreviewFrames] = useState<PreviewFrameShape[]>([]);
  const [currentFrameIndex, setCurrentFrameIndex] = useState(0);
  const [isPlaying, setIsPlaying] = useState(false);
  const [playbackSpeed, setPlaybackSpeed] = useState(1);
  const [overlays, setOverlays] = useState<Overlay[]>([]);
  const [roi, setRoi] = useState<NormalizedRect | null>(null);

  const [templateConfig, setTemplateConfig] = useState<TemplateMatchConfig>({
    templateName: '',
    threshold: 0.8,
    scales: [1.0],
    maxFrames: 50,
  });
  const [ocrConfig, setOcrConfig] = useState<OcrConfig>({
    maxFrames: 30,
    lang: 'ch',
  });
  const [detectConfig, setDetectConfig] = useState<DetectConfig>({
    maxFrames: 30,
  });

  const [availableTemplates, setAvailableTemplates] = useState<string[]>([]);

  useEffect(() => {
    loadTemplates();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const loadTemplates = async () => {
    try {
      const res = await api.recognitionRoiList();
      setAvailableTemplates(res.rois ?? []);
    } catch (e) {
      console.warn('加载模板列表失败:', e);
    }
  };

  const addLog = useCallback((message: string, level: 'info' | 'warn' | 'error' = 'info') => {
    const prefix = level === 'error' ? '[ERROR]' : level === 'warn' ? '[WARN]' : '[INFO]';
    setLogs((prev) => [...prev.slice(-99), `${prefix} ${message}`]);
  }, []);

  const generatePreview = useCallback(
    async (timestamp: number) => {
      try {
        const frame = await api.recognitionPreview({
          taskId: `preview-${Date.now()}`,
          videoPath,
          timestamp,
          templateMatches: overlays
            .filter((o) => o.type === 'rect' && o.label && !o.text)
            .map((o) => ({
              rect: o.rect!,
              confidence: o.confidence ?? 0,
              scale: 1,
              templateName: o.label ?? '',
              timestamp,
            })),
          ocrResults: overlays
            .filter((o) => o.type === 'rect' && o.text)
            .map((o) => ({
              rect: o.rect!,
              text: o.text ?? '',
              confidence: o.confidence ?? 0,
              timestamp,
            })),
          detections: [],
          rois: roi ? [roi] : [],
          quality: 80,
        });
        setPreviewFrame(frame);
        setPreviewFrames((prev) => (prev.length === 0 ? [frame] : prev));
      } catch (e) {
        console.warn('生成预览失败:', e);
      }
    },
    [videoPath, overlays, roi],
  );

  const runTemplateMatch = useCallback(async () => {
    if (!templateConfig.templateName) {
      addLog('请选择模板', 'warn');
      return;
    }

    setRunning(true);
    setCurrentTask('template');
    setProgress(0);
    setLogs([]);
    const workToken = useUpdateStore.getState().beginWork('recognition');

    try {
      addLog(`开始模板匹配: ${templateConfig.templateName}`);
      const result = await api.recognitionTemplateMatch({
        taskId: `task-${Date.now()}`,
        videoPath,
        templateName: templateConfig.templateName,
        threshold: templateConfig.threshold,
        scales: templateConfig.scales,
        roi: templateConfig.roi,
        frameTimestamps: templateConfig.frameTimestamps,
        maxFrames: templateConfig.maxFrames,
      });

      addLog(`模板匹配完成，找到 ${result.matches.length} 个匹配`);
      const newOverlays: Overlay[] = result.matches.map((m) => ({
        type: 'rect' as const,
        rect: m.rect,
        label: m.templateName,
        confidence: m.confidence,
        color: '#22c55e',
      }));
      setOverlays((prev) => [...prev, ...newOverlays]);

      const first = result.matches[0];
      if (first) await generatePreview(first.timestamp);

      onResultsChange?.({
        type: 'template',
        matches: result.matches,
        logs: [`找到 ${result.matches.length} 个匹配`],
      });
    } catch (e) {
      addLog(`模板匹配失败: ${e instanceof Error ? e.message : String(e)}`, 'error');
    } finally {
      setRunning(false);
      setCurrentTask(null);
      setProgress(1);
      useUpdateStore.getState().endWork(workToken);
    }
  }, [templateConfig, videoPath, addLog, onResultsChange, generatePreview]);

  const runOcr = useCallback(async () => {
    setRunning(true);
    setCurrentTask('ocr');
    setProgress(0);
    setLogs([]);
    const workToken = useUpdateStore.getState().beginWork('recognition');

    try {
      addLog('开始 OCR 识别');
      const result = await api.recognitionOcr({
        taskId: `task-${Date.now()}`,
        videoPath,
        roi: ocrConfig.roi,
        frameTimestamps: ocrConfig.frameTimestamps,
        maxFrames: ocrConfig.maxFrames,
        lang: ocrConfig.lang,
      });

      addLog(`OCR 完成，识别 ${result.results.length} 个文本区域`);
      const newOverlays: Overlay[] = result.results.map((r) => ({
        type: 'rect' as const,
        rect: r.rect,
        text: r.text,
        confidence: r.confidence,
        color: '#3b82f6',
      }));
      setOverlays((prev) => [...prev, ...newOverlays]);

      const first = result.results[0];
      if (first) await generatePreview(first.timestamp);

      onResultsChange?.({
        type: 'ocr',
        results: result.results,
        logs: [`识别 ${result.results.length} 个文本区域`],
      });
    } catch (e) {
      addLog(`OCR 失败: ${e instanceof Error ? e.message : String(e)}`, 'error');
    } finally {
      setRunning(false);
      setCurrentTask(null);
      setProgress(1);
      useUpdateStore.getState().endWork(workToken);
    }
  }, [ocrConfig, videoPath, addLog, onResultsChange, generatePreview]);

  const runDetect = useCallback(async () => {
    setRunning(true);
    setCurrentTask('detect');
    setProgress(0);
    setLogs([]);
    const workToken = useUpdateStore.getState().beginWork('recognition');

    try {
      addLog('开始目标检测');
      const result = await api.recognitionDetect({
        taskId: `task-${Date.now()}`,
        videoPath,
        roi: detectConfig.roi,
        frameTimestamps: detectConfig.frameTimestamps,
        maxFrames: detectConfig.maxFrames,
      });

      addLog(`目标检测完成，检测到 ${result.results.length} 个目标`);
      const newOverlays: Overlay[] = result.results.map((r) => ({
        type: 'rect' as const,
        rect: r.rect,
        label: r.label,
        confidence: r.confidence,
        color: '#ef4444',
      }));
      setOverlays((prev) => [...prev, ...newOverlays]);

      const first = result.results[0];
      if (first) await generatePreview(first.timestamp);

      onResultsChange?.({
        type: 'detect',
        results: result.results,
        logs: [`检测到 ${result.results.length} 个目标`],
      });
    } catch (e) {
      addLog(`目标检测失败: ${e instanceof Error ? e.message : String(e)}`, 'error');
    } finally {
      setRunning(false);
      setCurrentTask(null);
      setProgress(1);
      useUpdateStore.getState().endWork(workToken);
    }
  }, [detectConfig, videoPath, addLog, onResultsChange, generatePreview]);

  const generatePreviewSequence = useCallback(
    async (timestamps: number[]) => {
      const frames: PreviewFrameShape[] = [];
      for (const ts of timestamps) {
        try {
          const frame = await api.recognitionPreview({
            taskId: `preview-${Date.now()}-${ts}`,
            videoPath,
            timestamp: ts,
            quality: 70,
          });
          frames.push(frame);
        } catch (e) {
          console.warn(`生成预览帧 ${ts}s 失败:`, e);
        }
      }
      if (frames.length > 0) {
        setPreviewFrames(frames);
        const first = frames[0];
        if (first) setPreviewFrame(first);
        setCurrentFrameIndex(0);
      }
    },
    [videoPath],
  );

  const clearResults = useCallback(() => {
    setOverlays([]);
    setPreviewFrame(null);
    setPreviewFrames([]);
    setCurrentFrameIndex(0);
    setLogs([]);
    setProgress(0);
  }, []);

  useEffect(() => {
    let unlisten: (() => void) | null = null;
    (async () => {
      unlisten = await api.onRecognitionProgress((e) => {
        setProgress(e.progress);
        if (e.message) addLog(e.message);
      });
    })();
    return () => {
      unlisten?.();
    };
  }, [addLog]);

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto p-4">
      <Tabs
        value={activeTab}
        onValueChange={(k) => setActiveTab(k as TabKey)}
        aria-label="识别功能"
      >
        <Tab
          tabKey="template"
          title={
            <span className="inline-flex items-center gap-1">
              <Zap className="h-4 w-4" /> 模板匹配
            </span>
          }
        >
          <Card>
            <CardHeader className="p-3">
              <h3 className="font-medium">模板匹配配置</h3>
            </CardHeader>
            <CardBody className="space-y-3 p-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>选择模板</Label>
                  <Select
                    selectedKeys={templateConfig.templateName ? [templateConfig.templateName] : []}
                    onSelectionChange={(keys) => {
                      const val = Array.from(keys)[0];
                      setTemplateConfig((prev) => ({
                        ...prev,
                        templateName: typeof val === 'string' ? val : String(val ?? ''),
                      }));
                    }}
                  >
                    {availableTemplates.length === 0 ? (
                      <SelectItem key="__empty" textValue="暂无可用模板">
                        暂无可用模板，请先在 ROI 管理中保存
                      </SelectItem>
                    ) : (
                      availableTemplates.map((t) => (
                        <SelectItem key={t} textValue={t}>
                          {t}
                        </SelectItem>
                      ))
                    )}
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label>匹配阈值</Label>
                  <Input
                    type="number"
                    min={0.1}
                    max={1}
                    step={0.05}
                    value={String(templateConfig.threshold)}
                    onValueChange={(v) =>
                      setTemplateConfig((prev) => ({ ...prev, threshold: Number(v) || 0.8 }))
                    }
                  />
                </div>
                <div className="space-y-1 sm:col-span-2">
                  <Label>多尺度匹配</Label>
                  <Input
                    placeholder="例如: 0.5, 1.0, 1.5, 2.0"
                    value={templateConfig.scales.join(', ')}
                    onValueChange={(v) =>
                      setTemplateConfig((prev) => ({
                        ...prev,
                        scales: v
                          .split(',')
                          .map((s) => Number(s.trim()))
                          .filter((n) => !isNaN(n) && n > 0),
                      }))
                    }
                  />
                  <p className="text-xs text-default-400">逗号分隔的缩放比例，1.0 为原尺寸</p>
                </div>
                <div className="space-y-1 sm:col-span-2">
                  <Label>最大处理帧数</Label>
                  <Input
                    type="number"
                    min={1}
                    max={500}
                    value={String(templateConfig.maxFrames)}
                    onValueChange={(v) =>
                      setTemplateConfig((prev) => ({ ...prev, maxFrames: Number(v) || 50 }))
                    }
                  />
                </div>
              </div>
              <div className="flex gap-2 pt-2">
                <Button
                  onPress={runTemplateMatch}
                  isDisabled={running || !templateConfig.templateName}
                  className="flex-1"
                >
                  {running && currentTask === 'template' ? (
                    <>
                      <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                      运行中… {Math.round(progress * 100)}%
                    </>
                  ) : (
                    <>
                      <Play className="mr-1 h-4 w-4" />
                      开始匹配
                    </>
                  )}
                </Button>
                <Button variant="secondary" onPress={clearResults} isDisabled={running}>
                  <RotateCcw className="mr-1 h-4 w-4" />
                  清除结果
                </Button>
              </div>
            </CardBody>
          </Card>
        </Tab>

        <Tab
          tabKey="ocr"
          title={
            <span className="inline-flex items-center gap-1">
              <FileText className="h-4 w-4" /> OCR 识别
            </span>
          }
        >
          <Card>
            <CardHeader className="p-3">
              <h3 className="font-medium">OCR 识别配置</h3>
            </CardHeader>
            <CardBody className="space-y-3 p-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>识别语言</Label>
                  <Select
                    selectedKeys={[ocrConfig.lang ?? 'ch']}
                    onSelectionChange={(keys) => {
                      const val = Array.from(keys)[0];
                      setOcrConfig((prev) => ({
                        ...prev,
                        lang: typeof val === 'string' ? val : String(val ?? ''),
                      }));
                    }}
                  >
                    <SelectItem key="ch" textValue="ch">
                      中文 (简体)
                    </SelectItem>
                    <SelectItem key="en" textValue="en">
                      英文
                    </SelectItem>
                    <SelectItem key="ch+en" textValue="ch+en">
                      中英混合
                    </SelectItem>
                    <SelectItem key="japan" textValue="japan">
                      日文
                    </SelectItem>
                    <SelectItem key="korean" textValue="korean">
                      韩文
                    </SelectItem>
                  </Select>
                </div>
                <div className="space-y-1">
                  <Label>最大处理帧数</Label>
                  <Input
                    type="number"
                    min={1}
                    max={300}
                    value={String(ocrConfig.maxFrames)}
                    onValueChange={(v) =>
                      setOcrConfig((prev) => ({ ...prev, maxFrames: Number(v) || 30 }))
                    }
                  />
                </div>
              </div>
              <div className="flex gap-2 pt-2">
                <Button onPress={runOcr} isDisabled={running} className="flex-1">
                  {running && currentTask === 'ocr' ? (
                    <>
                      <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                      运行中… {Math.round(progress * 100)}%
                    </>
                  ) : (
                    <>
                      <Play className="mr-1 h-4 w-4" />
                      开始识别
                    </>
                  )}
                </Button>
                <Button variant="secondary" onPress={clearResults} isDisabled={running}>
                  <RotateCcw className="mr-1 h-4 w-4" />
                  清除结果
                </Button>
              </div>
            </CardBody>
          </Card>
        </Tab>

        <Tab
          tabKey="detect"
          title={
            <span className="inline-flex items-center gap-1">
              <ImageIcon className="h-4 w-4" /> 目标检测
            </span>
          }
        >
          <Card>
            <CardHeader className="p-3">
              <h3 className="font-medium">目标检测配置</h3>
            </CardHeader>
            <CardBody className="space-y-3 p-3">
              <div className="grid gap-3 sm:grid-cols-2">
                <div className="space-y-1">
                  <Label>最大处理帧数</Label>
                  <Input
                    type="number"
                    min={1}
                    max={300}
                    value={String(detectConfig.maxFrames)}
                    onValueChange={(v) =>
                      setDetectConfig((prev) => ({ ...prev, maxFrames: Number(v) || 30 }))
                    }
                  />
                </div>
                <div className="space-y-1">
                  <Label className="text-default-500">注意</Label>
                  <p className="text-xs text-default-400">需要先在识别服务配置中加载 ONNX 模型</p>
                </div>
              </div>
              <div className="flex gap-2 pt-2">
                <Button onPress={runDetect} isDisabled={running} className="flex-1">
                  {running && currentTask === 'detect' ? (
                    <>
                      <Loader2 className="mr-1 h-4 w-4 animate-spin" />
                      运行中… {Math.round(progress * 100)}%
                    </>
                  ) : (
                    <>
                      <Play className="mr-1 h-4 w-4" />
                      开始检测
                    </>
                  )}
                </Button>
                <Button variant="secondary" onPress={clearResults} isDisabled={running}>
                  <RotateCcw className="mr-1 h-4 w-4" />
                  清除结果
                </Button>
              </div>
            </CardBody>
          </Card>
        </Tab>

        <Tab
          tabKey="preview"
          title={
            <span className="inline-flex items-center gap-1">
              <Settings className="h-4 w-4" /> 预览调试
            </span>
          }
        >
          {previewFrame && (
            <Card>
              <CardHeader className="p-3">
                <div className="flex items-center justify-between">
                  <h3 className="font-medium">ROI 区域选择</h3>
                  <Badge className="text-xs">
                    {previewFrame.width}×{previewFrame.height}
                  </Badge>
                </div>
              </CardHeader>
              <CardBody className="p-3">
                <RoiSelector
                  imageSrc={`data:image/jpeg;base64,${previewFrame.imageBase64}`}
                  naturalWidth={previewFrame.width}
                  naturalHeight={previewFrame.height}
                  roi={roi ?? undefined}
                  onRoiChange={(r) => setRoi(r)}
                  editable={true}
                  overlays={overlays}
                  maxWidth={800}
                />
              </CardBody>
            </Card>
          )}

          {previewFrame && (
            <Card>
              <CardHeader className="p-3">
                <h3 className="font-medium">识别结果预览</h3>
              </CardHeader>
              <CardBody className="p-3">
                <RecognitionPreview
                  frame={previewFrame}
                  frames={previewFrames}
                  overlays={overlays}
                  isPlaying={isPlaying}
                  playbackSpeed={playbackSpeed}
                  currentFrameIndex={currentFrameIndex}
                  onFrameIndexChange={setCurrentFrameIndex}
                  onPlayPause={() => setIsPlaying(!isPlaying)}
                  onPrevFrame={() => setCurrentFrameIndex(Math.max(0, currentFrameIndex - 1))}
                  onNextFrame={() =>
                    setCurrentFrameIndex(Math.min(previewFrames.length - 1, currentFrameIndex + 1))
                  }
                  onSpeedChange={setPlaybackSpeed}
                  showControls={true}
                  maxWidth={800}
                />
              </CardBody>
            </Card>
          )}

          {!previewFrame && (
            <Card className="p-8 text-center text-default-400">
              <ImageIcon className="mx-auto mb-3 h-12 w-12 opacity-30" />
              <p className="text-sm">暂无预览帧</p>
              <p className="mt-1 text-xs">运行模板匹配、OCR 或目标检测后将自动生成预览</p>
            </Card>
          )}

          {previewFrames.length > 0 && (
            <Card>
              <CardHeader className="p-3">
                <h3 className="font-medium">批量生成预览序列</h3>
              </CardHeader>
              <CardBody className="p-3">
                <div className="flex flex-wrap gap-2">
                  <Button
                    variant="secondary"
                    size="sm"
                    onPress={() => {
                      const timestamps = Array.from(
                        { length: 10 },
                        (_, i) => ((i + 1) * videoDuration) / 11,
                      );
                      generatePreviewSequence(timestamps);
                    }}
                  >
                    <Download className="mr-1 h-4 w-4" />
                    均匀采样 10 帧
                  </Button>
                  <Button
                    variant="secondary"
                    size="sm"
                    onPress={() => {
                      const timestamps = Array.from(
                        { length: 20 },
                        (_, i) => ((i + 1) * videoDuration) / 21,
                      );
                      generatePreviewSequence(timestamps);
                    }}
                  >
                    <Download className="mr-1 h-4 w-4" />
                    均匀采样 20 帧
                  </Button>
                </div>
              </CardBody>
            </Card>
          )}

          {logs.length > 0 && (
            <Card className="flex h-64 flex-col">
              <CardHeader className="flex items-center justify-between p-3">
                <h3 className="text-sm font-medium">运行日志</h3>
                <Button variant="ghost" size="icon" onPress={() => setLogs([])}>
                  <RotateCcw className="h-4 w-4" />
                </Button>
              </CardHeader>
              <CardBody className="flex-1 overflow-auto rounded-lg bg-default-50 p-3 font-mono text-xs">
                <div className="space-y-1">
                  {logs.map((log, i) => (
                    <div
                      key={i}
                      className={cn(
                        'rounded px-2 py-1',
                        log.includes('[ERROR]') && 'bg-danger-50 text-danger',
                        log.includes('[WARN]') && 'bg-warning-50 text-warning',
                        log.includes('[INFO]') && 'bg-success-50 text-success',
                      )}
                    >
                      {log}
                    </div>
                  ))}
                </div>
              </CardBody>
            </Card>
          )}
        </Tab>
      </Tabs>
    </div>
  );
}
