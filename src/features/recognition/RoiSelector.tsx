import { useCallback, useEffect, useRef, useState } from 'react';
import type { MouseEventHandler, ReactNode } from 'react';
import { X, Maximize2, RotateCcw } from 'lucide-react';
import { Button } from '@/components/ui';
import { cn } from '@/lib/utils';

interface RoiRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

interface RoiSelectorProps {
  /** 视频帧图片 URL（base64 或文件路径） */
  imageSrc: string;
  /** 图片原始尺寸 */
  naturalWidth: number;
  naturalHeight: number;
  /** 当前 ROI（归一化 0-1） */
  roi?: RoiRect;
  /** ROI 变化回调 */
  onRoiChange: (roi: RoiRect | null) => void;
  /** 是否启用编辑 */
  editable?: boolean;
  /** 额外叠加层 */
  overlays?: Overlay[];
  /** 容器最大宽度 */
  maxWidth?: number;
}

interface Overlay {
  type: 'rect' | 'text' | 'point';
  rect?: RoiRect;
  text?: string;
  point?: { x: number; y: number };
  color?: string;
  label?: string;
  confidence?: number;
}

/** ROI 选择器：在视频帧上框选区域，返回归一化坐标 */
export function RoiSelector({
  imageSrc,
  naturalWidth,
  naturalHeight,
  roi,
  onRoiChange,
  editable = true,
  overlays = [],
  maxWidth = 800,
}: RoiSelectorProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const imageRef = useRef<HTMLImageElement>(null);
  const [displayWidth, setDisplayWidth] = useState(0);
  const [displayHeight, setDisplayHeight] = useState(0);
  const [isSelecting, setIsSelecting] = useState(false);
  const [selectionStart, setSelectionStart] = useState<{ x: number; y: number } | null>(null);
  const [currentSelection, setCurrentSelection] = useState<RoiRect | null>(null);
  const [showFullscreen, setShowFullscreen] = useState(false);

  // 计算显示尺寸（保持比例）
  useEffect(() => {
    if (!imageRef.current) return;
    const img = imageRef.current;
    if (img.naturalWidth === 0) return;

    const aspectRatio = img.naturalWidth / img.naturalHeight;
    let w = Math.min(img.naturalWidth, maxWidth);
    let h = w / aspectRatio;

    // 限制最大高度
    const maxHeight = window.innerHeight * 0.7;
    if (h > maxHeight) {
      h = maxHeight;
      w = h * aspectRatio;
    }

    setDisplayWidth(w);
    setDisplayHeight(h);
  }, [imageSrc, maxWidth]);

  // 转换：归一化 -> 像素（显示坐标）
  const normToPx = useCallback(
    (rect: RoiRect) => ({
      x: rect.x * displayWidth,
      y: rect.y * displayHeight,
      width: rect.width * displayWidth,
      height: rect.height * displayHeight,
    }),
    [displayWidth, displayHeight]
  );

  // 转换：像素（显示坐标） -> 归一化
  const pxToNorm = useCallback(
    (x: number, y: number, w: number, h: number): RoiRect => ({
      x: Math.max(0, Math.min(1, x / displayWidth)),
      y: Math.max(0, Math.min(1, y / displayHeight)),
      width: Math.max(0, Math.min(1, w / displayWidth)),
      height: Math.max(0, Math.min(1, h / displayHeight)),
    }),
    [displayWidth, displayHeight]
  );

  // 鼠标按下：开始选择
  const handleMouseDown: MouseEventHandler<HTMLDivElement> = useCallback(
    (e) => {
      if (!editable || e.button !== 0) return;
      const rect = containerRef.current?.getBoundingClientRect();
      if (!rect) return;

      const x = e.clientX - rect.left;
      const y = e.clientY - rect.top;

      // 点击在图片范围内
      if (x >= 0 && x <= displayWidth && y >= 0 && y <= displayHeight) {
        setIsSelecting(true);
        setSelectionStart({ x, y });
        setCurrentSelection({ x, y, width: 0, height: 0 });
      }
    },
    [editable, displayWidth, displayHeight]
  );

  // 鼠标移动：更新选择框
  useEffect(() => {
    if (!isSelecting) return;

    const handleMove = (e: MouseEvent) => {
      if (!selectionStart || !containerRef.current) return;
      const rect = containerRef.current.getBoundingClientRect();
      const x = Math.max(0, Math.min(displayWidth, e.clientX - rect.left));
      const y = Math.max(0, Math.min(displayHeight, e.clientY - rect.top));

      const startX = selectionStart.x;
      const startY = selectionStart.y;
      const width = x - startX;
      const height = y - startY;

      setCurrentSelection({
        x: width >= 0 ? startX : x,
        y: height >= 0 ? startY : y,
        width: Math.abs(width),
        height: Math.abs(height),
      });
    };

    const handleUp = () => {
      if (isSelecting && currentSelection && currentSelection.width > 5 && currentSelection.height > 5) {
        const norm = pxToNorm(
          currentSelection.x,
          currentSelection.y,
          currentSelection.width,
          currentSelection.height
        );
        onRoiChange(norm);
      }
      setIsSelecting(false);
      setSelectionStart(null);
      setCurrentSelection(null);
    };

    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
    return () => {
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };
  }, [isSelecting, selectionStart, currentSelection, displayWidth, displayHeight, pxToNorm, onRoiChange]);

  // 清除 ROI
  const handleClear = useCallback(() => {
    onRoiChange(null);
  }, [onRoiChange]);

  // 打开全屏预览
  const handleFullscreen = useCallback(() => {
    setShowFullscreen(true);
  }, []);

  // 渲染叠加层
  const renderOverlays = useCallback((): ReactNode[] => {
    if (displayWidth === 0 || displayHeight === 0) return [];

    const nodes: ReactNode[] = [];

    // 当前选择框
    if (isSelecting && currentSelection && currentSelection.width > 0 && currentSelection.height > 0) {
      nodes.push(
        <div
          key="selecting"
          className="absolute border-2 border-primary/80 bg-primary/10 pointer-events-none"
          style={{
            left: currentSelection.x,
            top: currentSelection.y,
            width: currentSelection.width,
            height: currentSelection.height,
          }}
        />
      );
    }

    // 现有 ROI
    if (roi && !isSelecting) {
      const px = normToPx(roi);
      nodes.push(
        <div
          key="roi"
          className="absolute border-2 border-primary bg-primary/5 pointer-events-none"
          style={{
            left: px.x,
            top: px.y,
            width: px.width,
            height: px.height,
          }}
        >
          <div className="absolute -top-5 left-0 text-xs text-primary font-medium bg-primary/90 px-1 rounded">
            ROI
          </div>
        </div>
      );
    }

    // 额外叠加层
    overlays.forEach((overlay, idx) => {
      if (overlay.type === 'rect' && overlay.rect) {
        const px = normToPx(overlay.rect);
        const color = overlay.color || '#ef4444';
        nodes.push(
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
                className="absolute -top-5 left-0 text-xs font-medium px-1 rounded"
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
        nodes.push(
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
        nodes.push(
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
    });

    return nodes;
  }, [displayWidth, displayHeight, isSelecting, currentSelection, roi, overlays, normToPx]);

  return (
    <div className="relative">
      {/* 主预览区 */}
      <div
        ref={containerRef}
        className={cn(
          'relative inline-block bg-default-100 rounded-lg overflow-hidden cursor-crosshair',
          editable && 'hover:border-primary/50 transition-colors',
          !editable && 'cursor-default'
        )}
        onMouseDown={handleMouseDown}
        style={{ maxWidth: '100%' }}
      >
        <img
          ref={imageRef}
          src={imageSrc}
          alt="视频帧"
          style={{ width: displayWidth, height: displayHeight, display: 'block' }}
          draggable={false}
        />
        {renderOverlays()}
      </div>

      {/* 工具栏 */}
      {editable && (
        <div className="flex items-center gap-2 mt-3">
          {roi && (
            <Button
              variant="ghost"
              size="sm"
              onPress={handleClear}
              aria-label="清除 ROI"
            >
              <RotateCcw className="h-4 w-4 mr-1" />
              清除选择
            </Button>
          )}
          <Button variant="ghost" size="sm" onPress={handleFullscreen} aria-label="全屏预览">
            <Maximize2 className="h-4 w-4 mr-1" />
            全屏
          </Button>
        </div>
      )}

      {/* 全屏模态框 */}
      {showFullscreen && (
        <FullscreenModal
          imageSrc={imageSrc}
          naturalWidth={naturalWidth}
          naturalHeight={naturalHeight}
          roi={roi}
          overlays={overlays}
          onClose={() => setShowFullscreen(false)}
        />
      )}
    </div>
  );
}

/** 全屏预览模态框 */
function FullscreenModal({
  imageSrc,
  naturalWidth,
  naturalHeight,
  roi,
  overlays,
  onClose,
}: {
  imageSrc: string;
  naturalWidth: number;
  naturalHeight: number;
  roi?: RoiRect;
  overlays: Overlay[];
  onClose: () => void;
}) {
  const [scale, setScale] = useState(1);
  const [pan, setPan] = useState({ x: 0, y: 0 });
  const [isPanning, setIsPanning] = useState(false);
  const panStart = useRef({ x: 0, y: 0 });

  const handleWheel = useCallback(
    (e: React.WheelEvent) => {
      e.preventDefault();
      setScale((prev) => {
        const next = Math.max(0.1, Math.min(5, prev * (e.deltaY > 0 ? 0.9 : 1.1)));
        return next;
      });
    },
    []
  );

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if (e.button === 0) {
      setIsPanning(true);
      panStart.current = { x: e.clientX - pan.x, y: e.clientY - pan.y };
    }
  }, [pan]);

  useEffect(() => {
    const handleMove = (e: MouseEvent) => {
      if (isPanning) {
        setPan({ x: e.clientX - panStart.current.x, y: e.clientY - panStart.current.y });
      }
    };
    const handleUp = () => setIsPanning(false);
    window.addEventListener('mousemove', handleMove);
    window.addEventListener('mouseup', handleUp);
    return () => {
      window.removeEventListener('mousemove', handleMove);
      window.removeEventListener('mouseup', handleUp);
    };
  }, [isPanning]);

  // 计算显示尺寸（全屏）
  const maxW = window.innerWidth * 0.95;
  const maxH = window.innerHeight * 0.9;
  const aspectRatio = naturalWidth / naturalHeight;
  let w = Math.min(naturalWidth, maxW);
  let h = w / aspectRatio;
  if (h > maxH) {
    h = maxH;
    w = h * aspectRatio;
  }
  w *= scale;
  h *= scale;

  const normToPx = (rect: RoiRect) => ({
    x: rect.x * w / scale,
    y: rect.y * h / scale,
    width: rect.width * w / scale,
    height: rect.height * h / scale,
  });

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/90"
      onClick={onClose}
      onWheel={handleWheel}
    >
      <div
        className="relative bg-black rounded-lg overflow-hidden"
        style={{ width: w, height: h, transform: `translate(${pan.x}px, ${pan.y}px)` }}
        onMouseDown={handleMouseDown}
        onClick={(e) => e.stopPropagation()}
      >
        <img src={imageSrc} alt="全屏预览" style={{ width: w / scale, height: h / scale, display: 'block' }} />
        {roi && (
          <div
            className="absolute border-2 border-primary bg-primary/5"
            style={{
              left: normToPx(roi).x,
              top: normToPx(roi).y,
              width: normToPx(roi).width,
              height: normToPx(roi).height,
            }}
          />
        )}
        {overlays.map((overlay, idx) => {
          if (overlay.type === 'rect' && overlay.rect) {
            const px = normToPx(overlay.rect);
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
        onPress={onClose}
      >
        <X className="h-6 w-6" />
      </Button>
      <div className="absolute bottom-4 left-1/2 -translate-x-1/2 text-white text-sm">
        滚轮缩放，拖拽平移，点击空白处关闭
      </div>
    </div>
  );
}