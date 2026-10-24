import hashlib
import json
import os
import tempfile
import unittest
from pathlib import Path

from tools.verify_outputs import VerificationError, verify_outputs


class VerifyOutputsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.base = Path(self.temp.name)
        self.root = self.base / "out"
        self.root.mkdir(mode=0o700)
        self.result_path = self.base / "result.json"
        self.payload = b"deku package payload\n"
        (self.root / "dist").mkdir()
        (self.root / "dist" / "hello.pkg").write_bytes(self.payload)
        self.result = {
            "schema_version": 1,
            "outcome": "success",
            "outputs": [{
                "path": "dist/hello.pkg",
                "kind": "package",
                "size_bytes": len(self.payload),
                "sha256": hashlib.sha256(self.payload).hexdigest(),
            }],
        }
        self.write_result()

    def tearDown(self):
        self.temp.cleanup()

    def write_result(self, raw=None):
        if raw is None:
            raw = json.dumps(self.result).encode()
        self.result_path.write_bytes(raw)

    def test_accepts_exact_declared_output(self):
        self.assertEqual(verify_outputs(str(self.result_path), str(self.root)), (1, len(self.payload)))

    def test_rejects_digest_mismatch(self):
        self.result["outputs"][0]["sha256"] = "0" * 64
        self.write_result()
        with self.assertRaisesRegex(VerificationError, "digest mismatch"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_size_mismatch(self):
        self.result["outputs"][0]["size_bytes"] += 1
        self.write_result()
        with self.assertRaisesRegex(VerificationError, "size mismatch"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_undeclared_file(self):
        (self.root / "unexpected.bin").write_bytes(b"extra")
        with self.assertRaisesRegex(VerificationError, "undeclared output file"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_unsafe_declared_path(self):
        self.result["outputs"][0]["path"] = "../escape.pkg"
        self.write_result()
        with self.assertRaisesRegex(VerificationError, "unsafe output path"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_duplicate_json_keys(self):
        self.write_result(b'{"schema_version":1,"schema_version":1,"outcome":"success","outputs":[]}')
        with self.assertRaisesRegex(VerificationError, "duplicate JSON object key"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_output_path_collision(self):
        self.result["outputs"].append({
            "path": "dist", "kind": "file", "size_bytes": 1, "sha256": "a" * 64
        })
        self.write_result()
        with self.assertRaisesRegex(VerificationError, "path collision"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_symlinked_output(self):
        output = self.root / "dist" / "hello.pkg"
        output.unlink()
        outside = self.base / "outside"
        outside.write_bytes(self.payload)
        output.symlink_to(outside)
        with self.assertRaisesRegex(VerificationError, "symlink or special file"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_symlinked_parent_directory(self):
        (self.root / "dist" / "hello.pkg").unlink()
        (self.root / "dist").rmdir()
        outside = self.base / "outside"
        outside.mkdir()
        (outside / "hello.pkg").write_bytes(self.payload)
        (self.root / "dist").symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(VerificationError, "symlink or special file"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_symlinked_root(self):
        link = self.base / "out-link"
        link.symlink_to(self.root, target_is_directory=True)
        with self.assertRaisesRegex(VerificationError, "non-symlink directory"):
            verify_outputs(str(self.result_path), str(link))

    def test_rejects_hard_linked_output(self):
        outside = self.base / "outside-link"
        os.link(self.root / "dist" / "hello.pkg", outside)
        with self.assertRaisesRegex(VerificationError, "hard-linked output"):
            verify_outputs(str(self.result_path), str(self.root))

    @unittest.skipUnless(hasattr(os, "mkfifo"), "FIFO creation is not available")
    def test_rejects_special_files(self):
        os.mkfifo(self.root / "dist" / "pipe")
        with self.assertRaisesRegex(VerificationError, "special file"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_excess_total_size(self):
        with self.assertRaisesRegex(VerificationError, "configured limit"):
            verify_outputs(str(self.result_path), str(self.root), max_total_bytes=1)

    def test_rejects_unexpected_empty_directory(self):
        (self.root / "empty").mkdir()
        with self.assertRaisesRegex(VerificationError, "undeclared output directory"):
            verify_outputs(str(self.result_path), str(self.root))

    def test_rejects_result_symlink(self):
        real_result = self.base / "real-result.json"
        real_result.write_text(json.dumps(self.result))
        link = self.base / "result-link.json"
        link.symlink_to(real_result)
        with self.assertRaisesRegex(VerificationError, "regular non-symlink"):
            verify_outputs(str(link), str(self.root))

    def test_rejects_group_or_world_writable_output_root(self):
        if os.name != "posix":
            self.skipTest("POSIX permissions required")
        self.root.chmod(0o777)
        with self.assertRaisesRegex(VerificationError, "group- or world-writable"):
            verify_outputs(str(self.result_path), str(self.root))


if __name__ == "__main__":
    unittest.main()
