#!/usr/bin/env python3
"""Fail if an alert rule references a metric nothing emits.

An alert on a misspelled metric never fires, and it never fires SILENTLY:
you find out during the incident it was written to catch. Prometheus will
not warn you either — an expression over a nonexistent series is simply
always empty, which is indistinguishable from healthy.

So the names in the rules are checked against the names in the source.
This runs in CI because the failure mode is invisible at review time: the
YAML looks equally correct either way.
"""
import re
import subprocess
import sys
from pathlib import Path

RULES = Path("infra/k8s/base/monitoring/alerts.yaml")
SOURCE_DIRS = ["services", "consumers"]

# Suffixes Prometheus appends to a histogram or summary. `_total` is NOT
# among them: the sources spell counters with it already.
DERIVED_SUFFIXES = ("_bucket", "_sum", "_count")


def emitted_metrics():
    """Every atlas_* literal in the Rust and Kotlin sources."""
    out = subprocess.run(
        ["grep", "-rhoE", '"atlas_[a-z_]+"', *SOURCE_DIRS,
         "--include=*.rs", "--include=*.kt"],
        capture_output=True, text=True, check=False,
    ).stdout
    return {line.strip('"') for line in out.splitlines() if line.strip()}


def referenced_metrics():
    return set(re.findall(r"\batlas_[a-z_]+\b", RULES.read_text()))


def main():
    emitted = emitted_metrics()
    if not emitted:
        print("no metrics found in source - is this running from the repo root?")
        return 2

    referenced = referenced_metrics()
    missing = []
    for name in sorted(referenced):
        if name in emitted:
            continue
        base = next(
            (name[: -len(s)] for s in DERIVED_SUFFIXES if name.endswith(s)),
            None,
        )
        if base and base in emitted:
            continue
        missing.append(name)

    if missing:
        print(f"{RULES} references metrics that nothing emits:")
        for name in missing:
            print(f"  - {name}")
        print("\nEvery alert expression must name a metric the services "
              "actually export, or the rule silently never fires.")
        return 1

    print(f"all {len(referenced)} referenced metrics are emitted")
    return 0


if __name__ == "__main__":
    sys.exit(main())
