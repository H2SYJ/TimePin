import { AlertTriangle, CheckCircle, ChevronDown, ChevronUp, Download, ExternalLink, Github, Loader2, Star, X, AlertCircle, Info, GitFork, ShieldAlert } from 'lucide-react';
import { useCallback, useEffect, useState } from 'react';
import { Button, Card, CardBody, CardHeader, Input, Badge } from '@/components/ui';
import { api } from '@/lib/ipc';
import { cn } from '@/lib/utils';
import type { DiscoveredPlugin, DiscoverPluginsResult } from '@/types';

interface GitHubPluginCardProps {
  plugin: DiscoveredPlugin;
  onInstall: (plugin: DiscoveredPlugin) => void;
  installing: Set<string>;
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

function GitHubPluginCard({ plugin, onInstall, installing }: GitHubPluginCardProps) {
  const { repository, manifest, manifestError, isInstalled } = plugin;
  const isInstalling = installing.has(repository.fullName);
  const [expanded, setExpanded] = useState(false);
  const [showConsent, setShowConsent] = useState(false);
  const [consentChecked, setConsentChecked] = useState(false);

  const risk = getCapabilityRisk(manifest);

  const handleInstallClick = () => {
    if (!isInstalled && !isInstalling && manifest) {
      setConsentChecked(false);
      setShowConsent(true);
    }
  };

  const handleConfirmInstall = () => {
    if (consentChecked) {
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
                disabled={!manifest}
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
                        {schema.title || key}
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
  onCheckedChange: (v: boolean) => void;
  onConfirm: () => void;
  onCancel: () => void;
}

function InstallConsentModal({ plugin, risk, checked, onCheckedChange, onConfirm, onCancel }: InstallConsentModalProps) {
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

          <label className="flex items-start gap-2 cursor-pointer text-sm">
            <input
              type="checkbox"
              checked={checked}
              onChange={(e) => onCheckedChange(e.target.checked)}
              className="mt-0.5 h-4 w-4 shrink-0"
            />
            <span className="text-default-600">
              我已了解上述风险，<strong>自行承担</strong>由该插件引起的任何问题。
              我确认已查看或信任 <code className="font-mono">{repository.fullName}</code> 的源代码。
            </span>
          </label>
        </div>
        <div className="flex items-center justify-end gap-2 border-t border-divider p-4">
          <Button variant="ghost" size="sm" onClick={onCancel}>
            取消
          </Button>
          <Button
            variant={risk.level === 'high' ? 'danger' : 'primary'}
            size="sm"
            onClick={onConfirm}
            disabled={!checked}
          >
            <Download className="mr-1 h-4 w-4" aria-hidden="true" />
            确认安装
          </Button>
        </div>
      </div>
    </div>
  );
}

export function GitHubPluginDiscovery() {
  const [plugins, setPlugins] = useState<DiscoveredPlugin[]>([]);
  const [installError, setInstallError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(1);
  const [totalCount, setTotalCount] = useState(0);
  const [rateLimitRemaining, setRateLimitRemaining] = useState<number | null>(null);
  const [rateLimitReset, setRateLimitReset] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [githubToken, setGithubToken] = useState('');
  const [showTokenInput, setShowTokenInput] = useState(false);
  const [installing, setInstalling] = useState<Set<string>>(new Set());

  const loadPlugins = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const result: DiscoverPluginsResult = await api.discoverGitHubPlugins(page, 20, githubToken || undefined);
      setPlugins(result.plugins);
      setTotalCount(result.totalCount);
      setRateLimitRemaining(result.rateLimitRemaining);
      setRateLimitReset(result.rateLimitReset);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [page, githubToken]);

  useEffect(() => {
    loadPlugins();
  }, [loadPlugins]);

  const handleInstall = async (plugin: DiscoveredPlugin) => {
    const newInstalling = new Set(installing);
    newInstalling.add(plugin.repository.fullName);
    setInstalling(newInstalling);

    try {
      const result = await api.installGitHubPlugin({
        owner: plugin.repository.owner.login,
        repo: plugin.repository.name,
        branch: plugin.repository.defaultBranch,
        githubToken: githubToken || undefined,
      });
      // 更新本地状态标记为已安装
      setPlugins((prev) =>
        prev.map((p) =>
          p.repository.fullName === plugin.repository.fullName
            ? { ...p, isInstalled: true, localPath: result }
            : p
        )
      );
      setInstallError(null);
    } catch (e) {
      setInstallError(e instanceof Error ? e.message : String(e));
    } finally {
      const newInstalling = new Set(installing);
      newInstalling.delete(plugin.repository.fullName);
      setInstalling(newInstalling);
    }
  };

  const handleTokenSave = () => {
    localStorage.setItem('github_token', githubToken);
    loadPlugins();
  };

  // 启动时恢复 token
  useEffect(() => {
    const saved = localStorage.getItem('github_token');
    if (saved) setGithubToken(saved);
  }, []);

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
          </div>
          <p className="mt-1.5 text-sm text-default-500">
            搜索带有 <code className="rounded border border-divider bg-background px-1 font-mono">time-pin-plugins</code> 标签的 GitHub 仓库，
            一键安装社区插件。
          </p>
        </div>
        <div className="flex items-center gap-2">
          {showTokenInput ? (
            <div className="flex items-center gap-2">
              <Input
                placeholder="GitHub Personal Access Token (可选，提高速率限制)"
                type="password"
                value={githubToken}
                onChange={(e) => setGithubToken(e.target.value)}
                className="w-64"
              />
              <Button size="sm" onClick={handleTokenSave}>
                <CheckCircle className="mr-1 h-4 w-4" />
                保存
              </Button>
              <Button variant="ghost" size="icon" onClick={() => setShowTokenInput(false)}>
                <X className="h-4 w-4" />
              </Button>
            </div>
          ) : (
            <Button variant="secondary" size="sm" onClick={() => setShowTokenInput(true)}>
              <Info className="mr-1 h-4 w-4" />
              设置 Token
            </Button>
          )}
          <Button variant="secondary" onClick={loadPlugins} disabled={loading}>
            <Loader2 className={cn('h-4 w-4', loading && 'animate-spin')} aria-hidden="true" />
            刷新
          </Button>
        </div>
      </div>

      {/* 速率限制提示 */}
      {rateLimitRemaining !== null && (
        <div className="flex items-center gap-2 text-xs text-default-500">
          <Info className="h-3 w-3" aria-hidden="true" />
          <span>API 剩余调用: {rateLimitRemaining}</span>
          {rateLimitReset && (
            <>
              <span>·</span>
              <span>重置于: {new Date(rateLimitReset * 1000).toLocaleTimeString('zh-CN')}</span>
            </>
          )}
          <span>·</span>
          <span>未登录仅 60 次/小时，配置 Token 可提升到 5000 次/小时</span>
        </div>
      )}

      {/* 错误提示 */}
      {error && (
        <Card className="rounded-2xl border-danger/30 bg-danger/10 p-4">
          <div className="flex items-center gap-2 text-danger">
            <AlertCircle className="h-4 w-4" aria-hidden="true" />
            <span className="text-sm">{error}</span>
          </div>
        </Card>
      )}
      {installError && (
        <Card className="rounded-2xl border-danger/30 bg-danger/10 p-4">
          <div className="flex items-center gap-2 text-danger">
            <AlertCircle className="h-4 w-4" aria-hidden="true" />
            <span className="text-sm">安装失败: {installError}</span>
          </div>
        </Card>
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
              installing={installing}
            />
          ))
        )}

        {/* 分页 */}
        {totalCount > 20 && (
          <div className="flex items-center justify-center gap-2 pt-4">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPage((p) => Math.max(1, p - 1))}
              disabled={page <= 1 || loading}
            >
              上一页
            </Button>
            <span className="text-sm text-default-500">
              第 {page} 页 / 共 {Math.ceil(totalCount / 20)} 页
            </span>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => setPage((p) => p + 1)}
              disabled={page * 20 >= totalCount || loading}
            >
              下一页
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}