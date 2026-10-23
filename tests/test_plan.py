from copy import deepcopy
import json
import unittest
from pathlib import Path

from deku_build.plan import BuildPlanError, load_and_validate, validate_plan

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
        with self.assertRaisesRegex(BuildPlanError, "inside the output directory"):
            validate_plan(plan)

    def test_credentials_in_source_url_are_rejected(self):
        plan = deepcopy(self.plan)
        plan["source"]["url"] = "https://user:password@example.com/source.git"
        with self.assertRaisesRegex(BuildPlanError, "without embedded credentials"):
            validate_plan(plan)

    def test_unknown_top_level_fields_are_rejected(self):
        plan = deepcopy(self.plan)
        plan["run"] = "make"
        with self.assertRaisesRegex(BuildPlanError, "exactly"):
            validate_plan(plan)


if __name__ == "__main__":
    unittest.main()
