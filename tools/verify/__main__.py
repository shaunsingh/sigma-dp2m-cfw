"""python -m tools.verify OUTDIR: run the emulated suites on a `dp2m build` output; exit 1 on any FAIL."""

from __future__ import annotations

import argparse
import sys
import traceback
from pathlib import Path

from . import custom, labels, m6m
from .harness import Build, Report, Skip

ROOT = Path(__file__).resolve().parents[2]
SUITES = {"m6m": m6m.run, "custom": custom.run, "labels": labels.run}


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(prog="python -m tools.verify", description=__doc__)
    ap.add_argument("out", type=Path, help="build output dir (C78FRM.bin, Firm.bin, DP2MVxxx.bin, m6m/)")
    ap.add_argument("--fw", type=Path, default=ROOT / "data" / "DP2MV105.bin", help="stock package (default: %(default)s)")
    ap.add_argument("--suite", choices=SUITES, action="append", help="run only these suites")
    ap.add_argument("--artifacts", type=Path, help="write contact sheets here")
    a = ap.parse_args(argv)
    b = Build.load(a.out, a.fw)
    if a.artifacts:
        a.artifacts.mkdir(parents=True, exist_ok=True)
    summary = []
    for name in a.suite or SUITES:
        print(f"== {name}", flush=True)
        r = Report(a.artifacts)
        try:
            SUITES[name](b, r)
            status = "FAIL" if r.failed else "PASS"
        except Skip as e:
            print(f"  SKIP {e}")
            status = "SKIP"
        except Exception as e:  # a crashed suite is a failure, the others still run
            traceback.print_exc()
            r.check(False, f"suite aborted: {type(e).__name__}: {e}")
            status = "FAIL"
        summary.append((name, status, r))
    print("== summary")
    for name, status, r in summary:
        print(f"  {name:<7} {status}  {r.passed} pass, {r.failed} fail")
        for f in r.fails:
            print(f"    FAIL {f}")
    return int(any(s == "FAIL" for _, s, _ in summary))


if __name__ == "__main__":
    sys.exit(main())
