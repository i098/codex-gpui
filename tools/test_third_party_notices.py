import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "third_party_notices", Path(__file__).with_name("third-party-notices.py")
)
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


class NoticesTest(unittest.TestCase):
    def test_missing_declared_license_stops_packaging(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            package = {
                "id": "dependency", "name": "dependency", "version": "1.0.0",
                "manifest_path": str(root / "vendor" / "Cargo.toml"),
                "license": "MIT", "license_file": "missing-license.txt",
            }
            metadata = {
                "workspace_root": str(root),
                "packages": [{"id": "app", "name": "app", "version": "1", "manifest_path": str(root / "crates/app/Cargo.toml")}, package],
                "resolve": {"nodes": [
                    {"id": "app", "deps": [{"pkg": "dependency", "dep_kinds": [{"kind": None}]}]},
                    {"id": "dependency", "deps": []},
                ]},
            }
            with self.assertRaises(FileNotFoundError):
                notices.notices(metadata)


if __name__ == "__main__":
    unittest.main()
