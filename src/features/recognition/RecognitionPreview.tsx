import { useCallback, useEffect, useRef, useState } from 'react';
import { Play, Pause, SkipBack, SkipForward, RotateCcw, Expand, X, Eye, EyeOff } from 'lucide-react';
import { Button, Card, CardBody, CardHeader, Label, Select, SelectItem } from '@/components/ui';
import { cn } from '@/lib/utils';

interface PreviewFrame {
  frameIndex: number;
  timestamp: number;
  imageBase64: string;
  width: number;
  height: number;
}

interface Overlay {
  type: 'rect' | 'text' | 'point';
  rect?: { x: number; y: number; width: number; height: number };
  text?: string;
  point?: { x: number; y: number };
  color?: string;
  label?: string;
  confidence?: number;
}

interface RecognitionPreviewProps {
  /** 当前预览帧 */
  frame?: PreviewFrame;
  /** 历史帧列表（用于逐帧播放） */
  frames?: PreviewFrame[];
  /** 叠加层 */
  overlays?: Overlay[];
  /** 是否正在播放 */
  isPlaying?: boolean;
  /** 播放速度 */
  playbackSpeed?: number;
  /** 当前帧索引 */
  currentFrameIndex?: number;
  /** 回调：帧索引变化 */
  onFrameIndexChange?: (index: number) => void;
  /** 回调：播放/暂停 */
  onPlayPause?: () => void;
  /** 回调：上一帧 */
  onPrevFrame?: () => void;
  /** 回调：下一帧 */
  onNextFrame?: () => void;
  /** 回调：速度变化 */
  onSpeedChange?: (speed: number) => void;
  /** 显示控制栏 */
  showControls?: boolean;
  /** 最大宽度 */
  maxWidth?: number;
}

export function RecognitionPreview({
  frame,
  frames = [],
  overlays = [],
  isPlaying = false,
  playbackSpeed = 1,
  currentFrameIndex = 0,
  onFrameIndexChange,
  onPlayPause,
  onPrevFrame,
  onNextFrame,
  onSpeedChange,
  showControls = true,
  maxWidth = 800,
}: RecognitionPreviewProps) {
  const imageRef = useRef<HTMLImageElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [displayWidth, setDisplayWidth] = useState(0);
  const [displayHeight, setDisplayHeight] = useState(0);
  const [showOverlays, setShowOverlays] = useState(true);
  const [showFullscreen, setShowFullscreen] = useState(false);

  // 计算显示尺寸
  useEffect(() => {
    if (!frame || !imageRef.current) return;
    const img = imageRef.current;
    if (img.naturalWidth === 0) return;

    const aspectRatio = img.naturalWidth / img.naturalHeight;
    let w = Math.min(img.naturalWidth, maxWidth);
    let h = w / aspectRatio;

    const maxHeight = window.innerHeight * 0.7;
    if (h > maxHeight) {
      h = maxHeight;
      w = h * aspectRatio;
    }

    setDisplayWidth(w);
    setDisplayHeight(h);
  }, [frame, maxWidth]);

  // 归一化 -> 像素
  const normToPx = useCallback(
    (rect: { x: number; y: number; width: number; height: number }) => ({
      x: rect.x * displayWidth,
      y: rect.y * displayHeight,
      width: rect.width * displayWidth,
      height: rect.height * displayHeight,
    }),
    [displayWidth, displayHeight]
  );

  // 渲染叠加层
  const renderOverlays = useCallback(() => {
    if (!showOverlays || displayWidth === 0) return null;

    return overlays.map((overlay, idx) => {
      if (overlay.type === 'rect' && overlay.rect) {
        const px = normToPx(overlay.rect);
        const color = overlay.color || '#ef4444';
        return (
          <div
            key={`overlay-${idx}`}
            className="absolute border-2 pointer-events-none"
            style={{
              left: px.x,
              top: px.y,
              width: px.width,
              height: px.height,
              borderColor: color,
              backgroundColor: `${color}15`,
            }}
          >
            {overlay.label && (
              <div
                className="absolute -top-6 left-0 text-xs font-medium px-1 rounded whitespace-nowrap"
                style={{ color, backgroundColor: `${color}CC` }}
              >
                {overlay.label}
                {overlay.confidence !== undefined && ` ${(overlay.confidence * 100).toFixed(0)}%`}
              </div>
            )}
          </div>
        );
      } else if (overlay.type === 'text' && overlay.rect && overlay.text) {
        const px = normToPx(overlay.rect);
        return (
          <div
            key={`overlay-text-${idx}`}
            className="absolute text-xs font-medium pointer-events-none whitespace-nowrap"
            style={{
              left: px.x,
              top: px.y - 20,
              color: overlay.color || '#fff',
              textShadow: '0 0 3px #000',
            }}
          >
            {overlay.text}
          </div>
        );
      } else if (overlay.type === 'point' && overlay.point) {
        return (
          <div
            key={`overlay-point-${idx}`}
            className="absolute w-3 h-3 rounded-full pointer-events-none transform -translate-x-1/2 -translate-y-1/2"
            style={{
              left: overlay.point.x * displayWidth,
              top: overlay.point.y * displayHeight,
              backgroundColor: overlay.color || '#ef4444',
              boxShadow: '0 0 0 2px #fff',
            }}
          />
        );
      }
      return null;
    });
  }, [showOverlays, overlays, displayWidth, displayHeight, normToPx]);

  // 全屏模态框
  const FullscreenModal = useCallback(() => {
    if (!showFullscreen || !frame) return null;

    const maxW = window.innerWidth * 0.95;
    const maxH = window.innerHeight * 0.9;
    const aspectRatio = frame.width / frame.height;
    let w = Math.min(frame.width, maxW);
    let h = w / aspectRatio;
    if (h > maxH) {
      h = maxH;
      w = h * aspectRatio;
    }

    const fsNormToPx = (rect: { x: number; y: number; width: number; height: number }) => ({
      x: rect.x * w,
      y: rect.y * h,
      width: rect.width * w,
      height: rect.height * h,
    });

    return (
      <div
        className="fixed inset-0 z-50 flex items-center justify-center bg-black/95"
        onClick={() => setShowFullscreen(false)}
      >
        <div className="relative bg-black rounded-lg overflow-hidden" style={{ width: w, height: h }}>
          <img
            src={`data:image/jpeg;base64,${frame.imageBase64}`}
            alt="全屏预览"
            style={{ width: w, height: h, display: 'block' }}
          />
          {showOverlays && overlays.map((overlay, idx) => {
            if (overlay.type === 'rect' && overlay.rect) {
              const px = fsNormToPx(overlay.rect);
              const color = overlay.color || '#ef4444';
              return (
                <div
                  key={`fs-${idx}`}
                  className="absolute border-2"
                  style={{
                    left: px.x,
                    top: px.y,
                    width: px.width,
                    height: px.height,
                    borderColor: color,
                    backgroundColor: `${color}15`,
                  }}
                />
              );
            }
            return null;
          })}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="absolute top-4 right-4 text-white hover:bg-white/10"
          onPress={() => setShowFullscreen(false)}
        >
          <X className="h-6 w-6" />
        </Button>
        <div className="absolute bottom-4 left-1/2 -translate-x-1/2 text-white text-sm">
          点击空白处关闭
        </div>
      </div>
    );
  }, [showFullscreen, frame, overlays, showOverlays]);

  if (!frame) {
    return (
      <Card className="w-full">
        <CardBody className="flex h-64 items-center justify-center bg-default-100">
          <div className="text-center text-default-400">
            <RotateCcw className="h-12 w-12 mx-auto mb-2 animate-spin" />
            <p className="text-sm">等待预览帧…</p>
          </div>
        </CardBody>
      </Card>
    );
  }

  const imageSrc = `data:image/jpeg;base64,${frame.imageBase64}`;

  return (
    <div className="relative">
      {/* 预览区域 */}
      <Card className="overflow-hidden">
        <div
          ref={containerRef}
          className="relative inline-block bg-default-100"
          style={{ maxWidth: '100%' }}
        >
          <img
            ref={imageRef}
            src={imageSrc}
            alt={`预览帧 ${frame.timestamp.toFixed(3)}s`}
            style={{ width: displayWidth, height: displayHeight, display: 'block' }}
            draggable={false}
          />
          {renderOverlays()}

          {/* 时间戳水印 */}
          <div className="absolute bottom-2 right-2 px-2 py-1 text-xs font-mono bg-black/70 text-white rounded">
            {frame.timestamp.toFixed(3)}s
            {frames.length > 1 && ` / ${currentFrameIndex + 1}/${frames.length}`}
          </div>
        </div>

        {showControls && (
          <CardHeader className="p-3 border-t border-divider">
            <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
              <div className="flex items-center gap-3 flex-wrap">
                <Button
                  variant={isPlaying ? 'secondary' : 'primary'}
                  size="sm"
                  onPress={onPlayPause}
                  aria-label={isPlaying ? '暂停' : '播放'}
                >
                  {isPlaying ? <Pause className="h-4 w-4" /> : <Play className="h-4 w-4" />}
                </Button>
                <Button variant="ghost" size="icon" onPress={onPrevFrame} aria-label="上一帧" isDisabled={currentFrameIndex <= 0}>
                  <SkipBack className="h-4 w-4" />
                </Button>
                <Button variant="ghost" size="icon" onPress={onNextFrame} aria-label="下一帧" isDisabled={frames.length > 0 && currentFrameIndex >= frames.length - 1}>
                  <SkipForward className="h-4 w-4" />
                </Button>

                <div className="flex items-center gap-2 border-l border-divider pl-3 ml-2">
                  <Label className="text-xs text-default-500">速度</Label>
                  <Select
                    selectedKeys={[String(playbackSpeed)]}
                    onSelectionChange={(keys) => {
                      const val = Array.from(keys)[0];
                      if (val) onSpeedChange?.(Number(val));
                    }}
                  >
                    <SelectItem key="0.25" textValue="0.25">0.25x</SelectItem>
                    <SelectItem key="0.5" textValue="0.5">0.5x</SelectItem>
                    <SelectItem key="1" textValue="1">1x</SelectItem>
                    <SelectItem key="2" textValue="2">2x</SelectItem>
                  </Select>
                </div>

                <Button
                  variant="ghost"
                  size="icon"
                  onPress={() => setShowOverlays(!showOverlays)}
                  aria-label={showOverlays ? '隐藏叠加层' : '显示叠加层'}
                  className={showOverlays ? 'text-primary' : ''}
                >
                  {showOverlays ? <Eye className="h-4 w-4" /> : <EyeOff className="h-4 w-4" />}
                </Button>

                <Button variant="ghost" size="icon" onPress={() => setShowFullscreen(true)} aria-label="全屏">
                  <Expand className="h-4 w-4" />
                </Button>
              </div>

              <div className="flex items-center gap-2 text-xs text-default-400">
                <span>帧尺寸: {frame.width}×{frame.height}</span>
                <span>显示: {Math.round(displayWidth)}×{Math.round(displayHeight)}</span>
              </div>
            </div>
          </CardHeader>
        )}
      </Card>

      {/* 帧导航条 */}
      {showControls && frames.length > 1 && (
        <div className="mt-3 overflow-x-auto pb-2">
          <div className="flex gap-1 min-w-max" style={{ width: displayWidth || 'auto' }}>
            {frames.map((f, idx) => (
              <button
                key={f.frameIndex}
                onClick={() => onFrameIndexChange?.(idx)}
                className={cn(
                  'flex-shrink-0 h-16 w-20 relative border-2 rounded transition-colors',
                  idx === currentFrameIndex ? 'border-primary' : 'border-transparent hover:border-divider',
                )}
                style={{ backgroundImage: `url(data:image/jpeg;base64,${f.imageBase64})`, backgroundSize: 'cover', backgroundPosition: 'center' }}
                aria-label={`帧 ${idx + 1}: ${f.timestamp.toFixed(3)}s`}
                aria-current={idx === currentFrameIndex ? 'true' : 'false'}
              >
                <div className="absolute bottom-0 left-0 right-0 h-4 bg-black/60 text-white text-[10px] px-1 truncate">
                  {f.timestamp.toFixed(2)}s
                </div>
              </button>
            ))}
          </div>
        </div>
      )}

      {FullscreenModal()}
    </div>
  );
}