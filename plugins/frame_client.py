"""Standard-library client for TimePin's separately versioned frames.extract service.

FramesClient accepts send_request(method, params) -> the JSON-RPC *result* (not
an envelope). JsonRpcPeer supplies that callback for synchronous JSON Lines
stdio plugins, including nested calls from a host run handler. No OpenCV import
or stdio mutation happens when this module is imported. See docs/frame-service.md.
"""

from __future__ import annotations

import base64
import binascii
import itertools
import json
import math
import os
import sys
import threading
from typing import Any, Callable, Dict, Mapping, Optional, Sequence, TextIO, Union

FRAMES_PROTOCOL_VERSION = "1.0.0"
FRAMES_METHOD = "frames.extract"
_MAX_ID = (1 << 64) - 1
SendRequest = Callable[[str, Dict[str, Any]], Any]
Handler = Callable[[str, Dict[str, Any]], Any]
ImageSource = Union[str, os.PathLike, Mapping[str, Any]]


class FrameValidationError(ValueError):
    """Invalid local options, frame data, or encoded image bytes."""


class JsonRpcProtocolError(ValueError):
    """A JSON object from the peer violates the supported JSON-RPC contract."""


class JsonRpcError(RuntimeError):
    """Remote error, or an error deliberately raised by a request handler."""

    def __init__(self, code: int, message: str, data: Any = None) -> None:
        super().__init__("JSON-RPC error [{}]: {}".format(code, message))
        self.code = code
        self.message = message
        self.data = data


def _number(value: Any, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise FrameValidationError("{} must be a finite number".format(name))
    try:
        number = float(value)
    except OverflowError as exc:
        raise FrameValidationError("{} must be finite".format(name)) from exc
    if not math.isfinite(number):
        raise FrameValidationError("{} must be finite".format(name))
    return number


def _path(value: Any, name: str) -> str:
    try:
        path = os.fspath(value)
    except TypeError as exc:
        raise FrameValidationError("{} must be a non-empty text path".format(name)) from exc
    if not isinstance(path, str) or not path.strip() or "\x00" in path:
        raise FrameValidationError("{} must be a non-empty text path".format(name))
    return path


class FramesClient:
    """Transport-independent wrapper; the dispatcher owns IDs and RPC errors.

    The host authorizes video_path and scratch_dir against the active run.
    Local validation here is not a security boundary. Neither the client nor
    the host deletes successful output; the plugin must clean up before reuse.
    """

    def __init__(self, send_request: SendRequest) -> None:
        if not callable(send_request):
            raise TypeError("send_request must be callable")
        self._send_request = send_request

    def extract(
        self,
        video_path: Union[str, os.PathLike],
        *,
        time_range: Sequence[float],
        scratch_dir: Union[str, os.PathLike],
        mode: str = "image2",
        sample_fps: Optional[float] = None,
        output_format: str = "png",
        delivery: str = "files",
        protocol_version: str = FRAMES_PROTOCOL_VERSION,
    ) -> Dict[str, Any]:
        """Extract [start, end) seconds; return the host result unchanged.

        pts_sec is a source timestamp, never calculated from index/sample_fps.
        scratch_dir is required even for base64 delivery. Optional wire fields
        are sent explicitly so callback integrations see a complete request.
        """
        video = _path(video_path, "video_path")
        scratch = _path(scratch_dir, "scratch_dir")
        if protocol_version != FRAMES_PROTOCOL_VERSION:
            raise FrameValidationError("unsupported frames protocolVersion")
        if mode not in ("image2", "keyframes"):
            raise FrameValidationError("mode must be image2 or keyframes")
        if output_format not in ("jpeg", "png"):
            raise FrameValidationError("output_format must be jpeg or png")
        if delivery not in ("files", "base64"):
            raise FrameValidationError("delivery must be files or base64")
        if not isinstance(time_range, (list, tuple)) or len(time_range) != 2:
            raise FrameValidationError("time_range must be [start, end]")
        start = _number(time_range[0], "time_range start")
        end = _number(time_range[1], "time_range end")
        if not 0 <= start < end:
            raise FrameValidationError("time_range requires 0 <= start < end")
        fps = None if sample_fps is None else _number(sample_fps, "sample_fps")
        if fps is not None and fps <= 0:
            raise FrameValidationError("sample_fps must be positive or None")
        result = self._send_request(FRAMES_METHOD, {
            "protocolVersion": protocol_version,
            "video_path": video,
            "options": {
                "mode": mode,
                "time_range": [start, end],
                "sample_fps": fps,
                "output_format": output_format,
                "scratch_dir": scratch,
                "delivery": delivery,
            },
        })
        self._validate_result(result, mode, delivery)
        return result

    @staticmethod
    def _validate_result(result: Any, mode: str, delivery: str) -> None:
        if not isinstance(result, dict):
            raise JsonRpcProtocolError("frames.extract result must be an object, not an RPC envelope")
        if result.get("protocolVersion") != FRAMES_PROTOCOL_VERSION:
            raise JsonRpcProtocolError("unsupported frames.extract result protocolVersion")
        frames = result.get("frames")
        count = result.get("total_frames")
        if (not isinstance(frames, list) or type(count) is not int
                or count != len(frames) or result.get("mode") != mode):
            raise JsonRpcProtocolError("invalid frames.extract result frames/count/mode")
        for frame in frames:
            if (not isinstance(frame, dict) or type(frame.get("index")) is not int
                    or frame["index"] < 0):
                raise JsonRpcProtocolError("invalid frame index")
            try:
                _number(frame.get("pts_sec"), "pts_sec")
                _path(frame.get("path"), "frame path")
            except FrameValidationError as exc:
                raise JsonRpcProtocolError(str(exc)) from exc
            if "image_base64" in frame and not isinstance(frame["image_base64"], str):
                raise JsonRpcProtocolError("image_base64 must be text")
            if delivery == "base64" and "image_base64" not in frame:
                raise JsonRpcProtocolError("base64 delivery requires image_base64")


def read_image_bytes(source: ImageSource) -> bytes:
    """Read encoded JPEG/PNG bytes using Unicode-safe Python file IO.

    If a frame contains image_base64, prefer it over path (the host also retains
    the file in base64 mode). Invalid base64 raises FrameValidationError rather
    than silently falling back to an unrelated file. File IO errors propagate.
    """
    if isinstance(source, Mapping):
        encoded = source.get("image_base64")
        if encoded is not None:
            if not isinstance(encoded, str):
                raise FrameValidationError("image_base64 must be text")
            try:
                return base64.b64decode(encoded, validate=True)
            except (ValueError, binascii.Error) as exc:
                raise FrameValidationError("invalid image_base64") from exc
        source = source.get("path")
    with open(_path(source, "frame path"), "rb") as image:
        return image.read()


def frame_to_ndarray(source: Union[ImageSource, bytes]) -> Any:
    """Optional cv2.imdecode helper (BGR), safe for non-ASCII Windows paths.

    Only callers of this helper need opencv-python and numpy. Neither is a
    dependency of FramesClient, JsonRpcPeer, or read_image_bytes.
    """
    try:
        import cv2
        import numpy as np
    except ImportError as exc:
        raise ImportError("frame_to_ndarray requires optional opencv-python and numpy") from exc
    data = source if isinstance(source, bytes) else read_image_bytes(source)
    image = cv2.imdecode(np.frombuffer(data, dtype=np.uint8), cv2.IMREAD_COLOR)
    if image is None:
        raise FrameValidationError("cv2.imdecode could not decode the image")
    return image


def _utf8_stdio(stream: TextIO) -> TextIO:
    # Reconfigure in place: a second TextIOWrapper would share/close its buffer.
    reconfigure = getattr(stream, "reconfigure", None)
    if reconfigure is not None:
        reconfigure(encoding="utf-8", errors="strict", newline="\n")
    return stream


def _valid_id(value: Any) -> bool:
    return type(value) is int and 0 <= value <= _MAX_ID


class JsonRpcPeer:
    """Single-reader synchronous JSON-RPC peer with reentrant request dispatch.

    Supply a request_handler(method, params) or pass one to serve_forever().
    While call() waits for frames.extract, the SAME handler services host
    requests (e.g. cancel). Notifications use notification_handler, or the
    request handler if no dedicated notification handler was supplied. No
    responses are sent to notifications. Handler code must branch by method.

    Incoming requests and outgoing calls may use the same numeric ID: direction
    plus method/result distinguishes them. Only responses for currently active
    calls are retained, so unrelated/late IDs cannot fill an unbounded queue.
    Nested calls may complete out of order. Concurrent readers/calls from other
    threads are unsupported; notifications may be written from worker threads.
    There is no built-in read timeout; the host owns run timeout/process teardown.
    """

    def __init__(
        self,
        stdin: Optional[TextIO] = None,
        stdout: Optional[TextIO] = None,
        *,
        request_handler: Optional[Handler] = None,
        notification_handler: Optional[Handler] = None,
    ) -> None:
        # Explicit streams are caller-owned text streams; do not rewrap/close them.
        self._in = _utf8_stdio(sys.stdin) if stdin is None else stdin
        self._out = _utf8_stdio(sys.stdout) if stdout is None else stdout
        self.request_handler = request_handler
        self.notification_handler = notification_handler
        self._ids = itertools.count(1)
        self._active: set = set()
        self._responses: Dict[int, Dict[str, Any]] = {}
        self._write_lock = threading.Lock()
        self._closed = False

    def _send(self, message: Dict[str, Any]) -> None:
        # Serialize before writing: unsupported objects never produce a partial line.
        line = json.dumps(message, ensure_ascii=False, allow_nan=False) + "\n"
        with self._write_lock:
            self._out.write(line)
            self._out.flush()

    def notify(self, method: str, params: Dict[str, Any]) -> None:
        self._send({"jsonrpc": "2.0", "method": method, "params": params})

    def respond(self, request_id: int, result: Any = None,
                error: Optional[Dict[str, Any]] = None) -> None:
        message: Dict[str, Any] = {"jsonrpc": "2.0", "id": request_id}
        message["error" if error is not None else "result"] = error if error is not None else result
        self._send(message)

    def call(self, method: str, params: Dict[str, Any]) -> Any:
        """Send a request; return its result or raise JsonRpcError/EOFError.

        Invalid responses raise JsonRpcProtocolError; local IO errors propagate.
        Do not run a separate stdin loop alongside this peer.
        """
        if self._closed:
            raise EOFError("JSON-RPC peer is closed")
        request_id = next(self._ids)
        if request_id > _MAX_ID:
            raise OverflowError("JSON-RPC u64 request ID space exhausted")
        self._active.add(request_id)
        try:
            self._send({"jsonrpc": "2.0", "id": request_id, "method": method, "params": params})
            while request_id not in self._responses:
                if not self.serve_once():
                    raise EOFError("Host input closed while waiting for {} (id {})".format(method, request_id))
            response = self._responses.pop(request_id)
            error = response.get("error")
            if error is not None:
                raise JsonRpcError(error["code"], error["message"], error.get("data"))
            return response["result"]
        finally:
            self._active.discard(request_id)
            self._responses.pop(request_id, None)

    send_request = call

    def serve_once(self, request_handler: Optional[Handler] = None) -> bool:
        """Pump one message; return False on EOF/close. The handler is retained."""
        if request_handler is not None:
            self.request_handler = request_handler
        if self._closed:
            return False
        incoming = self._read_message()
        if incoming is None:
            self._closed = True
            return False
        if incoming.get("jsonrpc") != "2.0":
            raise JsonRpcProtocolError("expected jsonrpc 2.0")
        if "method" in incoming:
            self._dispatch(incoming)
        elif "id" in incoming:
            request_id = incoming["id"]
            if not _valid_id(request_id):
                raise JsonRpcProtocolError("response id must be a u64 integer")
            if request_id in self._active:
                self._validate_response(incoming)
                if request_id in self._responses:
                    raise JsonRpcProtocolError("duplicate response id {}".format(request_id))
                self._responses[request_id] = incoming
            # Ignore unrelated/late responses; never accept them for a future ID.
        else:
            raise JsonRpcProtocolError("message has neither method nor id")
        return True

    def serve_forever(self, request_handler: Optional[Handler] = None) -> None:
        if request_handler is not None:
            self.request_handler = request_handler
        while self.serve_once():
            pass

    def close(self) -> None:
        """Stop pumping after the current handler; do not close caller streams."""
        self._closed = True

    def _read_message(self) -> Optional[Dict[str, Any]]:
        while True:
            line = self._in.readline()
            if line == "":
                return None
            if not line.strip():
                continue
            try:
                message = json.loads(line)
            except json.JSONDecodeError:
                continue  # Existing plugin transport tolerates non-JSON debug lines.
            if isinstance(message, dict):
                return message

    @staticmethod
    def _validate_response(message: Dict[str, Any]) -> None:
        error = message.get("error")
        if error is not None:
            if (not isinstance(error, dict) or type(error.get("code")) is not int
                    or not isinstance(error.get("message"), str)
                    or message.get("result") is not None):
                raise JsonRpcProtocolError("invalid JSON-RPC error response")
        elif "result" not in message:
            raise JsonRpcProtocolError("response has neither result nor error")

    def _dispatch(self, message: Dict[str, Any]) -> None:
        has_id = "id" in message
        request_id = message.get("id")
        if has_id and not _valid_id(request_id):
            raise JsonRpcProtocolError("request id must be a u64 integer")
        method = message["method"]
        params = message.get("params", {})
        try:
            if not isinstance(method, str) or not method:
                raise JsonRpcError(-32600, "method must be non-empty text")
            if not isinstance(params, dict):
                raise JsonRpcError(-32602, "params must be an object")
            handler = self.request_handler if has_id else (self.notification_handler or self.request_handler)
            if handler is None:
                if not has_id:
                    return  # Unhandled notifications have no error channel.
                raise JsonRpcError(-32601, "method not found: {}".format(method))
            if not has_id:
                # Notifications must never abort a pending call or produce a reply.
                try:
                    handler(method, params)
                except (EOFError, OSError, JsonRpcProtocolError):
                    raise
                except Exception:
                    pass
                return
            result = handler(method, params)
            if has_id:
                # Include serialization errors in the request error response.
                self.respond(request_id, result=result)
        except JsonRpcError as exc:
            if has_id:
                error = {"code": exc.code, "message": exc.message}
                if exc.data is not None:
                    error["data"] = exc.data
                self.respond(request_id, error=error)
        except (EOFError, OSError, JsonRpcProtocolError):
            raise  # A broken connection/protocol is not a successful host run.
        except Exception as exc:
            if has_id:
                self.respond(request_id, error={"code": -32603, "message": "{}: {}".format(type(exc).__name__, exc)})
            # Notification-handler failures cannot be acknowledged; dropping
            # them keeps the run loop alive instead of killing it mid-wait.


__all__ = [
    "FRAMES_METHOD", "FRAMES_PROTOCOL_VERSION", "FrameValidationError",
    "FramesClient", "JsonRpcError", "JsonRpcPeer", "JsonRpcProtocolError",
    "frame_to_ndarray", "read_image_bytes",
]
