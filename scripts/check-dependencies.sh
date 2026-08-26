#!/usr/bin/env bash
#
# Dependency vulnerability scan.
#
# # Why this is not just `cargo audit`
#
# `cargo audit` reads Cargo.lock, and Cargo.lock lists OPTIONAL
# dependencies that no enabled feature ever compiles. On this repository
# that was four of eight findings — quinn-proto (rated 7.5 high), rsa,
# rustls-webpki and spin are all in the lockfile and none of them are in
# any build graph. Reporting those as exposure is how a security report
# trains people to ignore it.
#
# So findings are cross-checked against `cargo tree`, which resolves
# features, and only advisories affecting crates that are actually built
# fail the run. The rest are printed as context.
set -euo pipefail

cd "$(dirname "$0")/.."

if ! command -v cargo-audit >/dev/null 2>&1; then
    echo "installing cargo-audit"
    cargo install cargo-audit --locked --quiet
fi

report=$(mktemp)
trap 'rm -f "$report"' EXIT

# cargo audit exits non-zero when it finds anything; we judge the findings
# ourselves, so the exit code is captured rather than allowed to kill the
# script under `set -e`.
cargo audit --json > "$report" 2>/dev/null || true

python3 - "$report" <<'PY'
import json
import subprocess
import sys

report = json.load(open(sys.argv[1]))

def in_build_graph(crate: str) -> bool:
    """Whether any enabled feature actually compiles this crate."""
    out = subprocess.run(
        ["cargo", "tree", "-i", crate, "-e", "normal"],
        capture_output=True, text=True,
    )
    return out.returncode == 0 and bool(out.stdout.strip())

vulns = report.get("vulnerabilities", {}).get("list", [])
reachable, lockfile_only = [], []
for v in vulns:
    name = v["package"]["name"]
    (reachable if in_build_graph(name) else lockfile_only).append(
        (name, v["package"]["version"], v["advisory"]["id"], v["advisory"]["title"])
    )

if lockfile_only:
    print(f"{len(lockfile_only)} advisory(ies) against crates that are in "
          f"Cargo.lock but never compiled - not exposure:")
    for name, ver, adv, title in lockfile_only:
        print(f"  - {name} {ver} ({adv}) {title[:70]}")
    print()

if reachable:
    print(f"{len(reachable)} advisory(ies) against crates this build ACTUALLY "
          f"compiles:")
    for name, ver, adv, title in reachable:
        print(f"  ! {name} {ver} ({adv}) {title[:70]}")
    print("\nThese are real. Upgrade, or add an explicit ignore in "
          "audit.toml with the reason.")
    sys.exit(1)

print("no advisories against compiled crates")
PY
