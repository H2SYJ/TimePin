"""ONNX Runtime 会话加载封装。

插件不应直接 `import onnxruntime`——用 `load_onnx_session(path, providers)`
拿到已配置好的 `InferenceSession`，CPU provider 优先，GPU 自动回退。
"""
from __future__ import annotations

import os
from pathlib import Path
from typing import Iterable, List, Optional, Union

import onnxruntime as ort


# 默认 provider 顺序：CUDA → DirectML → CPU（Windows 下 DML 友好）
DEFAULT_PROVIDERS: List[str] = [
    "CUDAExecutionProvider",
    "DmlExecutionProvider",
    "CPUExecutionProvider",
]


def providers_available() -> List[str]:
    """返回当前机器真正可用的 execution providers。"""
    return list(ort.get_available_providers())


def load_onnx_session(
    model_path: Union[str, os.PathLike, bytes],
    providers: Optional[Iterable[str]] = None,
    *,
    intra_op_num_threads: int = 2,
    inter_op_num_threads: int = 1,
    sess_options: Optional[ort.SessionOptions] = None,
) -> ort.InferenceSession:
    """加载 ONNX 模型，返回 `onnxruntime.InferenceSession`。

    Args:
        model_path: 文件路径（str/PathLike）或模型字节（bytes）。
                    字节形式可用于 PyInstaller 打包的内嵌模型。
        providers: provider 名称列表；None → `DEFAULT_PROVIDERS`。
                   不在可用列表里的 provider 会被 ort 自动跳过。
        intra_op_num_threads / inter_op_num_threads: 线程池大小。
            HUD 数字识别这类小模型用 2/1 足够，避免抢占 UI/主进程资源。
        sess_options: 已构建的 SessionOptions；提供时优先使用。

    Raises:
        FileNotFoundError: 路径不存在。
        onnxruntime.capi.onnxruntime_pybind11_state.Fail: ort 加载失败。
    """
    if isinstance(model_path, (bytes, bytearray)):
        data = bytes(model_path)
        model_arg: Union[str, bytes] = data
    else:
        p = Path(model_path)
        if not p.is_file():
            raise FileNotFoundError(f"ONNX model not found: {p}")
        model_arg = str(p)

    if providers is None:
        providers = list(DEFAULT_PROVIDERS)
    else:
        providers = list(providers)

    if sess_options is None:
        sess_options = ort.SessionOptions()
        sess_options.intra_op_num_threads = intra_op_num_threads
        sess_options.inter_op_num_threads = inter_op_num_threads
        # 小模型 + 频繁推理：关闭图优化可能反而慢，保持默认即可
        sess_options.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_ALL

    return ort.InferenceSession(model_arg, sess_options, providers=providers)
