import { AlertTriangle, CheckCircle, ChevronDown, ChevronUp, Download, ExternalLink, Github, Loader2, Star, X, AlertCircle, Info, GitFork, ShieldAlert, KeyRound } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Card, CardBody, CardHeader, Checkbox, Input, Badge } from '@/components/ui';
import { api } from '@/lib/ipc';
import { resolveLocalizedText } from '@/lib/localization';
import { cn } from '@/lib/utils';
import { useAppStore } from '@/stores/app';
import type {
  DiscoveredPlugin,
  DiscoverPluginsResult,
  GitHubApiError,
  GitHubRateInfo,
  GitHubSession,
} from '@/types';

/** 匿名主限流横幅：每个应用会话只弹出一次（组件卸载重挂不重复）。 */
let primaryLimitBannerAcked = false;

/** 内存缓存：组件卸载重挂仍可"查看缓存"（不落盘，不假称磁盘缓存）。 */
interface DiscoveryCache {
  page: number;
  plugins: DiscoveredPlugin[];
  totalCount: number;
  /** 缓存写入时的身份；身份变更后不再提供旧凭据派生的远端数据。 */
  identity: GitHubSession['identity'];
  rateLimits: GitHubRateInfo[];
}
let discoveryCache: DiscoveryCache | null = null;
// 仅保留已完成安装的本地副作用，不将旧身份的远端仓库元数据带到新身份。
const installedPluginPaths = new Map<string, string>();

/** 测试辅助：重置模块级横幅确认与内存缓存（仅测试使用）。 */
export function __resetGitHubDiscoverySessionState() {
  primaryLimitBannerAcked = false;
  discoveryCache = null;
  installedPluginPaths.clear();
}

function resourceLabel(resource: string | null): string {
  if (resource === 'search') return 'Search API';
  if (resource === 'core') return 'Core API';
  return resource ? `${resource} API` : 'API（资源未知）';
}

function getRateLimits(snapshot: { rateLimit: GitHubRateInfo | null; rateLimits?: GitHubRateInfo[] }): GitHubRateInfo[] {
  return snapshot.rateLimits?.length ? snapshot.rateLimits : snapshot.rateLimit ? [snapshot.rateLimit] : [];
}

function mergeRateLimits(previous: GitHubRateInfo[], incoming: GitHubRateInfo[]): GitHubRateInfo[] {
  const byResource = new Map(previous.map((rate) => [rate.resource, rate]));
  incoming.forEach((rate) => byResource.set(rate.resource, rate));
  return [...byResource.values()];
}

/** 与主限流同族的本地等待错误（PRIMARY / RATE_WAIT 且有 reset/retry 时间）按资源桶隔离；
 *  secondary、unknown、无 retryAt 的停止状态一律保守全局。 */
function rateErrorScope(error: GitHubApiError): 'search' | 'core' | 'global' {
  const resource = error.rateLimit?.resource;
  if (
    (error.code === 'PRIMARY_RATE_LIMIT' || error.code === 'RATE_WAIT') &&
    (resource === 'search' || resource === 'core') &&
    error.retryAt != null
  ) {
    return resource;
  }
  return 'global';
}

function isGitHubApiError(e: unknown): e is GitHubApiError {
  return (
    typeof e === 'object' &&
    e !== null &&
    'code' in e &&
    'message' in e &&
    typeof (e as GitHubApiError).code === 'string'
  );
}

function toGitHubError(e: unknown): GitHubApiError {
  if (isGitHubApiError(e)) return e;
  return {
    schemaVersion: '1.0.0',
    code: 'NETWORK_ERROR',
    message: e instanceof Error ? e.message : '未知错误',
    retryAt: null,
    action: 'NONE',
    rateLimit: null,
  };
}

function formatResetTime(epochMs: number | null): string | null {
  if (epochMs == null) return null;
  return new Date(epochMs).toLocaleTimeString('zh-CN');
}

interface GitHubPluginCardProps {
  plugin: DiscoveredPlugin;
  onInstall: (plugin: DiscoveredPlugin) => void;
  installing: Set<string>;
  installDisabled: boolean;
  onOpenTokenPanel: () => void;
}

/** 按声明能力评估风险等级（high/medium/low） */
function getCapabilityRisk(manifest: DiscoveredPlugin['manifest']): { level: 'high' | 'medium' | 'low'; label: string; color: string; reasons: string[] } {
  if (!manifest) {
    return { level: 'high', label: '未知', color: 'danger', reasons: ['无法读取插件清单'] };
  }
  const caps = new Set(manifest.capabilities);
  const reasons: string[] = [];
  if (manifest.runtime === 'binary' || manifest.runtime === 'rust') {
    reasons.push('运行时为原生二进制（可执行任意代码）');
  }
  if (caps.has('fileWrite')) reasons.push('声明 fileWrite：可写入本地文件');
  if (caps.has('network')) reasons.push('声明 network：可访问网络');
  if (caps.has('gpu')) reasons.push('声明 gpu：使用显卡资源');
  if (caps.has('videoFrames') || caps.has('audioSamples')) {
    reasons.push('读取视频帧/音频样本');
  }
  if (reasons.length === 0) reasons.push('仅元数据访问，风险较低');

  const level =
    manifest.runtime === 'binary' || manifest.runtime === 'rust' || caps.has('fileWrite') || caps.has('network')
      ? 'high'
      : caps.has('gpu') || caps.has('videoFrames') || caps.has('audioSamples')
      ? 'medium'
      : 'low';
  const label = level === 'high' ? '高风险' : level === 'medium' ? '中风险' : '低风险';
  const color = level === 'high' ? 'danger' : level === 'medium' ? 'warning' : 'success';
  return { level, label, color, reasons };
}

function isOfficialOwner(owner: DiscoveredPlugin['repository']['owner']): boolean {
  return owner.login.toLowerCase() === 'h2syj';
}

function GitHubPluginCard({ plugin, onInstall, installing, installDisabled, onOpenTokenPanel }: GitHubPluginCardProps) {
  const { repository, manifest, manifestError, isInstalled } = plugin;
  const language = useAppStore((s) => s.settings.language);
  const isInstalling = installing.has(repository.fullName);
  const [expanded, setExpanded] = useState(false);
  const [showConsent, setShowConsent] = useState(false);
  const [consentChecked, setConsentChecked] = useState(false);

  const risk = getCapabilityRisk(manifest);
  const showPrivateRepoHint =
    plugin.verification === 'unverified' && !manifest && !isOfficialOwner(repository.owner);

  const handleInstallClick = () => {
    if (!installDisabled && !isInstalled && !isInstalling && manifest) {
      setConsentChecked(false);
      setShowConsent(true);
    }
  };

  const handleConfirmInstall = () => {
    if (consentChecked && !installDisabled) {
      setShowConsent(false);
      onInstall(plugin);
    }
  };

  const handleToggleExpand = () => {
    setExpanded(!expanded);
  };

  return (
    <Card className="overflow-hidden rounded-2xl border-divider bg-default-50/50">
      <CardHeader className="p-4">
        <div className="flex items-start justify-between gap-3">
          <div className="flex min-w-0 flex-1 items-center gap-3">
            <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary/15 text-primary">
              <Github className="h-5 w-5" aria-hidden="true" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2 flex-wrap">
                <h3 className="truncate font-medium">{repository.name}</h3>
                <Badge className="border border-divider bg-background text-xs">{repository.language || '未知'}</Badge>
                {manifest && (
                  <Badge className="border border-success/40 bg-success/10 text-success text-xs">
                    v{manifest.version}
                  </Badge>
                )}
                <Badge
                  className={cn(
                    'text-xs',
                    risk.level === 'high' && 'border border-danger/40 bg-danger/10 text-danger',
                    risk.level === 'medium' && 'border border-warning/40 bg-warning/10 text-warning',
                    risk.level === 'low' && 'border border-success/40 bg-success/10 text-success'
                  )}
                >
                  {risk.label}
                </Badge>
                {plugin.verification === 'verified' && (
                  <Badge className="border border-primary/40 bg-primary/10 text-primary text-xs">
                    官方
                  </Badge>
                )}
                {plugin.verification === 'unverified' && (
                  <Badge className="border border-warning/40 bg-warning/10 text-warning text-xs">
                    未验证
                  </Badge>
                )}
                {!manifest && manifestError && (
                  <Badge className="border border-warning/40 bg-warning/10 text-warning text-xs">
                    Manifest 错误
                  </Badge>
                )}
                {!manifest && !manifestError && (
                  <Badge className="border border-default/40 bg-default/10 text-default-500 text-xs">
                    无 Manifest
                  </Badge>
                )}
              </div>
              <p className="mt-1 truncate text-xs text-default-400">{repository.fullName}</p>
              {showPrivateRepoHint && (
                <p className="mt-1 text-xs text-default-400">
                  可能为私有仓库，
                  <Button
                    variant="ghost"
                    size="sm"
                    className="h-auto min-w-0 px-0 text-xs text-primary underline underline-offset-2"
                    onClick={onOpenTokenPanel}
                  >
                    配置 Token
                  </Button>
                  {' 后可尝试访问'}
                </p>
              )}
              {repository.description && (
                <p className="mt-1 line-clamp-2 text-sm text-default-500">{repository.description}</p>
              )}
              <div className="mt-2 flex flex-wrap items-center gap-3 text-xs text-default-400">
                <span className="flex items-center gap-1">
                  <Star className="h-3 w-3" aria-hidden="true" />
                  {repository.stargazersCount}
                </span>
                <span className="flex items-center gap-1">
                  <GitFork className="h-3 w-3" aria-hidden="true" />
                  {repository.forksCount}
                </span>
                <span className="flex items-center gap-1">
                  <Info className="h-3 w-3" aria-hidden="true" />
                  更新于 {new Date(repository.updatedAt).toLocaleDateString('zh-CN')}
                </span>
              </div>
            </div>
          </div>
          <div className="flex items-center gap-2 shrink-0">
            {isInstalled ? (
              <Button variant="ghost" size="icon" disabled className="text-success">
                <CheckCircle className="h-4 w-4" aria-hidden="true" />
                <span className="sr-only">已安装</span>
              </Button>
            ) : isInstalling ? (
              <Button variant="primary" size="sm" disabled>
                <Loader2 className="mr-1 h-4 w-4 animate-spin" aria-hidden="true" />
                安装中...
              </Button>
            ) : (
              <Button
                variant="primary"
                size="sm"
                onClick={handleInstallClick}
                disabled={!manifest || installDisabled}
              >
                <Download className="mr-1 h-4 w-4" aria-hidden="true" />
                安装
              </Button>
            )}
            <Button
              variant="ghost"
              size="icon"
              onClick={handleToggleExpand}
              aria-label={expanded ? '收起详情' : '展开详情'}
            >
              {expanded ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
            </Button>
          </div>
        </div>
      </CardHeader>

      {expanded && (
        <CardBody className="px-4 pb-4 pt-0 space-y-3">
          {/* 仓库链接 */}
          <div className="flex items-center gap-2 text-xs">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => window.open(repository.htmlUrl, '_blank')}
            >
              <ExternalLink className="mr-1 h-3 w-3" aria-hidden="true" />
              查看仓库
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => window.open(repository.owner.htmlUrl, '_blank')}
            >
              <Github className="mr-1 h-3 w-3" aria-hidden="true" />
              @{repository.owner.login}
            </Button>
          </div>

          {/* 风险警告 */}
          <div className="rounded-xl border border-warning/30 bg-warning/10 p-3">
            <div className="flex items-start gap-2">
              <AlertTriangle className="mt-0.5 h-4 w-4 shrink-0 text-warning" aria-hidden="true" />
              <div className="text-xs text-warning">
                <p className="font-medium">⚠️ 安全提醒：未验证的第三方插件</p>
                <p className="mt-1">
                  此插件来自 GitHub 社区，<strong>未经过 TimePin 官方审核和验证</strong>。
                  安装并运行第三方插件存在以下风险：
                </p>
                <ul className="mt-1.5 list-disc list-inside space-y-0.5">
                  <li>插件代码可能包含恶意行为（窃取数据、破坏文件等）</li>
                  <li>插件可能不稳定，导致程序崩溃或数据丢失</li>
                  <li>插件可能请求过度权限（网络访问、文件写入、GPU 使用）</li>
                  <li>插件可能不兼容当前版本的 TimePin</li>
                </ul>
                <p className="mt-2">
                  <strong>建议：</strong>仅安装你信任的作者发布的插件，安装前请查看源代码，
                  并在测试环境中验证后再用于重要项目。
                </p>
              </div>
            </div>
          </div>

          {/* Manifest 信息 */}
          {manifest && (
            <div className="space-y-2">
              <h4 className="text-xs font-medium text-default-500 uppercase tracking-wider">插件清单</h4>
              <div className="grid gap-2 text-xs sm:grid-cols-2">
                <div>
                  <span className="text-default-400">协议版本:</span>
                  <span className="ml-1 font-mono">{manifest.protocolVersion}</span>
                </div>
                <div>
                  <span className="text-default-400">运行时:</span>
                  <span className="ml-1 font-mono capitalize">{manifest.runtime}</span>
                </div>
                <div>
                  <span className="text-default-400">入口文件:</span>
                  <span className="ml-1 font-mono">{manifest.entry}</span>
                </div>
                <div>
                  <span className="text-default-400">作者:</span>
                  <span className="ml-1">{manifest.author}</span>
                </div>
              </div>
              <div>
                <span className="text-default-400">声明能力:</span>
                <div className="mt-1 flex flex-wrap gap-1">
                  {manifest.capabilities.map((cap) => (
                    <Badge key={cap} className="border border-divider bg-background">
                      {cap}
                    </Badge>
                  ))}
                </div>
              </div>
              {manifest.parametersSchema && Object.keys(manifest.parametersSchema).length > 0 && (
                <div>
                  <span className="text-default-400">可配置参数:</span>
                  <div className="mt-1 flex flex-wrap gap-1">
                    {Object.entries(manifest.parametersSchema).map(([key, schema]: [string, any]) => (
                      <Badge key={key} className="border border-primary/40 bg-primary/10 text-primary">
                        {resolveLocalizedText(schema.title, language) || key}
                      </Badge>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}

          {/* Manifest 错误 */}
          {manifestError && (
            <div className="rounded-xl border border-danger/30 bg-danger/10 p-3">
              <div className="flex items-start gap-2">
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-danger" aria-hidden="true" />
                <div className="text-xs text-danger">
                  <p className="font-medium">无法加载插件清单</p>
                  <p className="mt-1 font-mono">{manifestError}</p>
                </div>
              </div>
            </div>
          )}

          {/* Topics */}
          {repository.topics.length > 0 && (
            <div>
              <span className="text-xs text-default-400">标签:</span>
              <div className="mt-1 flex flex-wrap gap-1">
                {repository.topics.map((topic) => (
                  <Badge key={topic} className="border border-divider bg-background">
                    {topic}
                  </Badge>
                ))}
              </div>
            </div>
          )}
        </CardBody>
      )}

      {/* 安装同意弹窗 */}
      {showConsent && (
        <InstallConsentModal
          plugin={plugin}
          risk={risk}
          checked={consentChecked}
          disabled={installDisabled}
          onCheckedChange={setConsentChecked}
          onConfirm={handleConfirmInstall}
          onCancel={() => setShowConsent(false)}
        />
      )}
    </Card>
  );
}

interface InstallConsentModalProps {
  plugin: DiscoveredPlugin;
  risk: ReturnType<typeof getCapabilityRisk>;
  checked: boolean;
  disabled: boolean;
  onCheckedChange: (v: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

function InstallConsentModal({ plugin, risk, checked, disabled, onCheckedChange, onConfirm, onCancel }: InstallConsentModalProps) {
  const { repository, manifest } = plugin;
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-4"
      onClick={onCancel}
      role="dialog"
      aria-modal="true"
      aria-label="安装前风险确认"
    >
      <div
        className="w-full max-w-lg rounded-2xl border border-divider bg-background shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="border-b border-divider p-4">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-warning/15 text-warning">
              <ShieldAlert className="h-5 w-5" aria-hidden="true" />
            </div>
            <div className="min-w-0 flex-1">
              <h2 className="text-base font-semibold">安装第三方插件</h2>
              <p className="text-xs text-default-400 truncate">{repository.fullName}</p>
            </div>
            <Button variant="ghost" size="icon" onClick={onCancel} aria-label="关闭">
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>
        <div className="space-y-3 p-4">
          <div
            className={cn(
              'rounded-xl border p-3 text-xs',
              risk.level === 'high' && 'border-danger/30 bg-danger/10 text-danger',
              risk.level === 'medium' && 'border-warning/30 bg-warning/10 text-warning',
              risk.level === 'low' && 'border-success/30 bg-success/10 text-success'
            )}
          >
            <p className="font-medium">风险评估：{risk.label}</p>
            <ul className="mt-1 list-disc list-inside space-y-0.5">
              {risk.reasons.map((r, i) => (
                <li key={i}>{r}</li>
              ))}
            </ul>
          </div>

          <div className="rounded-xl border border-warning/30 bg-warning/10 p-3 text-xs text-warning">
            <p className="font-medium">⚠️ 未经过 TimePin 官方审核</p>
            <p className="mt-1">
              此插件来自 GitHub 社区（{repository.fullName}），TimePin <strong>无法保证其安全性</strong>。
              插件在独立进程中运行，但仍可能：窃取数据、损坏文件、泄露隐私、导致应用崩溃。
            </p>
          </div>

          {manifest && (
            <div className="rounded-xl border border-divider bg-default-50 p-3 text-xs">
              <p className="font-medium text-default-500">声明能力</p>
              <div className="mt-1 flex flex-wrap gap-1">
                {manifest.capabilities.length === 0 ? (
                  <span className="text-default-400">无（仅元数据访问）</span>
                ) : (
                  manifest.capabilities.map((cap) => (
                    <Badge key={cap} className="border border-divider bg-background">
                      {cap}
                    </Badge>
                  ))
                )}
              </div>
              <p className="mt-2 text-default-400">
                运行时：<span className="font-mono">{manifest.runtime}</span> · 入口：
                <span className="font-mono">{manifest.entry}</span>
              </p>
            </div>
          )}

          <Checkbox
            checked={checked}
            onCheckedChange={onCheckedChange}
            aria-label="我已了解上述风险并自行承担插件引起的问题"
            className="items-start"
          >
            <span className="text-sm text-default-600">
              我已了解上述风险，<strong>自行承担</strong>由该插件引起的任何问题。
              我确认已查看或信任 <code className="font-mono">{repository.fullName}</code> 的源代码。
            </span>
          </Checkbox>
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-divider p-4">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            取消
          </Button>
          <Button
            variant={risk.level === 'high' ? 'danger' : 'primary'}
            size="sm"
            onClick={onConfirm}
            disabled={!checked || disabled}
          >
            <Download className="mr-1 h-4 w-4" aria-hidden="true" />
            确认安装
          </Button>
        </div>
      </div>
    </div>
  );
}

/** 配置 Token 面板：受控输入不触发联网；提交才调用 configure_github_token（后端 /user 验证）。 */
interface TokenPanelProps {
  submitting: boolean;
  error: string | null;
  networkDisabled: boolean;
  onSubmit: (candidate: string, remember: boolean) => void;
  onCancel: () => void;
  focusRequest: number;
}

function TokenPanel({ submitting, error, networkDisabled, onSubmit, onCancel, focusRequest }: TokenPanelProps) {
  const [candidate, setCandidate] = useState('');
  const [remember, setRemember] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // 再次点击卡片入口时也应定位到现有面板，不清空正在编辑的候选值。
  useEffect(() => {
    containerRef.current?.scrollIntoView?.({ block: 'nearest' });
    containerRef.current?.querySelector<HTMLInputElement>('input[type="password"]')?.focus();
  }, [focusRequest]);

  const handleSubmit = () => {
    const trimmed = candidate.trim();
    if (!trimmed || submitting || networkDisabled) return;
    onSubmit(trimmed, remember);
  };

  return (
    <Card ref={containerRef} className="rounded-2xl border-divider bg-default-50/50 p-4" role="form" aria-label="配置 GitHub Token">
      <div className="flex flex-col gap-3">
        <div className="flex items-start gap-2">
          <KeyRound className="mt-0.5 h-4 w-4 shrink-0 text-primary" aria-hidden="true" />
          <div className="text-xs text-default-500">
            <p className="font-medium text-foreground">配置 GitHub Personal Access Token</p>
            <p className="mt-1 leading-relaxed">
              推荐使用 <strong>fine-grained token</strong>：仅需公共仓库只读权限，有效期建议 30–90 天。
              若使用 classic token，<strong>无需勾选任何 scope</strong>（也不需要 public_repo）。
              Token 由应用安全保管，不会出现在界面或日志中。
            </p>
          </div>
        </div>
        <Input
          label="Personal Access Token"
          placeholder="输入 token（输入时不会发送任何请求）"
          type="password"
          value={candidate}
          onValueChange={setCandidate}
          isInvalid={!!error}
          errorMessage={error ?? undefined}
          isDisabled={submitting}
          autoComplete="off"
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              handleSubmit();
            }
          }}
        />
        <Checkbox
          checked={remember}
          onCheckedChange={setRemember}
          isDisabled={submitting}
          aria-label="记住此 token"
          labelClassName="text-xs"
        >
          记住此 token（写入系统凭据库）。不勾选时仅保存在内存，退出应用后失效。
        </Checkbox>
        {networkDisabled && (
          <p className="text-xs text-warning" role="status">
            请求已暂停，等待限制解除后才能验证 Token。
          </p>
        )}
        <div className="flex items-center justify-end gap-2">
          <Button variant="ghost" size="sm" onClick={onCancel} disabled={submitting}>
            取消
          </Button>
          <Button size="sm" onClick={handleSubmit} disabled={submitting || networkDisabled || candidate.trim() === ''}>
            {submitting ? (
              <>
                <Loader2 className="mr-1 h-4 w-4 animate-spin" aria-hidden="true" />
                验证中...
              </>
            ) : (
              '验证并保存'
            )}
          </Button>
        </div>
      </div>
    </Card>
  );
}

type PendingRetry =
  | { kind: 'search'; page: number }
  | { kind: 'install'; plugin: DiscoveredPlugin };

type RequestScope = 'search' | 'core' | 'global';

interface RequestBlock {
  scope: RequestScope;
  retryAt: number | null;
  /** 只有明确资源归属的主限流可保留匿名候选校验例外。 */
  allowCandidate: boolean;
}

function isActiveBlock(block: RequestBlock): boolean {
  return block.retryAt == null || block.retryAt > Date.now();
}

function isRateLimitError(error: GitHubApiError): boolean {
  return ['PRIMARY_RATE_LIMIT', 'SECONDARY_RATE_LIMIT', 'RATE_LIMIT_UNKNOWN', 'RATE_WAIT'].includes(error.code);
}

export function GitHubPluginDiscovery() {
  // 在确认当前身份前，不把上一次挂载的凭据派生数据呈现给用户。
  const [plugins, setPlugins] = useState<DiscoveredPlugin[]>([]);
  const [viewingCache, setViewingCache] = useState(false);
  const [installError, setInstallError] = useState<GitHubApiError | null>(null);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(discoveryCache?.page ?? 1);
  const [totalCount, setTotalCount] = useState(0);
  const [rateLimits, setRateLimits] = useState<GitHubRateInfo[]>([]);
  const [session, setSession] = useState<GitHubSession | null>(null);
  const [error, setError] = useState<GitHubApiError | null>(null);
  const [showTokenPanel, setShowTokenPanel] = useState(false);
  const [tokenFocusRequest, setTokenFocusRequest] = useState(0);
  const [tokenSubmitting, setTokenSubmitting] = useState(false);
  const [tokenClearing, setTokenClearing] = useState(false);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [installing, setInstalling] = useState<Set<string>>(new Set());
  const [showLimitBanner, setShowLimitBanner] = useState(false);
  const [requestBlocks, setRequestBlocks] = useState<RequestBlock[]>([]);

  const mountedRef = useRef(true);
  const initialPageRef = useRef(page);
  const generationRef = useRef(0);
  const requestSeqRef = useRef(0);
  const sessionRequestSeqRef = useRef(0);
  const inFlightRef = useRef<{ key: string; promise: Promise<void> } | null>(null);
  const pendingRetryRef = useRef<PendingRetry | null>(null);
  const credentialMutationRef = useRef(false);
  const sessionRef = useRef<GitHubSession | null>(null);
  const requestBlocksRef = useRef<RequestBlock[]>([]);
  const installingRef = useRef(new Set<string>());
  // 本次会话确认安装的结果优先于可能更早拍摄的远端/本地发现快照。
  const installedPathsRef = useRef(installedPluginPaths);
  const rateLimitsRef = useRef<GitHubRateInfo[]>([]);
  const updateRateLimits = useCallback((next: GitHubRateInfo[] | ((prev: GitHubRateInfo[]) => GitHubRateInfo[])) => {
    const resolved = typeof next === 'function' ? next(rateLimitsRef.current) : next;
    rateLimitsRef.current = resolved;
    setRateLimits(resolved);
  }, []);

  const updateRequestBlocks = useCallback((next: RequestBlock[] | ((previous: RequestBlock[]) => RequestBlock[])) => {
    const blocks = typeof next === 'function' ? next(requestBlocksRef.current) : next;
    requestBlocksRef.current = blocks;
    setRequestBlocks(blocks);
  }, []);

  const isBlockedScope = useCallback((scope: 'search' | 'core') => {
    return requestBlocksRef.current.some((block) =>
      isActiveBlock(block) && (block.scope === 'global' || block.scope === scope),
    );
  }, []);

  const canValidateCandidate = () => {
    const active = requestBlocksRef.current.filter(isActiveBlock);
    if (active.some((block) => block.scope === 'global')) return false;
    const core = active.find((block) => block.scope === 'core');
    // /user 不使用 search 桶；core 主限流仍保留新匿名候选身份的例外，但不能绕过全局保护。
    return core == null || (core.allowCandidate &&
      (sessionRef.current?.identity === 'ANONYMOUS' || sessionRef.current?.identity === 'TOKEN_INVALID'));
  };

  const invalidateCredentialRequests = useCallback(() => {
    generationRef.current += 1;
    sessionRequestSeqRef.current += 1;
    requestSeqRef.current += 1;
    inFlightRef.current = null;
    setLoading(false);
    // 身份变更后旧凭据派生的远端数据不可信：丢弃缓存与配额快照，但保留已安装的本地副作用与全局停止。
    discoveryCache = null;
    if (mountedRef.current) {
      setViewingCache(false);
      setPlugins([]);
      setTotalCount(0);
    }
    updateRateLimits([]);
    updateRequestBlocks((blocks) => blocks.filter((block) => block.scope === 'global'));
    setShowLimitBanner(false);
  }, [updateRateLimits, updateRequestBlocks]);

  const applySession = useCallback((next: GitHubSession) => {
    const previousIdentity = sessionRef.current?.identity;
    sessionRef.current = next;
    setSession(next);
    if (previousIdentity == null) {
      if (discoveryCache?.identity === next.identity) {
        setPlugins(discoveryCache.plugins);
        setTotalCount(discoveryCache.totalCount);
        updateRateLimits(mergeRateLimits(discoveryCache.rateLimits, getRateLimits(next)));
      } else {
        discoveryCache = null;
        updateRateLimits(getRateLimits(next));
      }
    } else if (previousIdentity !== next.identity) {
      // 身份/配额快照随身份走：旧身份的快照不再可信。
      updateRateLimits(getRateLimits(next));
      if (discoveryCache && discoveryCache.identity !== next.identity) discoveryCache = null;
    } else {
      updateRateLimits((prev) => mergeRateLimits(prev, getRateLimits(next)));
    }
    if (next.identity === 'TOKEN_INVALID') setShowTokenPanel(true);
  }, [updateRateLimits]);

  const refreshSession = useCallback(async (generation = generationRef.current, failedOperation?: PendingRetry) => {
    if (generation !== generationRef.current) return;
    const seq = ++sessionRequestSeqRef.current;
    try {
      const next = await api.getGitHubSession();
      if (!mountedRef.current || generation !== generationRef.current || seq !== sessionRequestSeqRef.current) return;
      if (sessionRef.current && sessionRef.current.identity !== next.identity) {
        // 后端 401 撤销凭据也是一次身份变更，旧身份的并发结果不再适用。
        invalidateCredentialRequests();
      }
      applySession(next);
      if (next.identity === 'TOKEN_INVALID' && failedOperation) {
        pendingRetryRef.current = failedOperation;
      }
    } catch {
      // 查询失败不阻塞浏览；不允许旧查询覆盖更新的会话。
    }
  }, [applySession, invalidateCredentialRequests]);

  const recordFailure = useCallback((apiError: GitHubApiError, operation?: PendingRetry, candidateValidation = false) => {
    if (!candidateValidation && apiError.code === 'AUTH_INVALID') {
      // 活跃凭据被 401 撤销：立即使所有旧请求/缓存失效，不等待后续会话查询。
      invalidateCredentialRequests();
    }
    const rate = apiError.rateLimit;
    if (rate && !candidateValidation) updateRateLimits((prev) => mergeRateLimits(prev, [rate]));
    const scope = rateErrorScope(apiError);
    const primaryLimit = !candidateValidation && apiError.code === 'PRIMARY_RATE_LIMIT' && scope !== 'global';
    const anonymousPrimary = primaryLimit &&
      (sessionRef.current?.identity === 'ANONYMOUS' || sessionRef.current?.identity === 'TOKEN_INVALID');
    if (isRateLimitError(apiError) || (apiError.retryAt != null && apiError.retryAt > Date.now())) {
      updateRequestBlocks((blocks) => {
        const active = blocks.filter(isActiveBlock);
        const previous = active.find((block) => block.scope === scope);
        // 不同资源的等待独立到期；同资源并发错误不能缩短等待，全局停止也不会被资源错误覆盖。
        return [...active.filter((block) => block.scope !== scope), {
          scope,
          retryAt: previous
            ? previous.retryAt == null || apiError.retryAt == null ? null : Math.max(previous.retryAt, apiError.retryAt)
            : apiError.retryAt,
          allowCandidate: primaryLimit && (previous?.allowCandidate ?? true),
        }];
      });
    }
    if (operation && (anonymousPrimary || apiError.code === 'AUTH_INVALID')) pendingRetryRef.current = operation;
    if (apiError.code === 'AUTH_INVALID') setShowTokenPanel(true);
    if (anonymousPrimary && !primaryLimitBannerAcked) {
      primaryLimitBannerAcked = true;
      setShowLimitBanner(true);
    }
  }, [invalidateCredentialRequests, updateRateLimits, updateRequestBlocks]);

  const applyResult = useCallback((targetPage: number, result: DiscoverPluginsResult) => {
    const merged = result.plugins.map((plugin) => {
      const path = installedPathsRef.current.get(plugin.repository.fullName);
      return path ? { ...plugin, isInstalled: true, localPath: path } : plugin;
    });
    setPlugins(merged);
    setTotalCount(result.totalCount);
    const incoming = getRateLimits(result);
    updateRateLimits((prev) => mergeRateLimits(prev, incoming));
    const snapshot = mergeRateLimits(rateLimitsRef.current, incoming);
    setViewingCache(false);
    discoveryCache = {
      page: targetPage,
      plugins: merged,
      totalCount: result.totalCount,
      identity: sessionRef.current?.identity ?? 'ANONYMOUS',
      rateLimits: snapshot,
    };
  }, [updateRateLimits]);

  const loadPlugins = useCallback(async (targetPage: number) => {
    if (isBlockedScope('search') || credentialMutationRef.current) return;
    const generation = generationRef.current;
    // StrictMode 同代次去重；新凭据永远不能复用旧匿名 Promise。
    const key = `discover:${generation}:${targetPage}`;
    if (inFlightRef.current?.key === key) return inFlightRef.current.promise;
    const seq = ++requestSeqRef.current;
    const promise = (async () => {
      setPage(targetPage);
      setViewingCache(false);
      setLoading(true);
      setError(null);
      let failedOperation: PendingRetry | undefined;
      try {
        const result = await api.discoverGitHubPlugins(targetPage, 20);
        if (!mountedRef.current || generation !== generationRef.current || seq !== requestSeqRef.current) return;
        applyResult(targetPage, result);
      } catch (e) {
        if (!mountedRef.current || generation !== generationRef.current || seq !== requestSeqRef.current) return;
        const apiError = toGitHubError(e);
        failedOperation = { kind: 'search', page: targetPage };
        setError(apiError);
        recordFailure(apiError, failedOperation);
        if (apiError.code === 'AUTH_INVALID') void refreshSession(generationRef.current, failedOperation);
      } finally {
        if (inFlightRef.current?.key === key) inFlightRef.current = null;
        if (mountedRef.current && generation === generationRef.current && seq === requestSeqRef.current) {
          setLoading(false);
          void refreshSession(generation, failedOperation);
        }
      }
    })();
    inFlightRef.current = { key, promise };
    return promise;
  }, [applyResult, isBlockedScope, recordFailure, refreshSession]);

  useEffect(() => {
    mountedRef.current = true;
    localStorage.removeItem('github_token'); // 只删除、不迁移、不发送旧凭据。
    let disposed = false;
    // 先确认身份，再决定是否可以提供匿名主限流恢复。
    const generation = generationRef.current;
    void refreshSession(generation).then(() => {
      if (!disposed && generation === generationRef.current) void loadPlugins(initialPageRef.current);
    });
    return () => {
      disposed = true;
      mountedRef.current = false;
      sessionRequestSeqRef.current += 1;
    };
  }, [loadPlugins, refreshSession]);

  useEffect(() => {
    const deadlines = requestBlocks.flatMap((block) => block.retryAt == null ? [] : [block.retryAt]);
    if (deadlines.length === 0) return; // 无可靠时间的停止状态保持原样，不猜测恢复时间。
    const timer = setTimeout(() => {
      updateRequestBlocks((blocks) => blocks.filter(isActiveBlock));
    }, Math.max(0, Math.min(...deadlines) - Date.now()));
    return () => clearTimeout(timer);
  }, [requestBlocks, updateRequestBlocks]);

  const handleInstall = async (plugin: DiscoveredPlugin, retry = false) => {
    const name = plugin.repository.fullName;
    if (isBlockedScope('core') || credentialMutationRef.current || installingRef.current.has(name) || installedPathsRef.current.has(name)) return;
    const generation = generationRef.current;
    installingRef.current.add(name);
    setInstalling(new Set(installingRef.current));
    let failedOperation: PendingRetry | undefined;
    try {
      const result = await api.installGitHubPlugin({
        owner: plugin.repository.owner.login,
        repo: plugin.repository.name,
        branch: plugin.repository.defaultBranch,
      });
      // 安装的本地副作用即使跨身份或组件卸载发生也必须保留。
      installedPathsRef.current.set(name, result);
      if (!mountedRef.current) return;
      const merge = (items: DiscoveredPlugin[]) => items.map((item) =>
        item.repository.fullName === name ? { ...item, isInstalled: true, localPath: result } : item,
      );
      setPlugins(merge);
      if (discoveryCache) discoveryCache = { ...discoveryCache, plugins: merge(discoveryCache.plugins) };
      if (generation === generationRef.current) setInstallError(null);
    } catch (e) {
      if (!mountedRef.current || generation !== generationRef.current) return;
      const apiError = toGitHubError(e);
      failedOperation = retry ? undefined : { kind: 'install', plugin };
      setInstallError(apiError);
      recordFailure(apiError, failedOperation);
      if (apiError.code === 'AUTH_INVALID') void refreshSession(generationRef.current, failedOperation);
    } finally {
      installingRef.current.delete(name);
      if (mountedRef.current) {
        setInstalling(new Set(installingRef.current));
        void refreshSession(generation, failedOperation);
      }
    }
  };

  const handleTokenSubmit = async (candidate: string, remember: boolean) => {
    // 同步防双击，不使用会静默吞掉修正后提交的任意时长冷却。
    if (credentialMutationRef.current || !canValidateCandidate()) return;
    credentialMutationRef.current = true;
    sessionRequestSeqRef.current += 1;
    setTokenSubmitting(true);
    setTokenError(null);
    try {
      const next = await api.configureGitHubToken(candidate, remember);
      // 后端身份已改变，即使面板在校验期间卸载，也不能留下旧凭据派生的模块缓存。
      discoveryCache = null;
      if (!mountedRef.current) return;
      invalidateCredentialRequests();
      applySession(next);
      setShowTokenPanel(false);
      setShowLimitBanner(false);
      setError(null);
      setInstallError(null);
      // invalidateCredentialRequests 已移除旧身份的资源等待，但保留并发到达的全局停止。
      const pending = pendingRetryRef.current;
      pendingRetryRef.current = null; // 消费原上下文，重试失败不重新武装安装。
      credentialMutationRef.current = false;
      if (pending?.kind === 'install') {
        void handleInstall(pending.plugin, true);
      } else {
        void loadPlugins(pending?.page ?? page);
      }
    } catch (e) {
      if (!mountedRef.current) return;
      const apiError = toGitHubError(e);
      setTokenError(apiError.message);
      recordFailure(apiError, undefined, true);
    } finally {
      credentialMutationRef.current = false;
      if (mountedRef.current) setTokenSubmitting(false);
    }
  };

  const handleTokenPanelCancel = () => {
    setShowTokenPanel(false);
    setTokenError(null);
    pendingRetryRef.current = null;
  };

  const handleClearToken = async () => {
    if (credentialMutationRef.current) return;
    credentialMutationRef.current = true;
    sessionRequestSeqRef.current += 1;
    setTokenClearing(true);
    try {
      const next = await api.clearGitHubToken();
      discoveryCache = null;
      if (!mountedRef.current) return;
      invalidateCredentialRequests();
      applySession(next);
      pendingRetryRef.current = null;
      credentialMutationRef.current = false;
      // 清除凭据不是限流旁路；若当前可联网则用新代次刷新。
      void loadPlugins(page);
    } catch (e) {
      if (mountedRef.current) setTokenError(toGitHubError(e).message);
    } finally {
      credentialMutationRef.current = false;
      if (mountedRef.current) setTokenClearing(false);
    }
  };

  const handleRefresh = () => { void loadPlugins(page); };
  const handlePageChange = (targetPage: number) => {
    if (loading || viewingCache || isBlockedScope('search') || credentialMutationRef.current) return;
    void loadPlugins(targetPage);
  };

  const openTokenPanelWithFocus = useCallback(() => {
    setTokenError(null);
    setShowTokenPanel(true);
    setTokenFocusRequest((request) => request + 1);
  }, []);

  const identity = session?.identity ?? 'ANONYMOUS';
  const isAnonymous = identity === 'ANONYMOUS';
  // 搜索入口的温和预警使用 search 快照，不让后续 manifest 的 core 响应覆盖它。
  const searchRateSnapshot = rateLimits.find((rate) => rate.resource === 'search')
    ?? rateLimits.find((rate) => rate.resource == null)
    ?? null;
  const softQuotaWarning = isAnonymous && searchRateSnapshot?.remaining != null && searchRateSnapshot.remaining < 10;
  const searchRateLimited = error != null && isRateLimitError(error);
  const searchBlocked = isBlockedScope('search');
  const installBlocked = isBlockedScope('core');
  const activeBlocks = requestBlocks.filter(isActiveBlock);
  const searchBlock = activeBlocks.find((block) => block.scope === 'global')
    ?? activeBlocks.find((block) => block.scope === 'search');
  const refreshDisabled = loading || searchBlocked || tokenSubmitting || tokenClearing;
  const installDisabled = installBlocked || tokenSubmitting || tokenClearing;
  const tokenValidationDisabled = tokenClearing || !canValidateCandidate();
  const retryBlockedUntil = searchBlock?.retryAt ?? null;
  const primaryError = [error, installError].find((failure) => failure?.code === 'PRIMARY_RATE_LIMIT');

  return (
    <div className="flex h-full flex-col gap-4 overflow-auto px-8 pb-8 pt-6">
      {/* 顶部工具栏 */}
      <div className="flex items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-3">
            <h1 className="text-2xl font-semibold tracking-tight">发现插件</h1>
            <Badge className="border border-primary/40 bg-primary/10 text-primary">
              GitHub (topic: time-pin-plugins)
            </Badge>
            <Badge
              className={cn(
                'text-xs',
                identity === 'AUTHENTICATED' && 'border border-success/40 bg-success/10 text-success',
                identity === 'ANONYMOUS' && 'border border-divider bg-background text-default-500',
                identity === 'TOKEN_INVALID' && 'border border-danger/40 bg-danger/10 text-danger'
              )}
            >
              {identity === 'AUTHENTICATED' && '已认证'}
              {identity === 'ANONYMOUS' && '匿名'}
              {identity === 'TOKEN_INVALID' && 'Token 已失效'}
            </Badge>
          </div>
          <p className="mt-1.5 text-sm text-default-500">
            搜索带有 <code className="rounded border border-divider bg-background px-1 font-mono">time-pin-plugins</code> 标签的 GitHub 仓库，
            一键安装社区插件。
          </p>
        </div>
        <div className="flex items-center gap-2">
          {identity === 'AUTHENTICATED' ? (
            <Button variant="secondary" size="sm" onClick={handleClearToken} disabled={tokenSubmitting || tokenClearing}>
              <KeyRound className="mr-1 h-4 w-4" aria-hidden="true" />
              清除 Token
            </Button>
          ) : (
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setTokenError(null);
                setShowTokenPanel((v) => !v);
              }}
            >
              <KeyRound className="mr-1 h-4 w-4" aria-hidden="true" />
              {identity === 'TOKEN_INVALID' ? '重新配置 Token' : '配置 Token'}
            </Button>
          )}
          <Button
            variant="secondary"
            onClick={handleRefresh}
            disabled={refreshDisabled}
            aria-label={retryBlockedUntil != null ? `请求受限，约 ${formatResetTime(retryBlockedUntil) ?? ''} 后可重试` : '刷新'}
          >
            <Loader2 className={cn('h-4 w-4', loading && 'animate-spin')} aria-hidden="true" />
            刷新
          </Button>
        </div>
      </div>

      {/* Token 配置面板（渐进披露） */}
      {showTokenPanel && (
        <TokenPanel
          focusRequest={tokenFocusRequest}
          submitting={tokenSubmitting}
          error={tokenError}
          networkDisabled={tokenValidationDisabled}
          onSubmit={handleTokenSubmit}
          onCancel={handleTokenPanelCancel}
        />
      )}
      {session?.storageWarning && (
        <p className="text-xs text-warning" role="status">
          凭据存储警告：{session.storageWarning}
        </p>
      )}
      {!showTokenPanel && tokenError && <p className="text-xs text-danger" role="alert">{tokenError}</p>}
      {activeBlocks.map((block) => (
        <p key={block.scope} className="text-xs text-warning" role="status">
          {block.scope === 'global' ? '请求已暂停。' : `${resourceLabel(block.scope)} 请求已暂停。`}
          {block.retryAt != null
            ? `请等待至 ${formatResetTime(block.retryAt)} 后手动重试。`
            : '服务未提供可靠的恢复时间。请按错误提示操作；若本次会话已停止，请稍后重启应用。'}
        </p>
      ))}

      {/* 匿名主限流：会话单次横幅（含校准后的 reset 时间） */}
      {showLimitBanner && (
        <Card className="rounded-2xl border-warning/30 bg-warning/10 p-4" role="alert" aria-label="GitHub API 已达上限">
          <div className="flex items-start gap-3">
            <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-warning" aria-hidden="true" />
            <div className="min-w-0 flex-1 text-sm text-warning">
              <p className="font-medium">GitHub API 请求次数已达上限</p>
              <p className="mt-1 text-xs">
                {primaryError?.rateLimit?.resetAt != null
                  ? `预计 ${formatResetTime(primaryError.rateLimit.resetAt)} 后重置。`
                  : '请稍后重试。'}
                你可以配置 Token 提高限额、稍后等解禁后手动刷新，或查看当前会话中已加载的缓存结果。
              </p>
              <div className="mt-3 flex flex-wrap gap-2">
                <Button
                  size="sm"
                  onClick={() => {
                    setShowTokenPanel(true);
                    setShowLimitBanner(false);
                    primaryLimitBannerAcked = true;
                  }}
                >
                  配置 Token
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => {
                    setShowLimitBanner(false);
                    primaryLimitBannerAcked = true;
                  }}
                >
                  稍后
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={discoveryCache == null}
                  onClick={() => {
                    setShowLimitBanner(false);
                    primaryLimitBannerAcked = true;
                    if (discoveryCache) {
                      setPlugins(discoveryCache.plugins);
                      setTotalCount(discoveryCache.totalCount);
                      setPage(discoveryCache.page);
                      setViewingCache(true);
                    }
                  }}
                >
                  查看缓存
                </Button>
              </div>
            </div>
            <Button
              variant="ghost"
              size="icon"
              aria-label="关闭限流提示"
              onClick={() => {
                setShowLimitBanner(false);
                primaryLimitBannerAcked = true;
              }}
            >
              <X className="h-4 w-4" />
            </Button>
          </div>
        </Card>
      )}

      {/* 速率限制信息（仅展示响应头数据，不硬编码配额口径）；各资源桶独立显示。 */}
      {rateLimits.some((rate) => rate.remaining != null || rate.resetAt != null) && (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-default-500">
          <Info className="h-3 w-3" aria-hidden="true" />
          {rateLimits
            .filter((rate) => rate.remaining != null || rate.resetAt != null)
            .map((rate) => (
              <span key={rate.resource ?? 'unknown'} className="flex items-center gap-1">
                {rate.remaining != null && (
                  <span>
                    {resourceLabel(rate.resource)} 剩余: {rate.remaining}
                    {rate.limit != null ? `/${rate.limit}` : ''}
                  </span>
                )}
                {rate.resetAt != null && (
                  <span className="text-default-400">
                    {rate.remaining != null ? '· ' : ''}重置于: {formatResetTime(rate.resetAt)}
                  </span>
                )}
              </span>
            ))}
          {isAnonymous && <span>匿名访问限额较低，配置 Token 可提升</span>}
        </div>
      )}

      {/* 匿名低剩余量软警告 */}
      {softQuotaWarning && !searchRateLimited && (
        <div className="flex items-center gap-2 text-xs text-warning" role="status">
          <AlertTriangle className="h-3 w-3" aria-hidden="true" />
          <span>匿名 API 额度即将用尽（剩余 {searchRateSnapshot?.remaining}），建议配置 Token。</span>
        </div>
      )}

      {/* Token 失效引导（含匿名回退成功的场景） */}
      {identity === 'TOKEN_INVALID' && (
        <Card className="rounded-2xl border-warning/30 bg-warning/10 p-4" role="alert" aria-label="Token 已失效">
          <div className="flex items-center gap-2 text-warning">
            <AlertTriangle className="h-4 w-4 shrink-0" aria-hidden="true" />
            <span className="text-sm">
              已保存的 GitHub Token 已失效（当前按匿名访问）。
              <button
                type="button"
                className="ml-1 underline underline-offset-2"
                onClick={() => setShowTokenPanel(true)}
              >
                重新配置 Token
              </button>
            </span>
          </div>
        </Card>
      )}

      {/* 错误提示 */}
      {error && (
        <Card className="rounded-2xl border-danger/30 bg-danger/10 p-4" role="alert" aria-label="搜索失败">
          <div className="flex items-start gap-2 text-danger">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <span className="text-sm">{error.message}</span>
              {error.code === 'NOT_FOUND_OR_PRIVATE' && (
                <p className="mt-1 text-xs">
                  仓库不存在或无权访问。若为私有仓库，可
                  <button
                    type="button"
                    className="mx-1 underline underline-offset-2"
                    onClick={() => setShowTokenPanel(true)}
                  >
                    配置 Token
                  </button>
                  后重试。
                </p>
              )}
              {(error.code === 'SECONDARY_RATE_LIMIT' || error.code === 'RATE_LIMIT_UNKNOWN' || error.code === 'RATE_WAIT') && (
                <p className="mt-1 text-xs">
                  请求被暂时限制，请等待{formatResetTime(error.retryAt) ? `至 ${formatResetTime(error.retryAt)} ` : '片刻'}后重试。
                </p>
              )}
              {error.code === 'AUTH_INVALID' && (
                <p className="mt-1 text-xs">
                  Token 验证失败，请
                  <button
                    type="button"
                    className="mx-1 underline underline-offset-2"
                    onClick={() => setShowTokenPanel(true)}
                  >
                    重新配置 Token
                  </button>
                  。
                </p>
              )}
            </div>
          </div>
        </Card>
      )}
      {installError && (
        <Card className="rounded-2xl border-danger/30 bg-danger/10 p-4" role="alert" aria-label="安装失败">
          <div className="flex items-start gap-2 text-danger">
            <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
            <div className="min-w-0 flex-1">
              <span className="text-sm">安装失败: {installError.message}</span>
              {installError.code === 'NOT_FOUND_OR_PRIVATE' && (
                <p className="mt-1 text-xs">仓库不存在或无权访问；若为私有仓库可配置 Token 后重试。</p>
              )}
            </div>
          </div>
        </Card>
      )}

      {/* 查看缓存提示 */}
      {viewingCache && (
        <div className="flex items-center gap-2 text-xs text-default-500" role="status">
          <Info className="h-3 w-3" aria-hidden="true" />
          <span>正在显示本次会话已加载的结果（内存缓存，未落盘）。点击「刷新」可在限额恢复后重新获取。</span>
        </div>
      )}

      {/* 插件列表 */}
      <div className="flex-1 overflow-auto space-y-3">
        {loading && plugins.length === 0 ? (
          <div className="flex items-center justify-center py-16 text-sm text-default-400">
            <Loader2 className="mr-2 h-4 w-4 animate-spin" /> 正在搜索 GitHub 仓库…
          </div>
        ) : plugins.length === 0 ? (
          <Card className="flex flex-col items-center justify-center gap-3 rounded-2xl border-divider bg-default-50/50 p-10 text-center">
            <div className="flex h-14 w-14 items-center justify-center rounded-2xl bg-primary/15 text-primary">
              <Github className="h-7 w-7" />
            </div>
            <div>
              <p className="text-sm font-medium">未发现插件</p>
              <p className="mt-1 text-xs leading-relaxed text-default-400">
                没有找到带有 <code className="rounded border border-divider bg-background px-1 font-mono">time-pin-plugins</code> 标签的公开仓库。
                如果你是插件作者，请在仓库设置中添加此标签。
              </p>
            </div>
          </Card>
        ) : (
          plugins.map((plugin) => (
            <GitHubPluginCard
              key={plugin.repository.fullName}
              plugin={plugin}
              onInstall={handleInstall}
              installDisabled={installDisabled}
              installing={installing}
              onOpenTokenPanel={openTokenPanelWithFocus}
            />
          ))
        )}

        {/* 分页 */}
        {totalCount > 20 && (
          <div className="flex items-center justify-center gap-2 pt-4">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => handlePageChange(Math.max(1, page - 1))}
              disabled={page <= 1 || loading || refreshDisabled || viewingCache}
            >
              上一页
            </Button>
            <span className="text-sm text-default-500">
              第 {page} 页 / 共 {Math.ceil(totalCount / 20)} 页
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => handlePageChange(page + 1)}
              disabled={page * 20 >= totalCount || loading || refreshDisabled || viewingCache}
            >
              下一页
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}
