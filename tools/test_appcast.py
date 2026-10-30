import os
import stat
import subprocess
import sys
import tempfile
import unittest
import xml.etree.ElementTree as ET

sys.path.insert(0, os.path.dirname(__file__))
import appcast

SU = "{http://www.andymatuschak.org/xml-namespaces/sparkle}"


def fake_sign_update(directory, length_offset=0):
    path = os.path.join(directory, "sign_update")
    with open(path, "w") as file:
        file.write(
            "#!/bin/sh\n"
            f'size=$(wc -c < "$3"); echo "sparkle:edSignature=\\"SIG$(wc -c < "$2")\\" length=\\"$((size + {length_offset}))\\""\n'
        )
    os.chmod(path, os.stat(path).st_mode | stat.S_IXUSR)
    return path


class AppcastTests(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.zip = os.path.join(self.dir.name, "codex-gpui-1.2.3-macos-arm64.zip")
        with open(self.zip, "wb") as file:
            file.write(b"bundle bytes")

    def tearDown(self):
        self.dir.cleanup()

    def test_versions_compare_numerically(self):
        self.assertGreater(appcast.parse_version("v1.10.0"), appcast.parse_version("1.9.9"))
        for bad in ("1.2", "v1.2.3-rc1", "01.2.3", ""):
            with self.assertRaises(ValueError):
                appcast.parse_version(bad)

    def test_checksum_mismatch_is_rejected(self):
        good = appcast.sha256_of(self.zip)
        appcast.verify_checksum(self.zip, f"{good}  x.zip\n")
        with self.assertRaises(ValueError):
            appcast.verify_checksum(self.zip, "0" * 64)
        with open(self.zip, "ab") as file:
            file.write(b"tampered")
        with self.assertRaises(ValueError):
            appcast.verify_checksum(self.zip, good)

    def test_length_mismatch_from_signer_is_rejected(self):
        tool = fake_sign_update(self.dir.name, length_offset=1)
        with self.assertRaises(ValueError):
            appcast.sign(tool, "key", self.zip)

    def test_cli_writes_signed_feed_and_checksum(self):
        out = os.path.join(self.dir.name, "feed")
        env = dict(os.environ, SPARKLE_ED_PRIVATE_KEY="secret")
        subprocess.run(
            [sys.executable, appcast.__file__, self.zip, "--tag", "v1.2.3", "--repo", "o/r",
             "--sign-update", fake_sign_update(self.dir.name), "--out", out],
            check=True, env=env,
        )
        item = ET.parse(os.path.join(out, "appcast.xml")).find("channel/item")
        enclosure = item.find("enclosure")
        self.assertEqual(item.find(SU + "version").text, "1.2.3")
        self.assertEqual(
            enclosure.get("url"),
            "https://github.com/o/r/releases/download/v1.2.3/codex-gpui-1.2.3-macos-arm64.zip",
        )
        self.assertEqual(enclosure.get("length"), str(os.path.getsize(self.zip)))
        self.assertTrue(enclosure.get(SU + "edSignature").startswith("SIG"))
        with open(os.path.join(out, os.path.basename(self.zip) + ".sha256")) as file:
            appcast.verify_checksum(self.zip, file.read())

    def test_cli_requires_key(self):
        env = {k: v for k, v in os.environ.items() if k != "SPARKLE_ED_PRIVATE_KEY"}
        result = subprocess.run(
            [sys.executable, appcast.__file__, self.zip, "--tag", "v1.2.3", "--repo", "o/r",
             "--sign-update", "x", "--out", self.dir.name],
            env=env, capture_output=True,
        )
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
