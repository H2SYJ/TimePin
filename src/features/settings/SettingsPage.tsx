import { Check, Info, RotateCcw, Save } from 'lucide-react';
import { useEffect, useState } from 'react';
import { Button, Card, Input, Label, Select, SelectItem, Switch } from '@/components/ui';
import { UpdatePanel } from '@/components/UpdatePanel';
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
    <Card className="rounded-2xl border-divider bg-default-50/50 p-6">
      <div className="mb-5">
        <h2 className="text-base font-semibold">{title}</h2>
        <p className="mt-1 text-sm text-default-500">{description}</p>
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
    <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:items-center sm:gap-4">
      <div>
        <Label>{label}</Label>
        {hint && <p className="mt-1 text-xs text-default-400">{hint}</p>}
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
      <header className="flex items-center justify-between gap-4 px-8 py-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">设置</h1>
          <p className="mt-1.5 text-sm text-default-500">
            配置保存在本地应用数据目录，重启后自动恢复。
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button variant="ghost" onPress={reset} isDisabled={!hasChanges}>
            <RotateCcw className="h-4 w-4" /> 撤销更改
          </Button>
          <Button onPress={() => void save()} isDisabled={!hasChanges}>
            {saved ? <Check className="h-4 w-4" /> : <Save className="h-4 w-4" />}
            {saved ? '已保存' : '保存设置'}
          </Button>
        </div>
      </header>

      <main className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-8 pb-8">
        <div className="w-full min-w-0 space-y-5 pb-8">
          <Section title="外观与语言" description="调整界面主题和语言偏好。">
            <Field label="主题">
              <Select
                aria-label="主题"
                selectedKeys={[draft.theme]}
                onSelectionChange={(keys) => {
                  const value = Array.from(keys)[0];
                  if (value) patch('theme', value as Theme);
                }}
              >
                <SelectItem key="system">跟随系统</SelectItem>
                <SelectItem key="light">浅色</SelectItem>
                <SelectItem key="dark">深色</SelectItem>
              </Select>
            </Field>
            <Field label="界面语言">
              <Select
                aria-label="界面语言"
                selectedKeys={[draft.language]}
                onSelectionChange={(keys) => {
                  const value = Array.from(keys)[0];
                  if (value) patch('language', String(value));
                }}
              >
                <SelectItem key="zh-CN">简体中文</SelectItem>
                <SelectItem key="en-US">English</SelectItem>
              </Select>
            </Field>
          </Section>

          <Section title="视频与媒体库" description="控制导入并发、缩略图质量及默认目录。">
            <Field label="默认导入目录" hint="留空时由文件选择器决定">
              <Input
                aria-label="默认导入目录"
                placeholder="例如 D:\Recordings"
                value={draft.importDirectory ?? ''}
                onValueChange={(value) => patch('importDirectory', value || null)}
              />
            </Field>
            <Field label="最大并发导入数" hint="同时处理的视频数量">
              <Select
                aria-label="最大并发导入数"
                selectedKeys={[String(draft.maxConcurrentImports)]}
                onSelectionChange={(keys) => {
                  const value = Array.from(keys)[0];
                  if (value) patch('maxConcurrentImports', Number(value));
                }}
              >
                {[1, 2, 3, 4, 6, 8].map((value) => (
                  <SelectItem key={String(value)} textValue={String(value)}>
                    {value}
                  </SelectItem>
                ))}
              </Select>
            </Field>
            <Field label="缩略图质量" hint="1–100，数值越高文件越大">
              <Input
                aria-label="缩略图质量"
                type="number"
                min={1}
                max={100}
                value={String(draft.thumbnailQuality)}
                onValueChange={(value) => patch('thumbnailQuality', Number(value))}
              />
            </Field>
            <Field label="缩略图最大尺寸" hint="按长边等比缩放，单位像素">
              <Input
                aria-label="缩略图最大尺寸"
                type="number"
                min={80}
                max={1920}
                value={String(draft.thumbnailMaxDimension)}
                onValueChange={(value) => patch('thumbnailMaxDimension', Number(value))}
              />
            </Field>
          </Section>

          <Section title="导出与处理" description="导出默认采用流复制，不会修改原始 MP4。">
            <Field label="默认导出模式" hint="帧精确模式会重编码首尾 GOP，耗时更长">
              <Select
                aria-label="默认导出模式"
                selectedKeys={[draft.defaultExportMode]}
                onSelectionChange={(keys) => {
                  const value = Array.from(keys)[0];
                  if (value) patch('defaultExportMode', value as ExportMode);
                }}
              >
                <SelectItem key="streamCopy">快速导出 · 流复制（默认）</SelectItem>
                <SelectItem key="frameAccurate">帧精确 · 首尾 GOP 重编码</SelectItem>
              </Select>
            </Field>
            <Field label="FFmpeg 路径" hint="留空时使用系统 PATH 中的 ffmpeg">
              <Input
                aria-label="FFmpeg 路径"
                placeholder="自动检测"
                value={draft.ffmpegPath ?? ''}
                onValueChange={(value) => patch('ffmpegPath', value || null)}
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

          <UpdatePanel />

          <div className="flex items-start gap-3 rounded-2xl border border-primary/25 bg-primary/[0.08] px-4 py-3 text-sm text-default-500">
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
