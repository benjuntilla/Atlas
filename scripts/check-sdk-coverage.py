#!/usr/bin/env python3
"""Fail if an SDK does not cover the whole gateway surface.

An SDK that silently lags the API is the worst kind of incomplete: it
looks finished, so nobody notices the missing endpoint until a user needs
it and reaches for the raw HTTP client instead. And nothing in a gateway
diff reminds you that three SDKs in another directory describe the same
surface.

So every path the router serves must appear in each SDK's source. The
check is deliberately crude — a substring search for the path literal —
because anything cleverer would need three language parsers, and the crude
version already catches the failure that actually happens: a route added
to the gateway and to one SDK.
"""
import re
import sys
from pathlib import Path

ROUTES_DIR = Path("services/gateway/src/routes")
PREFIXES = {
    "auth.rs": "/v1/auth",
    "geo.rs": "/v1/geo",
    "payments.rs": "/v1/payments",
    # Health probes are not part of the customer-facing API, so no SDK is
    # expected to expose them.
    "ops.rs": None,
}

SDKS = {
    "typescript": Path("sdks/typescript/src"),
    "rust": Path("sdks/rust/src"),
    "dart": Path("sdks/dart/lib"),
}

ROUTE_RE = re.compile(r'\.route\(\s*"([^"]+)"', re.MULTILINE)


def gateway_paths():
    paths = set()
    for file, prefix in PREFIXES.items():
        if prefix is None:
            continue
        source = ROUTES_DIR / file
        if not source.exists():
            continue
        for path in ROUTE_RE.findall(source.read_text()):
            # `/geofences/:id` is spelled with interpolation in every SDK,
            # so compare on the stable prefix before the parameter.
            paths.add(f"{prefix}{path}".split("/:")[0])
    return paths


def sdk_text(directory):
    return "\n".join(
        p.read_text()
        for p in directory.rglob("*")
        if p.is_file() and p.suffix in {".ts", ".rs", ".dart"}
    )


def main():
    expected = gateway_paths()
    if not expected:
        print("no routes parsed - is this running from the repo root?")
        return 2

    failed = False
    for name, directory in SDKS.items():
        if not directory.exists():
            print(f"{name}: directory {directory} is missing")
            failed = True
            continue
        text = sdk_text(directory)
        missing = sorted(p for p in expected if p not in text)
        if missing:
            failed = True
            print(f"{name} SDK does not cover:")
            for path in missing:
                print(f"  - {path}")
        else:
            print(f"{name}: covers all {len(expected)} endpoints")

    if failed:
        print("\nEvery SDK must cover the whole gateway surface.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
