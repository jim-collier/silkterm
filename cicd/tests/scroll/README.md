# Scroll regression harness

Deterministic, headless check of the smooth-scroll behaviour, driven off the
permanent `SILK_SCROLLDBG` per-frame trace in `pane.rs`. It runs SilkTerm headless
(the private `:98` Xvfb on X11, or a `cage` kiosk on Wayland), feeds it scenes that
model how real full-screen apps repaint, and asserts each app scrolls the way it is
*supposed* to right now.

Run it:

```sh
cicd/tests/scroll/run.bash            # deterministic scenes on X11 (gating)
cicd/tests/scroll/run.bash --wayland  # same scenes under a headless Wayland compositor (cage)
cicd/tests/scroll/run.bash --real     # + best-effort smoke of real less/nano/vim.tiny
cicd/tests/scroll/run.bash --help
```

cicd runs `run.bash` as part of stage 3 (skipped under `--quick`; non-fatal on an
environment miss, fatal on a measured regression). With `SCROLL_HARNESS_WAYLAND=1`
in `config.bash` it runs a second time with `--wayland`, so the same scenes must pass
on the Wayland backend too (proving the Linux binary scrolls identically on both
engines). The Wayland pass self-skips where `cage` is not installed.

## X11 vs Wayland

The trace is read from stderr, not from pixels, so both engines run the exact same
scenes and assertions. On X11 the app renders straight onto the private `:98` Xvfb;
on Wayland each scene runs SilkTerm as the single client of a throwaway headless
`cage` kiosk (software `pixman` compositor + software Vulkan), with `DISPLAY` unset so
winit can only pick the Wayland backend. `cage` needs no real GPU or display.

## What each scene expects

Scenes self-scroll on a timer (no key injection - unreliable here), so the trace is
deterministic. `analyze.py` reads it and checks:

| Scene  | shape (static bands)      | expected now |
|--------|---------------------------|--------------|
| less   | none top, 1 bottom        | slide, monotone (0 bounces) |
| vim    | none top, 2 bottom        | slide, monotone (0 bounces) |
| nano   | 1 top (title), 2 bottom   | slide, monotone (0 bounces) |
| muffer | 2 top (header), 1 bottom  | slide, monotone (0 bounces) |

Title-bar apps (nano, muffer) slide again: `SLIDE_TOP_BAND_APPS = true` in `pane.rs`,
with the reveal gap filled by the scrolled-off strip (the styled rows each step pushes
out of the region), so the fill is exact and nothing repositions. If the toggle is
ever turned back off, change the nano/muffer scenes from `slide` back to `hardcut` in
`run.bash`.

Plain shell-output scrolling (`ls -lA`, a command finishing on the last line, a fast
burst) is covered by the library tests in `cargo test` (`pane.rs`/`scroll.rs`): the
"page re-lists / jumps around / scrolls bottom-up" symptoms map to the
`scroll_shift` advance-correctness and the easing-monotonicity checks there.

## Exit codes

`0` all pass or skipped, `1` a real regression was measured. A scene with too little
trace or that never scrolled is a **skip** (an environment/timing miss), non-fatal
unless `--strict`.
