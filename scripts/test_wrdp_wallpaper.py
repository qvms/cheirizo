import importlib.util
from importlib.machinery import SourceFileLoader
import json
import tempfile
import unittest
from email.message import Message
from pathlib import Path
from unittest.mock import patch


SCRIPT = (
    Path(__file__).resolve().parents[1]
    / "vendor/wrdp-compositor/contrib/wrdp/bin/wrdp-wallpaper"
)
LOADER = SourceFileLoader("wrdp_wallpaper", str(SCRIPT))
SPEC = importlib.util.spec_from_loader(LOADER.name, LOADER)
assert SPEC is not None
MODULE = importlib.util.module_from_spec(SPEC)
LOADER.exec_module(MODULE)


class FakeResponse:
    def __init__(self, data, content_type, url="https://www.bing.com/image.jpg"):
        self.data = data
        self.url = url
        self.headers = Message()
        self.headers["Content-Type"] = content_type

    def __enter__(self):
        return self

    def __exit__(self, *_):
        return False

    def read(self, amount):
        return self.data[:amount]

    def geturl(self):
        return self.url


class FakeOpener:
    def __init__(self, responses):
        self.responses = iter(responses)

    def open(self, request, timeout):
        return next(self.responses)


class WallpaperTests(unittest.TestCase):
    def test_safe_bing_url(self):
        self.assertEqual(
            MODULE.safe_bing_url({"images": [{"url": "/image.jpg"}]}),
            "https://www.bing.com/image.jpg",
        )
        with self.assertRaises(ValueError):
            MODULE.safe_bing_url({"images": [{"url": "https://example.net/image.jpg"}]})

    def test_refresh_and_cached_command(self):
        metadata = json.dumps({"images": [{"url": "/image.jpg"}]}).encode()
        image = b"\xff\xd8\xffjpeg-data"
        responses = [
            FakeResponse(metadata, "application/json", MODULE.BING_ENDPOINT),
            FakeResponse(image, "image/jpeg"),
        ]
        with tempfile.TemporaryDirectory() as temporary, patch.object(
            MODULE.urllib.request, "build_opener", return_value=FakeOpener(responses)
        ):
            home = Path(temporary)
            destination = MODULE.refresh_bing(home)
            self.assertEqual(destination.read_bytes(), image)
            command = MODULE.swaybg_command(home, {"mode": "bing", "color": "#000000"})
            self.assertEqual(command[:2], ["swaybg", "-i"])
            self.assertEqual(Path(command[2]), destination)

    def test_rejects_unapproved_final_url_and_bad_signature(self):
        with patch.object(
            MODULE.urllib.request,
            "build_opener",
            return_value=FakeOpener(
                [FakeResponse(b"\xff\xd8\xffdata", "image/jpeg", "https://example.net/image.jpg")]
            ),
        ), self.assertRaises(ValueError):
            MODULE.fetch_limited("https://www.bing.com/image.jpg", "image")
        with patch.object(
            MODULE.urllib.request,
            "build_opener",
            return_value=FakeOpener([FakeResponse(b"not-an-image", "image/jpeg")]),
        ), self.assertRaises(ValueError):
            MODULE.fetch_limited("https://www.bing.com/image.jpg", "image")

    def test_solid_fallback(self):
        with tempfile.TemporaryDirectory() as temporary:
            command = MODULE.swaybg_command(
                Path(temporary), {"mode": "bing", "color": "not-a-colour"}
            )
            self.assertEqual(command, ["swaybg", "-c", "#bdbab4"])


if __name__ == "__main__":
    unittest.main()
