#!/usr/bin/env python3
"""Fail if the OpenAPI spec and the gateway's router disagree.

A spec that has drifted is worse than no spec: it is a document people
trust, generate clients from, and only discover is wrong at runtime. And
drift is invisible at review time — nothing in a router diff reminds you
that a YAML file two directories away describes the same surface.

So the paths and methods are extracted from both sides and compared. The
router is the source of truth; the spec has to keep up.
"""
import re
import sys
from pathlib import Path

import yaml

SPEC = Path("docs/openapi.yaml")
ROUTES_DIR = Path("services/gateway/src/routes")

# Each sub-router is mounted under a prefix by routes/mod.rs. Health lives
# at the root; everything else hangs off /v1.
PREFIXES = {
    "auth.rs": "/v1/auth",
    "geo.rs": "/v1/geo",
    "payments.rs": "/v1/payments",
    # Health probes are mounted at the root, outside /v1, because they are
    # not part of the customer-facing API.
    "ops.rs": "",
}

ROUTE_RE = re.compile(r'\.route\(\s*"([^"]+)"\s*,\s*(.+?)\)\s*$', re.MULTILINE)
METHOD_RE = re.compile(r"\b(get|post|put|patch|delete)\s*\(")


def router_surface():
    """{(path, METHOD)} as the gateway actually serves it."""
    known = set(PREFIXES) | {"mod.rs"}
    actual = {p.name for p in ROUTES_DIR.glob("*.rs")}
    unmapped = actual - known
    if unmapped:
        # A new route file nobody added here would be skipped silently,
        # which is the same drift this script exists to catch.
        raise SystemExit(
            f"{ROUTES_DIR} has files this script does not know how to mount: "
            f"{sorted(unmapped)}. Add them to PREFIXES."
        )

    found = set()
    for file, prefix in PREFIXES.items():
        source = ROUTES_DIR / file
        if not source.exists():
            continue
        for path, handlers in ROUTE_RE.findall(source.read_text()):
            # axum's `:id` is OpenAPI's `{id}`.
            openapi_path = re.sub(r":(\w+)", r"{\1}", path)
            full = f"{prefix}{openapi_path}"
            for method in METHOD_RE.findall(handlers):
                found.add((full, method.upper()))
    return found


def spec_surface():
    doc = yaml.safe_load(SPEC.read_text())
    return {
        (path, method.upper())
        for path, ops in doc.get("paths", {}).items()
        for method in ops
        if method.lower() in {"get", "post", "put", "patch", "delete"}
    }


def main():
    router = router_surface()
    if not router:
        print("no routes parsed - is this running from the repo root?")
        return 2
    spec = spec_surface()

    undocumented = sorted(router - spec)
    phantom = sorted(spec - router)

    for path, method in undocumented:
        print(f"  the gateway serves {method} {path}, the spec does not document it")
    for path, method in phantom:
        print(f"  the spec documents {method} {path}, the gateway does not serve it")

    if undocumented or phantom:
        print(f"\n{SPEC} disagrees with {ROUTES_DIR}. The router is the truth.")
        return 1

    print(f"spec matches the router: {len(router)} operations")
    return 0


if __name__ == "__main__":
    sys.exit(main())
