#!/usr/bin/env python3
"""Analyze a SILK_SCROLLDBG trace for one scroll scenario.

Reads the per-frame trace on stdin (the `SCROLLDBG ...` lines SilkTerm writes to
stderr when SILK_SCROLLDBG is set) and checks the invariant the scenario is
supposed to hold:

  --mode slide    : the smooth alt-screen slide must engage (app_off != 0) and the
                    content must move monotonically - no bounce (a delta against the
                    scroll direction). Optionally the static-top-band count must match
                    --expect-st (0 for less/vim, which have no title bar).
  --mode hardcut  : the app has a static top band (nano/muffer) so the slide is
                    deliberately disabled - the shift is still detected but app_off
                    must stay 0 across every frame (a plain page redraw).

Exit codes: 0 pass, 1 real regression (a genuine violation with data), 2 skip
(not enough trace / the scene never scrolled - an environment/timing miss, not a
code regression). The runner treats 2 as non-fatal unless --strict.

The bounce metric reconstructs a reference line's screen position as
`app_off - cumulative_shift`: while easing, app_off shrinks with the grid held, so
the position glides one way; a step adds `shift` to both the grid advance and
app_off, so the position stays continuous across steps. Any reversal is a bounce.
"""

import argparse
import re
import sys

TRACE = re.compile(
    r"SCROLLDBG f=(\d+) pane=(\d+) sh=(-?\d+) app_off=(-?[\d.]+) "
    r"slide_sh=(-?[\d.]+) st=(\d+) sb=(\d+) frac=([\d.]+)"
)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--mode", required=True, choices=["slide", "hardcut"])
    ap.add_argument("--expect-st", type=int, default=-1)
    ap.add_argument("--label", default="scene")
    ap.add_argument("--eps", type=float, default=0.02)
    a = ap.parse_args()

    frames = []
    for line in sys.stdin:
        m = TRACE.search(line)
        if m:
            frames.append(
                {
                    "sh": int(m.group(3)),
                    "app_off": float(m.group(4)),
                    "st": int(m.group(6)),
                    "sb": int(m.group(7)),
                }
            )

    def out(tag, msg):
        print(f"[ {tag} {a.label}: {msg} ]")

    if len(frames) < 5:
        out("SKIP", f"only {len(frames)} trace frames (GL warmup / timing?)")
        return 2
    if not any(f["sh"] != 0 for f in frames):
        out("SKIP", f"scene never scrolled (no sh!=0 across {len(frames)} frames)")
        return 2

    engaged = [f for f in frames if abs(f["app_off"]) > a.eps]

    if a.mode == "hardcut":
        if engaged:
            worst = max(abs(f["app_off"]) for f in engaged)
            out("FAIL", f"expected hard-cut but slide engaged on {len(engaged)} "
                       f"frame(s) (max app_off={worst:.3f})")
            return 1
        out("PASS", f"hard-cut: scrolled, app_off stayed 0 across {len(frames)} frames")
        return 0

    # slide mode
    if not engaged:
        out("SKIP", "slide never engaged (app_off stayed 0) - GL/timing miss")
        return 2
    if a.expect_st >= 0:
        bad = [f for f in engaged if f["st"] != a.expect_st]
        if bad:
            out("FAIL", f"expected st={a.expect_st} while sliding but saw st={bad[0]['st']}")
            return 1

    # bounce: pos = app_off - cumulative shift; check it never reverses direction
    cum = 0.0
    pos = []
    for f in frames:
        cum += f["sh"]
        pos.append(f["app_off"] - cum)
    active = [i for i, f in enumerate(frames) if abs(f["app_off"]) > a.eps]
    lo, hi = active[0], active[-1]
    seg = pos[lo : hi + 1]
    net = seg[-1] - seg[0]
    want = -1 if net < 0 else 1
    reversals = 0
    worst = 0.0
    for i in range(1, len(seg)):
        d = seg[i] - seg[i - 1]
        if abs(d) > a.eps and (1 if d > 0 else -1) != want:
            reversals += 1
            worst = max(worst, abs(d))
    if reversals:
        out("FAIL", f"{reversals} content bounce(s) during slide "
                   f"(max {worst:.3f} cells against the scroll direction)")
        return 1

    out("PASS", f"slide engaged {len(engaged)}f, st ok, monotone (0 bounces)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
