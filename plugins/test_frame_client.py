"""Standard-library tests: python -I -B plugins/test_frame_client.py."""

import base64
import builtins
import importlib.util
import io
import itertools
import json
import math
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest.mock import patch


MODULE_PATH = Path(__file__).resolve().with_name("frame_client.py")
SPEC = importlib.util.spec_from_file_location("frame_client_under_test", MODULE_PATH)
client = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(client)


def result(mode="image2", delivery="files"):
    # Deliberately not index / sample_fps: timestamps must survive unchanged.
    frame = {"index": 0, "pts_sec": 0.041708, "path": "C:/授权输出/帧 000000.jpg"}
    if delivery == "base64":
        frame["image_base64"] = base64.b64encode(b"jpeg-bytes").decode("ascii")
    return {"protocolVersion": "1.0.0", "frames": [frame], "total_frames": 1, "mode": mode}


def response(request_id, value=None, error=None):
    message = {"jsonrpc": "2.0", "id": request_id}
    message["error" if error is not None else "result"] = error if error is not None else value
    return message


def request(request_id, method, params=None):
    message = {"jsonrpc": "2.0", "method": method, "params": {} if params is None else params}
    if request_id is not None:
        message["id"] = request_id
    return message


def peer_for(messages, **kwargs):
    wire = "".join(json.dumps(message, ensure_ascii=False) + "\n" for message in messages)
    output = io.StringIO()
    return client.JsonRpcPeer(io.StringIO(wire), output, **kwargs), output


def sent(output):
    return [json.loads(line) for line in output.getvalue().splitlines()]


class FramesClientTests(unittest.TestCase):
    def call(self, callback=None, **changes):
        options = {"time_range": [0, 1], "scratch_dir": Path("授权输出/帧")}
        options.update(changes)
        return client.FramesClient(callback or (lambda _method, _params: result())).extract(
            Path("原视频/录像.mp4"), **options)

    def test_callback_exact_contract_and_real_timestamps(self):
        calls = []
        expected = result()
        actual = self.call(lambda method, params: calls.append((method, params)) or expected,
                           sample_fps=2)
        self.assertIs(actual, expected)
        self.assertEqual(actual["frames"][0]["pts_sec"], 0.041708)
        self.assertEqual(calls, [("frames.extract", {
            "protocolVersion": "1.0.0",
            "video_path": str(Path("原视频/录像.mp4")),
            "options": {"mode": "image2", "time_range": [0.0, 1.0], "sample_fps": 2.0,
                        "output_format": "png", "scratch_dir": str(Path("授权输出/帧")),
                        "delivery": "files"},
        })])

    def test_keyframes_png_and_base64(self):
        calls = []
        actual = self.call(lambda m, p: calls.append(p) or result("keyframes", "base64"),
                           mode="keyframes", output_format="png", delivery="base64")
        self.assertEqual(client.read_image_bytes(actual["frames"][0]), b"jpeg-bytes")
        self.assertIsNone(calls[0]["options"]["sample_fps"])
        self.assertEqual(calls[0]["options"]["output_format"], "png")

    def test_invalid_options_never_send_request(self):
        invalid = [
            {"mode": "indexed"}, {"mode": []}, {"delivery": "pipe"},
            {"output_format": "jpg"}, {"protocol_version": "2.0.0"},
            {"scratch_dir": ""}, {"scratch_dir": b"bytes"}, {"scratch_dir": "a\x00b"},
            {"time_range": None}, {"time_range": "01"}, {"time_range": [0]},
            {"time_range": [0, 1, 2]}, {"time_range": [1, 1]}, {"time_range": [2, 1]},
            {"time_range": [-1, 1]}, {"time_range": [0, math.inf]},
            {"time_range": [math.nan, 1]}, {"time_range": [False, 1]},
            {"time_range": [0, "1"]}, {"sample_fps": 0}, {"sample_fps": -1},
            {"sample_fps": math.nan}, {"sample_fps": math.inf},
            {"sample_fps": True}, {"sample_fps": "2"}, {"sample_fps": 10 ** 1000},
        ]
        def forbidden(*_args):
            self.fail("invalid options must not reach the host")
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(client.FrameValidationError):
                self.call(forbidden, **changes)

    def test_invalid_video_path_and_missing_required_options(self):
        frames = client.FramesClient(lambda _m, _p: result())
        for path in (None, b"bytes", "", "a\x00b"):
            with self.subTest(path=path), self.assertRaises(client.FrameValidationError):
                frames.extract(path, time_range=[0, 1], scratch_dir="out")
        with self.assertRaises(TypeError):
            frames.extract("video.mp4")
        with self.assertRaises(TypeError):
            client.FramesClient(None)

    def test_transport_errors_propagate(self):
        remote = client.JsonRpcError(-32000, "unindexed media", {"detail": "missing PTS"})
        def fail(_method, _params):
            raise remote
        with self.assertRaises(client.JsonRpcError) as caught:
            self.call(fail)
        self.assertIs(caught.exception, remote)

    def test_result_validation(self):
        invalid = [[], {"jsonrpc": "2.0", "id": 1, "result": result()}]
        for field, value in (("protocolVersion", "2.0.0"), ("frames", {}),
                             ("total_frames", 2), ("total_frames", True), ("mode", "keyframes")):
            value_result = result()
            value_result[field] = value
            invalid.append(value_result)
        for field, value in (("index", -1), ("index", False), ("pts_sec", math.nan),
                             ("pts_sec", "0.5"), ("path", ""), ("image_base64", 123)):
            value_result = result()
            value_result["frames"][0][field] = value
            invalid.append(value_result)
        for bad_result in invalid:
            with self.subTest(result=bad_result), self.assertRaises(client.JsonRpcProtocolError):
                self.call(lambda _m, _p: bad_result)
        with self.assertRaises(client.JsonRpcProtocolError):
            self.call(delivery="base64")

    def test_empty_result_is_valid(self):
        empty = {"protocolVersion": "1.0.0", "frames": [], "total_frames": 0, "mode": "image2"}
        self.assertEqual(self.call(lambda _m, _p: empty), empty)

    def test_no_implicit_file_writes_or_cleanup(self):
        with tempfile.TemporaryDirectory() as folder:
            video = Path(folder) / "原录像.mp4"
            video.write_bytes(b"original must stay unchanged")
            scratch = Path(folder) / "scratch"
            client.FramesClient(lambda _m, _p: result()).extract(
                video, time_range=[0, 1], scratch_dir=scratch)
            self.assertEqual(video.read_bytes(), b"original must stay unchanged")
            self.assertFalse(scratch.exists())


class ImageHelpersTests(unittest.TestCase):
    def test_unicode_file_path_and_path_mapping(self):
        with tempfile.TemporaryDirectory() as folder:
            image = Path(folder) / "游戏截图 含空格.png"
            image.write_bytes(b"\x89PNG\x00test")
            self.assertEqual(client.read_image_bytes(image), b"\x89PNG\x00test")
            self.assertEqual(client.read_image_bytes({"path": str(image)}), image.read_bytes())
            self.assertTrue(image.exists(), "reading must not delete a retained frame")

    def test_base64_preferred_even_when_path_is_missing(self):
        frame = {"path": "does-not-exist", "image_base64": "aGVsbG8="}
        self.assertEqual(client.read_image_bytes(frame), b"hello")
        for encoded in ("%%%", "游戏", 123):
            with self.subTest(encoded=encoded), self.assertRaises(client.FrameValidationError):
                client.read_image_bytes({"image_base64": encoded})
        with self.assertRaises(client.FrameValidationError):
            client.read_image_bytes({})
        with self.assertRaises(FileNotFoundError):
            client.read_image_bytes("does-not-exist")

    def test_optional_cv2_uses_imdecode_not_imread(self):
        calls = []
        decoded = object()
        fake_cv2 = types.SimpleNamespace(IMREAD_COLOR=1,
            imdecode=lambda array, flag: calls.append((array, flag)) or decoded)
        fake_np = types.SimpleNamespace(uint8="uint8", frombuffer=lambda data, dtype: (data, dtype))
        with patch.dict(sys.modules, {"cv2": fake_cv2, "numpy": fake_np}):
            self.assertIs(client.frame_to_ndarray({"image_base64": "aGVsbG8="}), decoded)
            self.assertEqual(calls, [((b"hello", "uint8"), 1)])
            fake_cv2.imdecode = lambda _array, _flag: None
            with self.assertRaises(client.FrameValidationError):
                client.frame_to_ndarray(b"invalid image")

    def test_optional_dependency_is_not_required(self):
        real_import = builtins.__import__
        def no_cv2(name, *args, **kwargs):
            if name == "cv2":
                raise ImportError("cv2 deliberately unavailable")
            return real_import(name, *args, **kwargs)
        with patch("builtins.__import__", side_effect=no_cv2):
            self.assertEqual(client.read_image_bytes({"image_base64": "aGk="}), b"hi")
            with self.assertRaisesRegex(ImportError, "optional opencv-python"):
                client.frame_to_ndarray(b"x")


class JsonRpcPeerTests(unittest.TestCase):
    def test_run_nested_extract_and_host_cancel_request_and_notification(self):
        cancelled = []
        messages = [request(50, "run"), request(1, "cancel", {"taskId": "任务"}),
                    request(None, "cancel", {"taskId": "通知"}), response(1, result()),
                    request(51, "shutdown")]
        peer, output = peer_for(messages)
        frames = client.FramesClient(peer.send_request)
        def handle(method, params):
            if method == "run":
                return frames.extract("游戏.mp4", time_range=[0, 1], scratch_dir="out/frames")
            if method == "cancel":
                cancelled.append(params["taskId"])
                return {"cancelled": True}
            if method == "shutdown":
                peer.close()
                return {}
            raise client.JsonRpcError(-32601, "unknown method")
        peer.serve_forever(handle)
        wire = sent(output)
        self.assertEqual(wire[0]["method"], "frames.extract")
        self.assertEqual(wire[0]["id"], 1)
        self.assertEqual(wire[1], response(1, {"cancelled": True}))
        self.assertEqual(wire[2], response(50, result()))
        self.assertEqual(wire[3], response(51, {}))
        self.assertEqual(cancelled, ["任务", "通知"])

    def test_nested_calls_preserve_outer_response_that_arrives_first(self):
        peer, output = peer_for([request(50, "run"), request(60, "probe"),
                                 response(1, "outer"), response(2, "inner")])
        def handle(method, _params):
            return peer.call("outer" if method == "run" else "inner", {})
        peer.serve_forever(handle)
        self.assertEqual(sent(output), [request(1, "outer"), request(2, "inner"),
                                        response(60, "inner"), response(50, "outer")])
        self.assertFalse(peer._responses)
        self.assertFalse(peer._active)

    def test_unknown_or_future_response_ids_are_not_cached(self):
        peer, output = peer_for([response(2, "stale"), response(1, "one"), response(2, "two")])
        self.assertEqual(peer.call("first", {}), "one")
        self.assertEqual(peer.call("second", {}), "two")
        self.assertEqual([message["id"] for message in sent(output)], [1, 2])
        self.assertFalse(peer._responses)

    def test_remote_error_preserves_code_message_and_data(self):
        error = {"code": -32000, "message": "视频缺少可用 PTS", "data": {"mode": "keyframes"}}
        peer, _output = peer_for([response(1, error=error)])
        with self.assertRaises(client.JsonRpcError) as caught:
            peer.call("frames.extract", {})
        self.assertEqual(caught.exception.code, -32000)
        self.assertEqual(caught.exception.message, error["message"])
        self.assertEqual(caught.exception.data, error["data"])
        self.assertFalse(peer._active)

    def test_default_method_not_found_and_invalid_params(self):
        peer, output = peer_for([request(10, "unknown"), request(11, "run", [1])])
        peer.serve_forever()
        wire = sent(output)
        self.assertEqual([item["error"]["code"] for item in wire], [-32601, -32602])
        self.assertEqual([item["id"] for item in wire], [10, 11])

    def test_handler_failures_and_serialization_errors_return_errors(self):
        peer, output = peer_for([request(1, "expected"), request(2, "unexpected"),
                                 request(3, "unserializable")])
        def handle(method, _params):
            if method == "expected":
                raise client.JsonRpcError(-32000, "expected", {"retained": True})
            if method == "unexpected":
                raise RuntimeError("plugin failure")
            return object()
        peer.serve_forever(handle)
        wire = sent(output)
        self.assertEqual(wire[0]["error"], {"code": -32000, "message": "expected", "data": {"retained": True}})
        self.assertEqual(wire[1]["error"]["code"], -32603)
        self.assertIn("plugin failure", wire[1]["error"]["message"])
        self.assertEqual(wire[2]["error"]["code"], -32603)

    def test_notifications_have_no_reply_even_on_handler_error(self):
        calls = []
        def notify(method, params):
            calls.append((method, params))
            raise ValueError("notification error")
        peer, output = peer_for([request(None, "cancel", {"taskId": "x"}), response(1, None)],
                                notification_handler=notify)
        self.assertIsNone(peer.call("frames.extract", {}))
        self.assertEqual(calls, [("cancel", {"taskId": "x"})])
        self.assertEqual(len(sent(output)), 1)
        peer.notify("log", {"message": "中文日志"})
        self.assertNotIn("id", sent(output)[1])
        self.assertIn("中文日志", output.getvalue())

    def test_eof_is_not_silent_success_and_closes_peer(self):
        peer, _output = peer_for([])
        with self.assertRaisesRegex(EOFError, "Host input closed"):
            peer.call("frames.extract", {})
        with self.assertRaisesRegex(EOFError, "peer is closed"):
            peer.call("frames.extract", {})
        self.assertFalse(peer._active)

    def test_default_stdio_peer_utf8_in_real_subprocess(self):
        code = """
import sys
sys.path.insert(0, sys.argv[1])
from frame_client import JsonRpcPeer, FramesClient
peer = JsonRpcPeer()
frames = FramesClient(peer.send_request)
def handle(method, params):
    if method == 'run':
        return frames.extract(params['video']['path'], time_range=[0, 1], scratch_dir=params['outputDir'])
    if method == 'shutdown':
        peer.close()
        return {}
peer.serve_forever(handle)
"""
        messages = [request(12, "run", {"video": {"path": "C:/录像/中文.mp4"}, "outputDir": "C:/输出/帧"}),
                    response(1, result()), request(13, "shutdown")]
        completed = subprocess.run([sys.executable, "-I", "-B", "-u", "-c", code, str(MODULE_PATH.parent)],
            input="".join(json.dumps(item, ensure_ascii=False) + "\n" for item in messages).encode("utf-8"),
            stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10, check=True)
        wire = [json.loads(line) for line in completed.stdout.decode("utf-8").splitlines()]
        self.assertEqual(wire[0]["params"]["video_path"], "C:/录像/中文.mp4")
        self.assertEqual(wire[1], response(12, result()))
        self.assertEqual(wire[2], response(13, {}))
        self.assertEqual(completed.stderr, b"")

    def test_invalid_rpc_responses_and_ids(self):
        invalid = [
            {"jsonrpc": "1.0", "id": 1, "result": {}},
            {"jsonrpc": "2.0", "id": 1},
            response(1, error="oops"), response(1, error={"code": True, "message": "oops"}),
            response(1, error={"code": -32000, "message": 1}),
            {"jsonrpc": "2.0", "id": 1, "result": {}, "error": {"code": -32000, "message": "bad"}},
        ]
        invalid += [response(request_id, {}) for request_id in (True, "1", None, -1, 2 ** 64)]
        for message in invalid:
            with self.subTest(message=message):
                peer, _output = peer_for([message])
                with self.assertRaises(client.JsonRpcProtocolError):
                    peer.call("frames.extract", {})
                self.assertFalse(peer._active)

    def test_rust_style_null_error_and_debug_lines(self):
        output = io.StringIO()
        peer = client.JsonRpcPeer(io.StringIO("debug text\n\n[]\n" + json.dumps({
            "jsonrpc": "2.0", "id": 1, "result": None, "error": None}) + "\n"), output)
        self.assertIsNone(peer.call("frames.extract", {}))

    def test_id_overflow_sends_nothing(self):
        peer, output = peer_for([])
        peer._ids = itertools.count(2 ** 64)
        with self.assertRaises(OverflowError):
            peer.call("frames.extract", {})
        self.assertEqual(output.getvalue(), "")

    def test_caller_streams_stay_open(self):
        input_stream, output_stream = io.StringIO(), io.StringIO()
        peer = client.JsonRpcPeer(input_stream, output_stream)
        peer.close()
        self.assertFalse(input_stream.closed)
        self.assertFalse(output_stream.closed)


if __name__ == "__main__":
    unittest.main()
