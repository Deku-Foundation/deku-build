from copy import deepcopy
import json
import tempfile
import unittest
from pathlib import Path

from deku_build.plan import (
    MAX_OUTPUTS,
    MAX_PLAN_BYTES,
    BuildPlanError,
    load_and_validate,
    validate_plan,
)

EXAMPLE = Path(__file__).parents[1] / "examples" / "build-plan.json"


class BuildPlanTests(unittest.TestCase):
    def setUp(self):
        self.plan = json.loads(EXAMPLE.read_text(encoding="utf-8"))

    def test_example_is_valid(self):
        self.assertEqual(validate_plan(self.plan), self.plan)

    def test_load_example(self):
        self.assertEqual(load_and_validate(EXAMPLE)["mode"], "validate-only")

    def test_unknown_schema_fails_closed(self):
        plan = deepcopy(self.plan)
        plan["schema_version"] = 2
        with self.assertRaisesRegex(BuildPlanError, "schema_version"):
            validate_plan(plan)

    def test_privileged_build_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["environment"]["privilege"] = "root"
        with self.assertRaisesRegex(BuildPlanError, "unprivileged"):
            validate_plan(plan)

    def test_network_during_build_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["environment"]["network"] = "enabled"
        with self.assertRaisesRegex(BuildPlanError, "disabled"):
            validate_plan(plan)

    def test_path_traversal_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["outputs"][0]["path"] = "../outside.tar"
        with self.assertRaisesRegex(BuildPlanError, "invalid format|normal relative path segments"):
            validate_plan(plan)

    def test_current_directory_output_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["outputs"][0]["path"] = "."
        with self.assertRaisesRegex(BuildPlanError, "invalid format|normal relative path segments"):
            validate_plan(plan)

    def test_empty_path_segment_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["outputs"][0]["path"] = "dist//package.tar"
        with self.assertRaisesRegex(BuildPlanError, "invalid format|normal relative path segments"):
            validate_plan(plan)

    def test_git_metadata_output_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["outputs"][0]["path"] = "cache/.git/config"
        with self.assertRaisesRegex(BuildPlanError, r"\.git"):
            validate_plan(plan)

    def test_parent_output_must_be_a_directory(self):
        plan = deepcopy(self.plan)
        plan["outputs"] = [
            {"path": "dist", "kind": "file"},
            {"path": "dist/package.tar", "kind": "archive"},
        ]
        with self.assertRaisesRegex(BuildPlanError, "output path collision"):
            validate_plan(plan)

    def test_directory_output_can_contain_child_outputs(self):
        plan = deepcopy(self.plan)
        plan["outputs"] = [
            {"path": "dist", "kind": "directory"},
            {"path": "dist/package.tar", "kind": "archive"},
        ]
        self.assertEqual(validate_plan(plan), plan)

    def test_output_collision_is_independent_of_input_order(self):
        for outputs in (
            [{"path": "dist", "kind": "file"}, {"path": "dist/package.tar", "kind": "archive"}],
            [{"path": "dist/package.tar", "kind": "archive"}, {"path": "dist", "kind": "file"}],
        ):
            plan = deepcopy(self.plan)
            plan["outputs"] = outputs
            with self.subTest(outputs=outputs):
                with self.assertRaisesRegex(BuildPlanError, "output path collision"):
                    validate_plan(plan)

    def test_too_many_outputs_are_rejected(self):
        plan = deepcopy(self.plan)
        plan["outputs"] = [
            {"path": f"out-{index:04d}.tar", "kind": "archive"}
            for index in range(MAX_OUTPUTS + 1)
        ]
        with self.assertRaisesRegex(BuildPlanError, "at most"):
            validate_plan(plan)

    def test_invalid_url_port_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["source"]["url"] = "https://example.com:invalid/source.git"
        with self.assertRaisesRegex(BuildPlanError, "malformed or contains an invalid port"):
            validate_plan(plan)

    def test_malformed_ipv6_url_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["source"]["url"] = "https://[::1/source.git"
        with self.assertRaisesRegex(BuildPlanError, "malformed or contains an invalid port"):
            validate_plan(plan)

    def test_source_url_whitespace_is_rejected(self):
        plan = deepcopy(self.plan)
        plan["source"]["url"] = "https://example.com/source with spaces.git"
        with self.assertRaisesRegex(BuildPlanError, "whitespace or control"):
            validate_plan(plan)

    def test_source_url_query_and_fragment_are_rejected(self):
        for suffix in ("?token=unexpected", "#moving-target"):
            plan = deepcopy(self.plan)
            plan["source"]["url"] += suffix
            with self.subTest(suffix=suffix):
                with self.assertRaisesRegex(BuildPlanError, "query string or fragment"):
                    validate_plan(plan)

    def test_credentials_in_source_url_are_rejected(self):
        plan = deepcopy(self.plan)
        plan["source"]["url"] = "https://user:password@example.com/source.git"
        with self.assertRaisesRegex(BuildPlanError, "embedded credentials"):
            validate_plan(plan)

    def test_duplicate_json_keys_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "duplicate.json"
            path.write_text(
                '{"schema_version":1,"schema_version":1,"plan_id":"hello-plan",'
                '"mode":"validate-only","source":{"url":"https://example.com/src.git",'
                '"revision":"' + "a" * 40 + '","sha256":"' + "b" * 64 + '"},'
                '"environment":{"architecture":"x86_64","network":"disabled-during-build",'
                '"privilege":"unprivileged"},"outputs":[{"path":"pkg.tar","kind":"archive"}]}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(BuildPlanError, "duplicate JSON object key"):
                load_and_validate(path)

    def test_unknown_top_level_fields_are_rejected(self):
        plan = deepcopy(self.plan)
        plan["run"] = "make"
        with self.assertRaisesRegex(BuildPlanError, "exactly"):
            validate_plan(plan)

    def test_oversized_json_is_rejected_before_deserialization(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "oversized.json"
            path.write_bytes(b" " * (MAX_PLAN_BYTES + 1))
            with self.assertRaisesRegex(BuildPlanError, "input limit"):
                load_and_validate(path)

    def test_invalid_utf8_is_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "invalid.json"
            path.write_bytes(b'{"invalid":"' + bytes([0xFF]) + b'"}')
            with self.assertRaisesRegex(BuildPlanError, "valid UTF-8 JSON"):
                load_and_validate(path)

    def test_missing_file_is_reported_as_build_plan_error(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "missing.json"
            with self.assertRaisesRegex(BuildPlanError, "cannot read plan"):
                load_and_validate(path)


if __name__ == "__main__":
    unittest.main()
