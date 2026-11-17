import { AlertTriangle, Check, RotateCcw, Save } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import { Button, Card, Input, Label, Select, SelectItem, Switch } from '@/components/ui';
import { UpdatePanel } from '@/components/UpdatePanel';
import { useAppStore } from '@/stores/app';
import { isTauri } from '@/lib/ipc';
import type { ExportMode, Settings, Theme } from '@/types';
import { AdvancedSettings } from './AdvancedSettings';
import { mergeSettingsDraft, validateSettingsDraft, type SettingsErrors } from './settingsValidation';
import { useDependencyInstaller } from './useDependencyInstaller';

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
  fieldId,
  error,
  children,
}: {
  label: string;
  hint?: string;
  fieldId?: string;
  error?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-1 items-start gap-2 sm:grid-cols-[180px_minmax(0,1fr)] sm:gap-4">
      <div>
        <Label htmlFor={fieldId}>{label}</Label>
        {hint && <p className="mt-1 text-xs text-default-400">{hint}</p>}
      </div>
      <div>
        {children}
        {error && (
          <p id={fieldId ? `${fieldId}-error` : undefined} role="alert" className="mt-1.5 text-sm text-danger">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}

const summaryLabels: Record<string, string> = {
  importDirectory: '默认导入目录',
  thumbnailQuality: '缩略图质量',
  thumbnailMaxDimension: '缩略图最大尺寸',
  maxConcurrentImports: '最大并发导入数',
};

function fieldIdFor(key: string): string | undefined {
  if (key in summaryLabels) return `field-${key}`;
  if (key.startsWith('plugin-')) return key;
  return undefined;
}

export function SettingsPage() {
  const desktop = isTauri();
  const settings = useAppStore((state) => state.settings);
  const updateSettings = useAppStore((state) => state.updateSettings);
  const [draft, setDraft] = useState<Settings>(settings);
  const [saved, setSaved] = useState(false);
  const [saving, setSaving] = useState(false);
  const [attempted, setAttempted] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const summaryRef = useRef<HTMLDivElement>(null);
  const previousSettings = useRef(settings);

  useEffect(() => {
    const previous = previousSettings.current;
    setDraft((current) => mergeSettingsDraft(current, previous, settings));
    previousSettings.current = settings;
  }, [settings]);

  const patch = <K extends keyof Settings>(key: K, value: Settings[K]) => {
    setSaved(false);
    setSaveError(null);
    setDraft((current) => ({ ...current, [key]: value }));
  };

  const errors: SettingsErrors = attempted ? validateSettingsDraft(draft) : {};
  const hasChanges = JSON.stringify(draft) !== JSON.stringify(settings);
  const errorKeys = Object.keys(errors);

  const save = async () => {
    setAttempted(true);
    const found = validateSettingsDraft(draft);
    if (Object.keys(found).length > 0) {
      // 让摘要获得焦点，便于屏幕阅读器/键盘用户定位问题。
      requestAnimationFrame(() => summaryRef.current?.focus());
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const ok = await updateSettings(draft);
      setSaved(ok);
      if (!ok) setSaveError(useAppStore.getState().error ?? '保存设置失败，请重试');
    } catch (cause) {
      setSaveError(String(cause));
      setSaved(false);
    } finally {
      setSaving(false);
    }
  };

  const installer = useDependencyInstaller((result) => {
    // 后端已持久化两个工具路径；只合并这两个字段，保留其它未保存更改。
    patch('ffmpegPath', result.ffmpegPath);
    patch('ffprobePath', result.ffprobePath);
  });

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
          <Button
            variant="ghost"
            onPress={() => {
              setDraft(settings);
              setSaved(false);
              setAttempted(false);
              setSaveError(null);
            }}
            isDisabled={!hasChanges || saving || installer.installing}
          >
            <RotateCcw className="h-4 w-4" /> 撤销更改
          </Button>
          <Button
            onPress={() => void save()}
            isDisabled={!desktop || !hasChanges || saving || installer.installing}
            isLoading={saving}
          >
            {saved && !hasChanges ? <Check className="h-4 w-4" /> : <Save className="h-4 w-4" />}
            {saving ? '保存中…' : saved && !hasChanges ? '已保存' : '保存设置'}
          </Button>
        </div>
      </header>

      <main className="scrollbar-thin min-h-0 flex-1 overflow-y-auto px-8 pb-8">
        <div className="w-full min-w-0 space-y-5 pb-8">
          {errorKeys.length > 0 && (
            <div
              ref={summaryRef}
              tabIndex={-1}
              role="alert"
              aria-labelledby="settings-error-title"
              className="rounded-2xl border border-danger/40 bg-danger/[0.06] px-4 py-3 text-sm"
            >
              <h2 id="settings-error-title" className="flex items-center gap-2 font-medium text-danger">
                <AlertTriangle className="h-4 w-4" aria-hidden="true" />
                请先修正以下 {errorKeys.length} 处设置项
              </h2>
              <ul className="mt-2 list-inside list-disc space-y-1 text-danger">
                {errorKeys.map((key) => (
                  <li key={key}>
                    <a
                      href={fieldIdFor(key) ? `#${fieldIdFor(key)}` : undefined}
                      className="underline underline-offset-2"
                    >
                      {summaryLabels[key] ?? (key.startsWith('plugin-') ? `插件搜索目录 ${Number(key.split('-')[1]) + 1}` : key)}
                    </a>
                    ：{errors[key]}
                  </li>
                ))}
              </ul>
            </div>
          )}
          {saveError && (
            <p role="alert" className="rounded-2xl border border-danger/40 bg-danger/[0.06] px-4 py-3 text-sm text-danger">
              {saveError}
            </p>
          )}

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
            <Field label="默认导入目录" hint="留空时由文件选择器决定" error={errors.importDirectory}>
              <Input
                id="field-importDirectory"
                aria-label="默认导入目录"
                placeholder="例如 D:\Recordings"
                value={draft.importDirectory ?? ''}
                onValueChange={(value) => patch('importDirectory', value || null)}
              />
            </Field>
            <Field label="最大并发导入数" fieldId="field-maxConcurrentImports" hint="同时处理的视频数量" error={errors.maxConcurrentImports}>
              <Select
                id="field-maxConcurrentImports"
                aria-label="最大并发导入数"
                isInvalid={Boolean(errors.maxConcurrentImports)}
                aria-describedby={errors.maxConcurrentImports ? 'field-maxConcurrentImports-error' : undefined}
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
            <Field label="缩略图质量" fieldId="field-thumbnailQuality" hint="1–100，数值越高文件越大" error={errors.thumbnailQuality}>
              <Input
                id="field-thumbnailQuality"
                aria-label="缩略图质量"
                isInvalid={Boolean(errors.thumbnailQuality)}
                aria-describedby={errors.thumbnailQuality ? 'field-thumbnailQuality-error' : undefined}
                type="number"
                min={1}
                max={100}
                value={String(draft.thumbnailQuality)}
                onValueChange={(value) => patch('thumbnailQuality', Number(value))}
              />
            </Field>
            <Field label="缩略图最大尺寸" fieldId="field-thumbnailMaxDimension" hint="按长边等比缩放，单位像素" error={errors.thumbnailMaxDimension}>
              <Input
                id="field-thumbnailMaxDimension"
                aria-label="缩略图最大尺寸"
                isInvalid={Boolean(errors.thumbnailMaxDimension)}
                aria-describedby={errors.thumbnailMaxDimension ? 'field-thumbnailMaxDimension-error' : undefined}
                type="number"
                min={80}
                max={1920}
                value={String(draft.thumbnailMaxDimension)}
                onValueChange={(value) => patch('thumbnailMaxDimension', Number(value))}
              />
            </Field>
          </Section>

          <Section title="导出与处理" description="导出默认采用流复制，不会修改原始 MP4。媒体工具路径已移至下方高级设置。">
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

          <AdvancedSettings
            draft={draft}
            patch={patch}
            errors={errors}
            saving={saving}
            installer={installer}
          />
        </div>
      </main>
    </div>
  );
}
