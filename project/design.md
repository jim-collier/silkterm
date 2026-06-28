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
	- [API alacritty_terminal 0.15.0](#api-alacritty_terminal-0150)
	- [Smooth-Scroll](#smooth-scroll)
	- [Output easing new text](#output-easing-new-text)
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

- `alacritty_terminal` crate (v0.15.0) ships PTY + full VT/ANSI parser + grid state as a standalone library. Inherit the two hardest, correctness-critical pieces.

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
|  +- renderers               Text (glyphon) + rects + bg image + glow
|  \- Tabs                    The tab list + active index
|     \- PaneManager          One per tab: a binary split tree (Node::Split / Leaf)
|        \- Pane              A leaf: layout rect, selection, per-pane state
|           \- TermInstance   Alacritty Term + its PTY-reader thread
\- DialogWin?                 Optional pop-out window (Settings / About),
                                self-contained with its own Gfx + text renderer
```

So a window is a list of tabs, a tab is a split tree of panes, a pane wraps one terminal; pop-out dialogs are independent sibling windows.

Frame loop: A PTY read or a user event marks the app dirty or starts an animation; `about_to_wait` renders when something is dirty or animating, and otherwise waits. A render advances the scroll easing, snaps the grid to the nearest whole line, and redraws each pane from current model state. (Frames are driven from `about_to_wait` rather than redraw requests, because `request_redraw` is unreliable under X11/Compiz here.)

### API (alacritty_terminal 0.15.0)

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

### Render Loop Sketch

- Frame: advance lerp -> cross-boundary check -> sync crate offset -> translate render -> draw cells (+overscan rows).

- Need: glyph atlas (rasterize font once, cache cells), cell metrics (width/height in px), vsync via wgpu surface.

### Environment

- Target: Debian, X11. (With or without compositing. Also Windows and macOS - all with x86_64 and ARM64 variants.

- Pixel-precise input: touchpad gives true pixel deltas; notched mouse wheel snaps to lines (clamp/accumulate notch deltas into smooth target).
