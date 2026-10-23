"""CLI for non-executing build plan validation."""
import argparse
import sys

from .plan import BuildPlanError, load_and_validate


def main() -> int:
    parser = argparse.ArgumentParser(description="Validate a Deku Build plan without executing it")
    parser.add_argument("plan", help="path to a JSON build plan")
    args = parser.parse_args()
    try:
        plan = load_and_validate(args.plan)
    except BuildPlanError as exc:
        print(f"invalid build plan: {exc}", file=sys.stderr)
        return 2
    print("build plan valid (validate-only; no source download or command execution)")
    print(f"plan: {plan['plan_id']} | architecture: {plan['environment']['architecture']}")
    print(f"outputs declared: {len(plan['outputs'])}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
