import { Check, Info, RotateCcw, Save } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Button, Card, Input, Label, Select, Switch } from '@/components/ui';
import { useAppStore } from '@/stores/app';
import type { ExportMode, Settings, Theme } from '@/types';

function Section({
  title,
  description,
  children,
}: {
  title: string;
  description: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="p-6">
      <div className="mb-5">
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="mt-1 text-sm text-muted-foreground">{description}</p>
      </div>
      <div className="space-y-5">{children}</div>
    </Card>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[180px_1fr] items-center gap-4">
      <div>
        <Label>{label}</Label>
        {hint && <p className="mt-1 text-xs text-muted-foreground">{hint}</p>}
      </div>
      <div>{children}</div>
    </div>
  );
}

export function SettingsPage() {
  const settings = useAppStore((state) => state.settings);
  const updateSettings = useAppStore((state) => state.updateSettings);
  const [draft, setDraft] = useState<Settings>(settings);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    setDraft(settings);
  }, [settings]);

  const patch = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSaved(false);
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const hasChanges = JSON.stringify(draft) !== JSON.stringify(settings);

  const save = async () => {
    const ok = await updateSettings(draft);
    setSaved(ok);
  };

  const reset = () => {
    setDraft(settings);
    setSaved(false);
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <header className="flex items-center justify-between gap-4 border-b border-border px-8 py-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight">软件设置</h1>
          <p className="mt-1 text-sm text-muted-foreground">
            配置保存在本地应用数据目录，重启后自动恢复。
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="ghost" onClick={reset} disabled={!hasChanges}>
            <RotateCcw className="h-4 w-4" /> 撤销更改
          </Button>
          <Button onClick={() => void save()} disabled={!hasChanges}>
            {saved ? <Check className="h-4 w-4" /> : <Save className="h-4 w-4" />}
            {saved ? '已保存' : '保存设置'}
          </Button>
        </div>
      </header>

      <main className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-8 py-6">
        <div className="mx-auto max-w-4xl space-y-5 pb-8">
          <Section title="外观与语言" description="调整界面主题和语言偏好。">
            <Field label="主题">
              <Select
                aria-label="主题"
                value={draft.theme}
                onChange={(event) => patch('theme', event.target.value as Theme)}
              >
                <option value="system">跟随系统</option>
                <option value="light">浅色</option>
                <option value="dark">深色</option>
              </Select>
            </Field>
            <Field label="界面语言">
              <Select
                aria-label="界面语言"
                value={draft.language}
                onChange={(event) => patch('language', event.target.value)}
              >
                <option value="zh-CN">简体中文</option>
                <option value="en-US">English</option>
              </Select>
            </Field>
          </Section>

          <Section title="视频与媒体库" description="控制导入并发、缩略图质量及默认目录。">
            <Field label="默认导入目录" hint="留空时由文件选择器决定">
              <Input
                aria-label="默认导入目录"
                placeholder="例如 D:\\Recordings"
                value={draft.importDirectory ?? ''}
                onChange={(event) => patch('importDirectory', event.target.value || null)}
              />
            </Field>
            <Field label="最大并发导入数" hint="同时处理的视频数量">
              <Select
                aria-label="最大并发导入数"
                value={String(draft.maxConcurrentImports)}
                onChange={(event) => patch('maxConcurrentImports', Number(event.target.value))}
              >
                {[1, 2, 3, 4, 6, 8].map((value) => (
                  <option key={value} value={value}>{value}</option>
                ))}
              </Select>
            </Field>
            <Field label="缩略图质量" hint="1–100，数值越高文件越大">
              <Input
                aria-label="缩略图质量"
                type="number"
                min={1}
                max={100}
                value={draft.thumbnailQuality}
                onChange={(event) => patch('thumbnailQuality', Number(event.target.value))}
              />
            </Field>
            <Field label="缩略图最大尺寸" hint="按长边等比缩放，单位像素">
              <Input
                aria-label="缩略图最大尺寸"
                type="number"
                min={80}
                max={1920}
                value={draft.thumbnailMaxDimension}
                onChange={(event) => patch('thumbnailMaxDimension', Number(event.target.value))}
              />
            </Field>
          </Section>

          <Section title="导出与处理" description="导出默认采用流复制，不会修改原始 MP4。">
            <Field label="默认导出模式" hint="帧精确模式会重编码首尾 GOP，耗时更长">
              <Select
                aria-label="默认导出模式"
                value={draft.defaultExportMode}
                onChange={(event) => patch('defaultExportMode', event.target.value as ExportMode)}
              >
                <option value="streamCopy">快速导出 · 流复制（默认）</option>
                <option value="frameAccurate">帧精确 · 首尾 GOP 重编码</option>
              </Select>
            </Field>
            <Field label="FFmpeg 路径" hint="留空时使用系统 PATH 中的 ffmpeg">
              <Input
                aria-label="FFmpeg 路径"
                placeholder="自动检测"
                value={draft.ffmpegPath ?? ''}
                onChange={(event) => patch('ffmpegPath', event.target.value || null)}
              />
            </Field>
            <Field label="硬件加速" hint="用于后续插件推理与缩略图处理">
              <div className="flex h-9 items-center">
                <Switch
                  checked={draft.hardwareAcceleration}
                  onCheckedChange={(checked) => patch('hardwareAcceleration', checked)}
                  label="启用硬件加速"
                />
              </div>
            </Field>
          </Section>

          <div className="flex items-start gap-3 rounded-xl border border-primary/20 bg-accent/50 px-4 py-3 text-sm text-muted-foreground">
            <Info className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
            <p>
              M1 阶段设置已持久化；插件目录、FFmpeg 自定义路径等高级项将在后续里程碑接入实际处理流程。
            </p>
          </div>
        </div>
      </main>
    </div>
  );
}