<!-- markdownlint-disable MD007 -- Unordered list indentation -->
<!-- markdownlint-disable MD010 -- No hard tabs -->
<!-- markdownlint-disable MD033 -- No inline html -->
<!-- markdownlint-disable MD055 -- Table pipe style [Expected: leading_and_trailing; Actual: leading_only; Missing trailing pipe] -->
<!-- markdownlint-disable MD041 -- First line in a file should be a top-level heading -->

<!-- TOC ignore:true -->
# SilkTerm design

<!-- TOC ignore:true -->
## Table of contents
<!-- TOC -->

- [Goal](#goal)
- [Architecture](#architecture)
	- [Language / Stack Decision](#language--stack-decision)
	- [Logical code organization](#logical-code-organization)
	- [API alacritty_terminal](#api-alacritty_terminal)
	- [Smooth-Scroll](#smooth-scroll)
	- [Output easing new text](#output-easing-new-text)
	- [Smooth-scroll inside full-screen apps](#smooth-scroll-inside-full-screen-apps)
	- [Render Loop Sketch](#render-loop-sketch)
	- [Environment](#environment)

<!-- /TOC -->

## Goal

GUI terminal emulator for Debian/X11/Compiz with pixel-by-pixel smooth scrolling, both:

- Animated easing on output (new text appears).
- Smooth scrollback navigation with wheel.

No existing Linux terminal does animated smooth-scroll on output. (Verified: WezTerm, kitty, foot, Alacritty, GNOME Terminal, Konsole all snap to cell rows.)

## Architecture

### Language / Stack Decision

Rust + `alacritty_terminal` crate (not a fork of Alacritty repo).

Rationale:

- `alacritty_terminal` crate (v0.15.0 at design time; v0.26 as built) ships PTY + full VT/ANSI parser + grid state as a standalone library. Inherit the two hardest, correctness-critical pieces.

- Do not `git fork alacritty` - its renderer is built to snap to cells and maintainers reject smooth scroll by design. Forking = fighting architecture + merge debt. Crate = clean dependency, build only the renderer.

- Renderer: `wgpu` (or `glium` as fallback). Glyph atlas + cell draw.

Rejected alternatives:

- Go (`aminal`, custom): Difficult due to dearth of existing plumbing options; parser is the hard part.

- Zig + libvterm + raylib: viable but less ecosystem glue than Rust path.

- Python: Excluded (not compiled).

### Logical code organization

SilkTerm implements an event-loop-driven renderer over a retained terminal model - closer to a game's update/render loop than to a widget framework. Three logical roles:

- Model (the only source of truth). Each pane embeds an `alacritty_terminal::Term`: the integer character grid, scrollback, cursor, and the full VT/ANSI parser. A per-pane background thread reads the child process's PTY, feeds the bytes into that `Term`, and wakes the UI thread. Global tunables live in one swappable `Settings` (an atomic `Arc`) that every layer reads. Nothing else caches grid contents.

- View (rebuilt every frame, pulled - never pushed). There is no retained widget tree. Each frame every visible pane snapshots its grid into draw data - styled text runs for the glyph renderer, plus solid quads for cell backgrounds, the cursor, and the selection - and the GPU renderers draw it. Smooth scroll is a view-only idea layered on top: the model only knows whole lines, the renderer interpolates a fractional offset between them. Chrome (menu bar, tab bar, context menus, dialogs) is drawn the same immediate-mode way.

- Controller (event routing). winit delivers all input to one `ApplicationHandler`. Keystrokes become PTY bytes for the focused pane (or drive an open menu/dialog instead); the mouse drives selection, focus, divider-drag, pane reorder, and menus. Input never edits the grid directly - it goes to the child, the child replies, and the model updates on the next PTY read.

The spine of the program is a single ownership tree:

```text
App  (winit ApplicationHandler)
+- State                      Main window
|  +- Gfx                     GPU backend: native wgpu surface, or a glutin GL
|  |                            context on X11 (the per-pixel-transparency path)
|  +- renderers               Text (glyphon) + rects + bg image + scrim
|  \- Tabs                    The tab list + active index
|     \- PaneManager          One per tab: a binary split tree (Node::Split / Leaf)
|        \- Pane              A leaf: layout rect, selection, per-pane state
|           \- TermInstance   Alacritty Term + its PTY-reader thread
\- DialogWin?                 Optional pop-out window (Settings / About),
                                self-contained with its own Gfx + text renderer
```

So a window is a list of tabs, a tab is a split tree of panes, a pane wraps one terminal; pop-out dialogs are independent sibling windows.

Frame loop: A PTY read or a user event marks the app dirty or starts an animation; `about_to_wait` renders when something is dirty or animating, and otherwise waits. A render advances the scroll easing, snaps the grid to the nearest whole line, and redraws each pane from current model state. (Frames are driven from `about_to_wait` rather than redraw requests, because `request_redraw` is unreliable under X11/Compiz here.)

### API (alacritty_terminal)

(As designed against 0.15.0; the build tracks the current release - 0.26 as of 2026-07. Signatures below are the stable core that carried over.)

- `Term::scroll_display(Scroll)` - moves viewport by whole lines. `Scroll` enum: `Delta(i32)`, `PageUp`, `PageDown`, `Top`, `Bottom`.

- `grid.display_offset()` - integer line offset from bottom = current viewport position.

- Grid cell iteration (`iter_visible` / indexing) = render source.

- `config::Scrolling` = history limit + line multiplier only. not animation. Ignore for smooth scroll.

Critical constraint: crate's `display_offset` is integer lines. No fractional scroll in crate. Smooth scroll lives entirely in the renderer.

### Smooth-Scroll

Crate owns integer "where grid is." Renderer owns fractional overlay.

1. Hold separate `visual_offset: f32` in render layer (separate from crate's integer `display_offset`).

1. On wheel input: set target, lerp `visual_offset` toward target each frame (~100ms ease).

1. When `visual_offset` crosses a full line boundary: call `scroll_display(Delta(+/-1))` to advance grid integer offset, subtract `1.0` from `visual_offset` to keep fractional remainder.

1. Render: draw grid translated vertically by `visual_offset * cell_height` pixels.

1. Draw one extra row at top + bottom so partial rows fill viewport edges during fractional offset.

### Output easing (new text)

Same mechanism: when new output pushes content up, animate `visual_offset` from +1 line back to 0 over the easing window instead of snapping. Treat output-scroll as an animated target like wheel-scroll.

### Smooth-scroll inside full-screen apps

Scrollback and output easing (above) both have an easy signal: the wheel turns, or the buffer grows, and we ease a fractional offset. Full-screen ("alt-screen") apps - less, vim, nano, muffer - are the hard case, and no other terminal animates them. They do not scroll a buffer; they own the screen and repaint whole lines in place. When such an app scrolls, the terminal just sees the entire grid change - the same text is suddenly a row or two higher or lower. Nothing tells us a scroll happened, by how much, or which rows were meant to hold still. So the whole feature is reverse-engineered from two successive grid snapshots.

- **Detect the scroll.** Every frame we fingerprint each visible row (a hash of its characters) and compare to last frame's fingerprints. `scroll_shift_signed` finds the vertical shift (up to 24 lines, either direction) that lines up the most rows - and requires a minimum number of rows to have *actually* moved, so an in-place status-line redraw, which lines up positionally but did not move, cannot false-trigger a slide.

- **Detect the fixed furniture.** Real TUIs pin a title bar at the top (nano, muffer) and/or a status/input line at the bottom (less, vim). We count the unchanged rows at each end; only the middle region is allowed to slide, the bands hold still. Handling both bands is what makes nano and muffer work where an earlier top-anchored detector only handled less.

- **Ease it into place.** The grid is already at the post-scroll position, so to animate we push the new content *back* by the detected shift and ease that offset to zero - the frame slides into place instead of snapping.

- **Fill the gap with the scrolled-off rows.** The scrolled-off lines are gone from the grid (the app overwrote them), so the gap the slide reveals cannot be redrawn from the model. Each frame the styled rows are snapshotted, and the moment a step is detected, the rows it pushed out of the region are moved into a small retained strip. The strip draws welded to the sliding content's edge and rides the same eased offset, so the gap is always exactly filled with real outgoing content - complete with its own cell backgrounds and readability scrim - and nothing ever moves relative to anything else. (An earlier design retained the whole previous shaped frame instead; its fill could trail the ease by a few lines and it repositioned at every re-capture, which read as a pulsing shadow under a title bar - the reason title-bar apps were temporarily gated to hard-cut.)

A sliding frame therefore composites as: the scrolled-off strip filling the revealed gap, the current middle region sliding over it (clipped between the two bands), the title and status bands redrawn unshifted, and the readability scrim following the whole thing, strip included.

Why it is hard, in one place: there is no scroll event to hook and `alacritty_terminal` does not expose the app's scroll region (DECSTBM), so "a scroll happened, by N lines, with these fixed bands" is inferred heuristically and must reject false positives (an in-place redraw must not bounce - the apt-status-bar hazard); the off-screen content is unrecoverable, so it has to be captured styled a frame before it vanishes and tiled pixel-accurately against the current frame; the fixed bands mean three regions have to tile with no gap and no overlap; and all of it is sub-line and per-frame, riding the same fractional renderer and scrim pass, under a redraw loop that cannot trust X11/Compiz redraw requests. It is opt-in (`smooth_scroll_apps`, default off). The strip retains roughly a screenful of scrolled-off rows, so even a fast wheel burst stays filled; the ease's lag ramp bounds how far the content trails reality.

### Text readability scrim

A bg-coloured backing behind glyphs so text stays legible over a busy background image or a near-transparent terminal. The scene's text is rendered to a coverage texture, turned into a halo, and composited under the crisp text, coloured per-pixel so each glyph's backing takes its own cell's bg colour. The cursor is a separate coverage texture so it can join the halo and the outline as independent toggles.

The halo shape is selectable ("Scrim function"), because a plain Gaussian blur is a poor legibility backing: it is a round kernel, so as the radius grows the backing rounds off and the corners of a solid block recede - a square of text reads as sitting on a separate round blob rather than an even plate. Four functions are offered:

- **Dilate** - the backing grows the same distance from every edge as a square (Chebyshev distance), so corners stay full. The most solid/boxy look.
- **SDF** (default) - the backing grows by true round (Euclidean) distance with full corners: round like the old blur but the corners no longer pull in. This is the described ideal.
- **DT** (distance transform) - the same Euclidean distance rendered as a solid plate with a crisp feathered lip, rather than a soft glow - a highlighter-style backing.
- **Gaussian [ugly]** - the legacy separable blur, kept as a baseline to compare against.

The distance functions share one engine: a separable, exactly-Euclidean distance transform bounded to the halo radius (per-column 1D distance, then a row combine), which is cheap (two passes, no jump-flood) and reads either metric off the same field. Independently, a "Scrim falloff" curve shapes how the backing fades with distance - S-curve, Gaussian, Linear, Logarithmic, or Exponential - applied both as the Gaussian kernel weight and as the distance-path transfer. Falloff and function are orthogonal: the function decides the halo's *shape*, the falloff its *fade*.

### Render Loop Sketch

- Frame: advance lerp -> cross-boundary check -> sync crate offset -> translate render -> draw cells (+overscan rows).

- Need: glyph atlas (rasterize font once, cache cells), cell metrics (width/height in px), vsync via wgpu surface.

### Environment

- Target: Debian, X11. (With or without compositing. Also Windows and macOS - all with x86_64 and ARM64 variants.)

- Pixel-precise input: touchpad gives true pixel deltas; notched mouse wheel snaps to lines (clamp/accumulate notch deltas into smooth target).

## Delivery (CI/CD, branches, releases)

Guiding constraint: GitHub is dumb git hosting plus optional release storage, nothing more. No hosted CI, no Actions, as few third-party tools as possible; the whole pipeline runs locally (`cicd/cicd.bash`).

- Merge gate: `cicd.bash --gate` (fmt check, clippy with warnings as errors, tests) runs as the `pre-push` hook for pushes to main or dev. This is the local stand-in for a hosted CI workflow; feature-branch pushes are not gated.
- Version-bump guard: the same `pre-push` hook blocks a push to main unless its `source/Cargo.toml` version is a strict increase over the version already on main (full semver precedence, including prerelease ordering) - so a release merge can't ship the same-or-lower version. It also requires the README Release badge to match that version (the same check `release.bash` makes, just earlier). Skips on the first main push and on branch deletes; overridable with `--no-verify` / `SKIP_GATE=1`.
- Branch flow: feature branches merge `--no-ff` into `dev` (the integration target). `main` is release-only: merging dev into main cuts a release.
- Releases: `cicd/utility/release.bash` tags the merge `v<version>` and can push the tag and attach the artifacts to a GitHub Release as plain uploads. The version comes from `source/Cargo.toml` alone - the tag is read from it and the build stamps from it, so they can never disagree. Version and README badge get bumped on dev before the release merge; nothing is ever committed directly on main.
- Artifact naming (stable; download links depend on it): `<exe>-<version>-<os-arch>[.exe]` plus `<exe>-<version>-sha256sums.txt`, collected by the pipeline into `cicd/artifacts/release/`.
- Pinning: `rust-toolchain.toml` pins rustc/clippy/rustfmt and the cross targets; cargo-installed helpers (cargo-deny, cargo-zigbuild) are pinned in `cicd/config.bash` (`TOOL_PINS`) with a non-gating drift warning. Dependency freshness is a periodic local `cargo update` pass; cargo-deny advisories flag anything urgent in every run.
- README badges: static shields only (release, license, minimum Rust). No CI badge - there is no hosted workflow to point one at.
