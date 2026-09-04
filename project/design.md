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
	- [Minimap](#minimap)
	- [Text readability scrim](#text-readability-scrim)
	- [Minimum contrast (2026-08-30)](#minimum-contrast-2026-08-30)
	- [Font fallback stack](#font-fallback-stack)
	- [Hyperlinks](#hyperlinks)
	- [What a double-click grabs (2026-08-26)](#what-a-double-click-grabs-2026-08-26)
	- [Measurements and display scaling](#measurements-and-display-scaling)
	- [Attention colors and dialog chrome](#attention-colors-and-dialog-chrome)
	- [Groups and sub-groups in the Settings dialog](#groups-and-sub-groups-in-the-settings-dialog)
	- [Saved themes](#saved-themes)
	- [The shell list and how it is filled](#the-shell-list-and-how-it-is-filled)
	- [What a pane's shell inherits](#what-a-panes-shell-inherits)
	- [A prompt is offered to bash, never installed (2026-08-30)](#a-prompt-is-offered-to-bash-never-installed-2026-08-30)
	- [One tip system, four places that draw it (2026-08-30)](#one-tip-system-four-places-that-draw-it-2026-08-30)
	- [Render Loop Sketch](#render-loop-sketch)
	- [Output notices under a flood](#output-notices-under-a-flood)
	- [Environment](#environment)
	- [Startup and slow external resources](#startup-and-slow-external-resources)
	- [Configuration format](#configuration-format)
	- [Variables in a setting (2026-08-30)](#variables-in-a-setting-2026-08-30)
	- [Command-line options](#command-line-options)
- [Delivery (CI/CD, branches, releases)](#delivery-cicd-branches-releases)

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

- View (rebuilt every frame, pulled - never pushed). There is no retained widget tree. Each frame every visible pane snapshots its grid into draw data: styled text runs for the glyph renderer, plus solid quads for cell backgrounds, the cursor, and the selection. The GPU renderers draw that. Smooth scroll is a view-only idea layered on top: the model only knows whole lines, the renderer interpolates a fractional offset between them. Chrome (menu bar, tab bar, context menus, dialogs) is drawn the same immediate-mode way.

- Controller (event routing). winit delivers all input to one `ApplicationHandler`. Keystrokes become PTY bytes for the focused pane (or drive an open menu/dialog instead); the mouse drives selection, focus, divider-drag, pane reorder, and menus. Input never edits the grid directly - it goes to the child, the child replies, and the model updates on the next PTY read.

The spine of the program is a single ownership tree:

```text
App  (winit ApplicationHandler)
+- State                      Main window
|  +- Gfx                     GPU backend: native wgpu surface, a glutin GL context
|  |                            on X11, or a composited DX12 surface on Windows
|  |                            (the two per-pixel-transparency paths)
|  +- renderers               Text (glyphon) + rects + bg image + scrim
|  \- Tabs                    The tab list + active index
|     \- PaneManager          One per tab: a binary split tree (Node::Split / Leaf)
|        \- Pane              A leaf: layout rect, selection, per-pane state
|           \- TermInstance   Alacritty Term + its PTY-reader thread
\- DialogWin?                 Optional pop-out window (Settings / About),
                                self-contained with its own Gfx + text renderer
```

So a window is a list of tabs, a tab is a split tree of panes, a pane wraps one terminal; pop-out dialogs are independent sibling windows.

Frame loop: a PTY read or a user event marks the app dirty or starts an animation. `about_to_wait` renders when something is dirty or animating, and otherwise waits. A render advances the scroll easing, snaps the grid to the nearest whole line, and redraws each pane from current model state. Frames are driven from `about_to_wait` rather than redraw requests, because `request_redraw` is unreliable under X11/Compiz here.

### API (alacritty_terminal)

(As designed against 0.15.0; the build tracks the current release - 0.26 as of 2026-07. Signatures below are the stable core that carried over.)

- `Term::scroll_display(Scroll)` - moves viewport by whole lines. `Scroll` enum: `Delta(i32)`, `PageUp`, `PageDown`, `Top`, `Bottom`.

- `grid.display_offset()` - integer line offset from bottom = current viewport position.

- Grid cell iteration (`iter_visible` / indexing) = render source.

- `config::Scrolling` = history limit + line multiplier only. not animation. Ignore for smooth scroll.

Critical constraint: crate's `display_offset` is integer lines. No fractional scroll in crate. Smooth scroll lives entirely in the renderer.

Sharing the terminal with the reader thread: the reader holds the terminal across a whole read cycle, so the renderer cannot simply take it every frame without stalling. It also cannot merely try and give up. The reader reclaims it immediately, and an impatient try can lose forever, which showed up as a pane frozen for seconds during heavy output. The rule is to try first, and after a couple of frames of getting nowhere, wait properly. Waiting is bounded, because it reserves the terminal ahead of the reader's next cycle. Trying is not bounded at all.

### Smooth-Scroll

Crate owns integer "where grid is." Renderer owns fractional overlay.

1. Hold separate `visual_offset: f32` in render layer (separate from crate's integer `display_offset`).

1. On wheel input: set target, lerp `visual_offset` toward target each frame (~100ms ease).

1. When `visual_offset` crosses a full line boundary: call `scroll_display(Delta(+/-1))` to advance grid integer offset, subtract `1.0` from `visual_offset` to keep fractional remainder.

1. Render: draw grid translated vertically by `visual_offset * cell_height` pixels.

1. Draw one extra row at top + bottom so partial rows fill viewport edges during fractional offset.

A gesture rests on a whole line, and on the line it was heading FOR. A pixel-delta wheel leaves a fractional target, and parking there renders every row shifted by a sub-cell fraction. Rounding to the nearest line is the obvious way to settle that and it is wrong at the end of a gesture: a scroll that stops nine tenths past a boundary goes all the way forward and then hops back, which reads as a glitch even though the travel is under a line. So the detent goes forward, in the direction the wheel was already turning. A scrollbar drag or a track click carries no direction and still rounds to nearest, which is what direct manipulation wants.

The ease curve is deliberately asymmetric. A single exponential lerp starts at peak speed on its first frame and crawls its last pixels in over a second. Both read wrong. Motion instead builds from rest through a two-stage cascade: the visual position chases a leading stage, which chases the target. The stop is sharpened by a minimum closing speed over the final fraction of a line. Ease-out above that band is unchanged. Neither stage can overshoot, so the curve cannot bounce.

### Output easing (new text)

Same mechanism: when new output pushes content up, animate `visual_offset` from +1 line back to 0 over the easing window instead of snapping. Treat output-scroll as an animated target like wheel-scroll.

The view never sits past the grid. The whole part of the offset is what the grid is scrolled by and the fraction is drawn, so an offset beyond the scrollback would pin the whole part while the fraction kept wrapping, one whole-cell hop per line. That was the nano wobble: a burst still easing when the alt screen (no scrollback) took over. The offset is clamped to the scrollback instead, which lands the ease the instant a screen swaps and caps how far a fresh terminal's first output eases.

A surface that could not be seen does not ease at all. A minimized or occluded window, and a tab that is not the shown one, build no frames while they are out of view, so whatever arrived meanwhile is a gap rather than motion. Easing that gap in would say the wrong thing twice: it animates content that is already old, and it reads as output arriving right now. Coming back on screen is one instant cut instead, and the flash that produces is the point - it marks the update as catching up rather than happening.

Catch-up speed is modeled as one curve on a time/speed graph, and each setting is a named segment of it. The curve starts and ends at zero. Each segment hands exactly one thing to the next: the point where it ended. In order:

- Ease-in lifts the speed from rest over its duration. It is the only segment that can leave zero.

- Ramp-up doubles the speed every one of its periods, toward whichever top applies.

- The top is either the single-screen speed or unbounded. Single-screen applies while the burst's own first line is still on screen, so a short listing never races. Once a screenful has scrolled past, the ramp reaches whatever keeps up.

- When the cap lifts mid-burst, Ease-in runs once more from the speed it found itself at, and then Ramp-up resumes.

The segments are straight lines and exponentials adjusted by time, rather than one smooth sigmoid-family curve per segment. That is the shape language audio and video production use, it is cheap to compute, and each knob stays a plain duration.

Winding down is the same curve traced backwards. Ramp-down is a braking curve computed from Ease-out's landing point: at any moment the speed may not exceed what could still be wound down, halving per the Ramp-down period, within the lines left to render. Applied continuously, that one rule is both the reserve and the deceleration. At speed, the view deliberately trails the live output by a braking distance. The moment output ceases, the speed rides the curve down and hands off to Ease-out exactly at the landing band. An earlier design only relaxed the speed during a lull, which in practice never fired. The ramp-down knob read as inert, and stops from speed were cliffs.

The backlog is deliberately not capped in lines. An earlier design capped it at 16 and drove speed from backlog depth. Any real burst filled the cap in about a tenth of a second, after which the view rode the raw output rate and the speed settings had no perceivable effect. The ramps bound the lag in time instead: about one ramp-down period at a steady rate. That is what makes the slow start physically possible. User navigation is exempt from the chase. Wheel and scrollbar keep a plain fixed ease, and a jump back to the bottom sweeps home at full ease speed.

The five settings that shape all of this are presented in the order they are watched, rather than grouped by mechanism:

- Ease-in: how gently the view leaves rest.

- Ramp-up: how hard it accelerates.

- Single-screen speed: the ceiling while the burst still fits on screen.

- Ramp-down: how gradually it winds down.

- Ease-out: how gently it lands.

A sixth, the initial scroll speed, was removed. It fed four separate mechanisms at once, which made every slider appear to influence every other, and the curve's own Ease-in now owns leaving rest.

Two of the five are matched pairs, and each pair runs one direction: higher Ease-in and Ease-out are gentler, higher Ramp-up and Ramp-down are harder. That constraint decides how a value is stored rather than the other way round. Both ends of the ease are stored as how long they take, not how fast they move, purely so each slider runs with its partner instead of against it. Storing the mechanism directly would have made one half of each pair read backwards.

A single "Smooth scrolling" master switch (`scroll.smooth`) turns all scroll animation off at once - wheel ease, output ease, and the full-screen-app slide - without touching the individual settings; their dialog controls gray out while it is off. Every effect group in Settings follows the same master-switch pattern (transparency, wallpaper, contrast mask, text scrim, scrollbar).

### Smooth-scroll inside full-screen apps

Scrollback and output easing (above) both have an easy signal: the wheel turns, or the buffer grows, and we ease a fractional offset. Full-screen ("alt-screen") apps - less, vim, nano, tmux - are the hard case, and no other terminal animates them. They own the screen. Most scroll a region of it with the terminal's own scroll commands (a linefeed at the bottom of a DECSTBM region, `CSI n S`), and the terminal throws the outgoing rows away because the alt screen keeps no scrollback. Some repaint whole lines in place instead, and then the grid just changes under us. Two mechanisms cover the two kinds, and the exact one is asked first.

- **The engine keeps a ledger (2026-09-03).** Our alacritty fork records every region scroll as it happens: which rows moved, by how many lines, and a copy of the rows the scroll pushed out. Each frame the pane reads it and clears it. That is the whole answer for anything that scrolls the terminal. The count is exact and uncapped, so a burst that replaces the screen between two frames is still one known number; the region says which rows hold still; and the outgoing rows are real content rather than a guess. This is what lets tmux ease at all, since it runs on the alt screen where the old approach had nothing to measure. The same count also carries plain output once the scrollback is full and its depth stops growing.

- **Fingerprints where the engine recorded nothing.** An app that repaints its lines with cursor addressing, or ConPTY on Windows re-emitting a scroll as a repaint, leaves no ledger entry. For those, every frame fingerprints each visible row (a hash of its characters) and `scroll_shift_signed` looks for the vertical shift, up to 24 lines either way, that lines up the most rows, requiring enough of them to have really moved. An in-place status-line redraw lines up positionally but did not move, so it cannot false-trigger a slide. The bands it holds still are measured the same way, as the unchanged rows at each end.

- **Ease it into place with the output chase.** The grid is already at the post-scroll position, so to animate we push the content back by the shift and ease that offset to zero. The offset runs the same curve and the same five sliders as plain output. An app scrolling its region by N lines looks exactly like N lines printed at a prompt, and a unit test holds the two channels to one trajectory.

- **Fill the gap with the scrolled-off rows.** The gap the slide reveals cannot be redrawn from the model. The rows the ledger kept, or on the fingerprint path the rows snapshotted styled a frame earlier, go into a retained strip. The strip draws welded to the sliding content's edge and rides the same eased offset, so the gap is always exactly filled with real outgoing content, complete with its own cell backgrounds and readability scrim. The offset can never open more gap than the strip holds. About three screens are kept, so a long burst eases through its tail. An earlier design retained the whole previous shaped frame instead; its fill could trail the ease and it repositioned at every re-capture, which read as a pulsing shadow under a title bar.

- **One row is pinned by reading, not by the region.** A pager like less scrolls the whole screen and rewrites its prompt on the bottom row afterwards. That row reads the same after the scroll as before, so it is held still even though the region says it moved. The same text ends up in the same place either way, so nothing is lost. A blank row never qualifies: tmux scrolls first and draws the freed row a moment later, and a frame built in between would otherwise pin that row and make its new line pop in while the rest slides.

A sliding frame therefore composites as four parts:

- The scrolled-off strip, filling the revealed gap.

- The current middle region, sliding over it, clipped between the two bands.

- The title and status bands, redrawn unshifted.

- The readability scrim, following the whole thing, strip included.

What makes this hard:

- The stock engine has no scroll event and does not expose the app's scroll region. The ledger is our own addition to the fork. Without it, "a scroll happened, by N lines, with these fixed bands" is inferred, and the inference must reject false positives: an in-place redraw must not bounce, the apt-status-bar hazard.

- The off-screen content is unrecoverable from the grid. The ledger copies each row on its way out, which costs a row copy per scrolled line on a screen without scrollback. The fingerprint path still has to capture it styled a frame ahead.

- tmux draws lazily. On a burst it scrolls the outer terminal by the grid's whole advance and only then redraws, so the rows that leave are whatever it had drawn before, sometimes blank. Every terminal's scrollback gets the same stale rows. The strip is faithful to what tmux sent, not to what its pane holds.

- tmux can only scroll a pane that spans the full width. Side-by-side panes are repainted, so they fall to the fingerprint path and mostly hard-cut.

- The fixed bands mean three regions have to tile with no gap and no overlap.

- All of it is sub-line and per-frame, riding the same fractional renderer and scrim pass, under a redraw loop that cannot trust X11/Compiz redraw requests.

It is switchable (`smooth_scroll_apps`, on by default).

### Minimap

An optional sidebar showing the whole scroll buffer in miniature, in the spirit of the Sublime Text / VS Code minimap. Off by default.

Where it sits:

- Per pane, not per window. Scrollback belongs to a pane, so in a split each pane carries its own map.

- The map owns a real column inside the pane's rect - it never overlays the text. Turning it on costs terminal columns, and the PTY resizes like any other layout change. A pane too narrow to spare the room gets no column at all; the column may never take more than half a pane.

- Left to right: terminal text (with the regular scrollbar still overlaying its right edge, unchanged), then the preview, then a slim always-visible scrollbar at the far edge. Two scrollbars on purpose. The inner one is the terminal's, and its position says so. Editors keep one bar at the far right; this is the deliberate departure.

- The configured width is the preview's. The far-edge bar adds a fixed 8 DIP of its own, so changing the width does not change how grabbable the bar is.

The mapping, which is the load-bearing decision:

- The whole buffer - history plus screen - always maps linearly onto the column, top-anchored, oldest first. The editors slide their minimap once the document outgrows it; this one never does. The marker over the preview and the far-edge thumb have to be the same object at the same pixels, and only a linear map keeps that true at every depth.

- With a short buffer, lines draw at a capped height (about 2 px at 1x) and the preview just does not reach the bottom of the column yet.

- With a deep buffer, lines go sub-pixel and blend down, so the map compresses instead of scrolling. At the default 10,000-line scrollback a line is a fraction of a pixel; colored regions still read as bands, which is most of the point.

- The marker carries a floor on its height so a deep buffer still leaves something to grab. The thumb takes the same span, never its own.

What a line looks like:

- Strokes, not glyphs. Per cell: a run of the cell's fg color where there is ink, over the cell's bg where it differs from the default. Hues survive, so errors, prompts and diffs stay findable from across the room.

- Across a line, coverage adds up, so a short or indented line reads as one. Down the column, color is averaged over only the lines that have ink, so a lone red line among blanks keeps its color rather than fading into them.

- How bright a pixel row gets is how much ink actually landed in it, so a mostly blank stretch reads dimmer than a solid page. That is what makes density legible from a distance. One inked line among many would otherwise almost vanish, so a pixel never falls below a set share of the strongest line in it.

- A line does not fill its own height. Once a line draws more than a pixel tall, the gap above and below is what stops a page of text reading as one block; below a pixel there is no room for a gap and the line is taken whole, with the two ramped between so the map does not change brightness as a growing buffer crosses that point.

- The column steps aside while a full-screen program runs, and the text gets its width back. Such a program draws on its own screen, which has no scroll buffer behind it, so the map would show a rectangle at the top of an otherwise empty column.

- Which programs are the exception is a setting rather than a rule, because there is no way to tell from the outside whether a full-screen program is one the map could usefully follow. It ships naming a pager and the two multiplexers.

Interaction:

- The marker drags like a thumb and rides the scroll target, so it tracks the pointer exactly.

- A click elsewhere in the column centers the view there, eased the same way a scrollbar drag settles.

- The wheel over the column scrolls the buffer, same as over the text - including under an app that is tracking the mouse, since there is no cell under the pointer to report.

Alt screen:

- The column stays, so the PTY is not resized every time an app flips screens. The preview shows the screen itself, with no marker and no thumb - there is nothing to scroll.

Cost when off:

- Truly off: no column, no cache, no per-frame work. The whole feature hangs off one config check, and the cache is freed the moment the column goes away.

Settings and chrome:

- A "Minimap" toggle and a width slider on the Movement tab, under the scrollbar cluster, plus a View-menu item. The marker and the far-edge thumb reuse the scrollbar colors, which is why those two color rows are not gated on the scrollbar being on.

How it is built:

- `minimap.rs` owns the line cache, the raster, the mapping and the hit tests. `pane.rs` carves the rect and routes events. Drawing is one textured quad per pane plus overlay quads for the marker and thumb.

- Each line rasterizes once into a fixed-width pixel row when it enters history, since history lines never change; the live screen rows re-raster when they do. A screen swap, a resize and a width change drop the cache. Sitting scrolled back with a full scrollback is the one case where nothing reports how many lines were pushed, so a changed newest-history line is taken as the sign the cache has fallen behind, and it rebuilds whole at a bounded rate.

- The composed image uploads as a texture the size of the column, so texture size limits and the GL context's VRAM-loss re-upload both stay non-issues.

- Under a flood every pixel of the map moves on every line, so a recompose is throttled rather than run per frame. A compose the throttle defers schedules a timed wake, not an animation flag - marking the window animating would bring it straight back, find the throttle still closed, and spin at the frame rate.

- Memory is about 5 MB per pane at the default scrollback and a 120 px column, freed while the map is off.

### Text readability scrim

A bg-colored backing behind glyphs so text stays legible over a busy background image or a near-transparent terminal. The scene's text is rendered to a coverage texture, turned into a halo, and composited under the crisp text, colored per-pixel so each glyph's backing takes its own cell's bg color. The cursor is a separate coverage texture so it can join the halo and the outline as independent toggles.

The halo shape is selectable ("Scrim function"), because a plain Gaussian blur is a poor legibility backing. It is a round kernel, so as the radius grows the backing rounds off and the corners of a solid block recede. A square of text then reads as sitting on a separate round blob rather than an even plate. Four functions are offered:

- **Dilate**. The backing grows the same distance from every edge as a square (Chebyshev distance), so corners stay full. The most solid/boxy look.

- **SDF** (default). The backing grows by true round (Euclidean) distance with full corners: round like the old blur, but the corners no longer pull in. This is the described ideal.

- **DT** (distance transform). The same Euclidean distance rendered as a solid plate with a crisp feathered lip, rather than a soft glow. A highlighter-style backing.

- **Gaussian [ugly]**. The legacy separable blur, kept as a baseline to compare against.

The distance functions share one engine: a separable, exactly-Euclidean distance transform bounded to the halo radius. It takes a per-column 1D distance, then a row combine. That is cheap - two passes, no jump-flood - and reads either metric off the same field. Independently, a "Scrim falloff" curve shapes how the backing fades with distance: Sigmoid, Half-normal, Linear, Logarithmic, or Exponential. It applies both as the blur kernel weight and as the distance-path transfer. Falloff and function are orthogonal: the function decides the halo's shape, the falloff its fade. The falloff is named for the curve it draws rather than for a blur, since the same word otherwise names both a shape and a fade. A bell curve's outer half is a half-normal, and a smoothstep is a sigmoid. Every curve is normalized to reach zero at the halo's outer edge, so a halo ends where its radius says it does.

A third knob, "Strength", decides how bold the finished halo is: each 10% doubles its opacity, up to ten doublings at 100%. Because the doubled value is clamped, the halo's core saturates first and the solid part grows outward along the falloff. So the backing thickens into a plate rather than merely brightening, and it still stops at the radius. At 0 the halo is exactly as the function and falloff built it, which is what ships.

### Minimum contrast (2026-08-30)

Programs pick text colors for a terminal they cannot see. One that assumes a light background writes near-black text, and on a dark one it disappears. So a floor is enforced on how close text may come to the color behind it, and anything under it is moved away: lighter on a dark background, darker on a light one.

The comparison is against the cell's own background color, not against what a pixel behind the glyph actually shows. Per-pixel would mean the wallpaper, the blur, the scrim and the cell color all at once, in the shader, and it would give one word two colors across a gradient. The cell color is also the honest answer in practice: a cell carrying its own background paints it solid, and one on the default background gets a scrim halo of exactly that color, with the wallpaper already pulled most of the way toward it.

Lightness is measured in Oklab rather than as a WCAG ratio. That ratio's constant term swamps the dark end, so two near-blacks score respectably while being invisible, which is the whole case this is for. The move changes Oklab L alone and leaves a and b, so hue and saturation survive and colors stay told apart: a lifted navy is still navy. It goes to whichever side the text is already on, unless that side has no room left before white or black, in which case it goes the other way. Pale text on a merely light background is the case that needs the flip.

The default floor is 45%, which puts previously invisible text at roughly 2.8:1 against a black background. Lower settings measure out as doing nothing visible at all. Two things are deliberately exempt. Text set to exactly its background color is left hidden, since that is how the hidden attribute works and how a program conceals a password. And ANSI black on a dark background is not exempt, even though it is invisible by definition - a program using it as a foreground has made the mistake this setting is for.

Every built-in theme's own foreground clears the floor on its own, which is checked at build time. A theme whose body text needed lifting would mean the floor was repainting the thing it is measured against.

### Font fallback stack

One monospace family is pinned for every weight, because the shaper picks the best face per query and would otherwise let a bold run land in a different family than the regular run beside it.

Which family that is comes from a single search order, the same on every platform:

- the OS monospace family, when "use system font" is on

- then the configured `font_family`, a comma-separated stack

- then the OS monospace family, when "use system font" is off

- then a built-in stack, which is also what a fresh config is written with

- then, only if none of the above is installed, whatever the generic monospace name resolves to

The setting only reorders that list; it never truncates it. An earlier version dropped `font_family` entirely while following the OS font. The same build and the same config then resolved differently depending on the platform, and a configured stack could be silently ignored. Every list is now always walked. A family that is not installed simply falls through to the next one, and the configured stack still has effect as a fallback.

Platforms differ only in what they report, not in the rules applied to it. Windows has a system font size but no monospace family, so following the family there is a no-op and resolution starts at `font_family` without a special case. A toggle with nothing behind it reads as inert, so the Settings checkbox grays out and says why. The same holds for a desktop with no font setting configured at all, which is why the check asks what was detected rather than which platform is running.

The built-in stack is last for a reason. The generic monospace query below it is effectively a lottery over installed fonts, and its winner may ship no bold face. That ejects bold runs into an arbitrary, often proportional, fallback whose advances can't be snapped to the cell grid. Every entry in the built-in stack carries a real bold face. When that stack changes, the outgoing value is recorded, so an existing config still carrying it verbatim is refreshed on the next launch. A stack the user edited is theirs and is left alone.

### Hyperlinks

- URLs in the output are clickable. A link must carry a scheme from a fixed list - http, https, ftp, ftps, sftp, ssh, file, mailto - rather than being guessed from shape. That keeps false positives near zero, since output is full of words with colons and slashes in them. It is also the whole of the security story: a scheme outside the list is not a link, so it can never be handed to the desktop. Bare `www.` prefixes and bare file paths were considered and left out for the same reason.

- Punctuation is trimmed the way a reader would. A full stop or comma after a URL belongs to the sentence, and so does a closing bracket the URL is sitting inside. One the URL itself opened is part of it. A URL that wraps across rows is one link, found from either half.

- Hovering underlines, Ctrl+click opens. The underline appears on a plain hover with no modifier, since a link the user cannot see is a link they will not try. Opening needs Ctrl so it can never be confused with selecting. The press arms and the release opens, so a slipped press can be dragged off to cancel. A right-click on a link puts "Open link" and "Copy link" at the top of the menu, and only there.

- An app that is watching the mouse itself owns the pointer, so nothing underlines over it - holding Shift asks for the local behavior instead, the same bypass selection already uses. The right-click menu continues to win over such an app, as all our chrome does.

- Links open through the desktop's own handler by default, with a configurable program to override it. Deciding what a URL means is the desktop's job, not a terminal's.

### What a double-click grabs (2026-08-26)

- A double-click asks three questions in order, and the first one that answers wins: is this a shape we can name, is it inside a matched pair, is it a word. Word selection was the only rule for a long time and it cannot handle a path with a space in it, because a space is what ends a word.

- The shapes are URLs and file URIs, drive paths (`C:\...`), UNC paths, absolute posix paths, and `~/` paths. Each has to start at an anchor a reader would recognize, with only whitespace, a quote or an opening bracket in front of it. Among the options considered, that was preferred over "anything that is not obviously a word", which reads `and/or` as a path.

- Git remotes and scp targets are shapes as well, in the `[user@]host:path` form. This one was added because a git prompt writes the remote inside brackets beside its status marks, and the matched-pair rule then handed back the marks along with it. Narrowing the pair rule was considered and rejected, since selecting a quoted phrase whole is wanted and was asked for separately. A host needs a dot and an alphabetic ending, and the path needs a separator and a letter in its first segment, which is what keeps `build:release/x` and `notes.txt:12/34` out.

- A remote is the one shape a file extension does not end. A prompt writes the branch after the repository as `repo.git:dev`, and that whole field is what a reader sees as one thing, so stopping at `.git` would leave the branch as a dead patch that selects the brackets instead.

- Where a path ends is two heuristics, both picked for what they refuse. A space is crossed only when a path separator turns up soon after, so a folder name with spaces stays whole while a path followed by a sentence does not swallow it. And the run stops at a file extension, which is what leaves a `:120:5` line number behind.

- A trailing full stop, comma or bracket comes off the same way it does for a link. The two share the trimming idea but not the code, since a path may hold characters a URL may not.

### Measurements and display scaling

- Every measurement in the interface is written once, in device-independent pixels, and turned into real ones only when it is drawn. A DIP is a ninety-sixth of an inch, so a border, a gap or a checkbox is the same physical size on any screen. Nothing is written in raw pixels any more - the terminal grid itself is the only thing sized in them, and that follows the font.

- Where the conversion happens differs by surface, and the difference is deliberate.

	- A pop-out dialog is solved end to end in DIP and converts once, where its layout meets its window. It owns its whole coordinate space, so one boundary is enough and a stray conversion inside would scale something twice.

	- The main window's chrome converts at each measurement instead. Menu bar, tab bar, menus, focus ring and pane gap all share a coordinate space with the terminal grid, which is in real pixels by nature, so there is no boundary to put a conversion on.

- **A measurement TAKEN in real pixels must convert the constant beside it, not the other way about.** Text is measured against the font, which is real pixels by nature; the clear space that goes around it is written in DIP. Adding the two as they stand and dividing the sum at the dialog's boundary shrinks the constant by the scale factor - so at 2x a tab's title had half the clear space its own box allowed for and sat flush against the right edge, and above that it ran past it. Every such site converts the constant where it is used, exactly as the main window's chrome does. There is one rule for it, so the four places that size the dialog's columns cannot drift apart.

- Conversion rounds to whole pixels. A rule or a hairline that landed between two of them would come out soft, and the one-pixel gap between panes is the extreme case: on a screen scaled below 1x, rounding alone would take it to nothing, so a measurement asked to be visible never rounds away.

- A raw-pixel measurement is invisible at 1x and only thins out as the scale factor rises, which makes this the kind of mistake nobody sees on the machine they wrote it on. So the scale factor can be overridden from the environment (`SILK_SCALE`), and a high-DPI layout can be looked at on an ordinary display. Off X11 there is no other way to ask for one.

### Attention colors and dialog chrome

- A theme carries two attention colors rather than one, because they answer different questions. **Highlights** marks several things at once: the live pane's ring, slider handles, revert arrows, the default button. It therefore stays calm enough to appear many times on a screen. **Focus** marks the single control the keyboard is on, so it is the more vivid of the pair and sits well away from its partner in hue. Every theme keeps its two well apart, because a theme that let them converge would draw "look at this" and "you are here" in the same color.

- The dialog's own accents follow the theme. They used to be a fixed blue while the theme's attention color was something else entirely, so the panel could not agree with the terminal it belonged to. The pressed-button fill is that color mixed back toward the panel, which is what makes a pressed button read as pressed rather than as the focused one.

- A focused field shows one outline, not two. The ring lands exactly on the box's own outline and the box stands its border down. Where the ring genuinely spans more than one control, such as a color chip and its hex field, it stays a little outside instead, and only the field's border gives way.

- Tabs sit on a recessed **Gutter** strip and stand on the rule that closes it off, the way tabbed interfaces generally read. The current tab is a lighter gray rather than an accent: "you are here" is not the same job as "look at this". Above the rows there is no heading repeating the tab's own name, since the strip has said it already.

- Controls whose label does not explain them carry a line of flyover help. One that is grayed out explains why instead, that being the more urgent question at the time. The text wraps to the panel rather than being clamped to its edge, so neither a longer sentence nor a larger interface font can push it out of view.

### Groups and sub-groups in the Settings dialog

- Settings are organized two ways. A **group** is a titled section with a rule under it and clear space above. A **sub-group** has no title of its own. It is a control followed by the controls that depend on it, whose labels step right so the run reads as belonging to the leader. A master switch and the things it governs is the shape this exists for.

- Only labels move. Every control keeps the one column it shares with every other row, because a settings list is scanned down that column and a control that wandered with its label would break it. A sub-group is therefore free of any bookkeeping. It is read off the indentation rather than declared a second time, so the leader and its members cannot disagree about who belongs to what.

- A fraction stored as a decimal is shown as a whole percent. Nobody thinks in 0.35, and the file is a different audience from the dialog. The decimal is what the renderer wants and what a hand-edited config should keep. The two directions are exact inverses, so reverting one lands on its own default rather than a hair off it.

- The tabs follow what a person is looking at rather than what the code calls it: Background, Text, Cursor, Movement, Themes, Window. Settings that describe one subject sit together even when they are implemented in different places. The cursor's shape, its animation and whether it joins the text halo are all "cursor" to the person changing them.

### Saved themes

- A theme the user saves is stored **whole**: both variants, the ten palette colors and the sixteen ANSI colors, rather than as a base theme plus the differences.

	- Saving, renaming and deleting all become one operation on one config subtree.

	- A stale color cannot survive under a name that no longer sets it.

	- A saved theme is self-contained enough to hand to someone else.

- What identifies a saved theme in the file is a slug that never changes, with the display name stored beside it. A rename therefore rewrites one line instead of moving a subtree, and the `theme` setting keeps holding a name a person would recognize.

- **Nothing records "this theme has unsaved changes".** A per-color override that disagrees with the theme is that record, and it already lives in the config file. So the Save button is right after a restart, with no flag to keep in step. Saving folds the overrides into the theme and drops them, which is also what makes the button go quiet again.

- A saved theme may take a built-in's name and stand in for it. That gives "customize a built-in" an obvious home, and deleting the saved copy puts the built-in back rather than leaving the name pointing at nothing. Built-ins themselves cannot be renamed or deleted.

- Picking a theme takes on its colors wholesale rather than keeping the previous theme's tweaks on top. A picker that visibly changed nothing on every color that had been edited would read as broken, and those tweaks belonged to the theme being left behind.

### The shell list and how it is filled

- The shells a new tab can be started with are one list, stored in the config as `shells.<key>` with a title, a command, an active flag, a comment, and the date a scan last found the program installed. File order is list order, which is also menu order. It is a plain part of the config, so it can be hand-edited, and the Settings dialog's "Shell" tab is an editor for something that already works - the list came first on purpose.

- **The list names the default shell: its first switched-on entry.** There was a separate `shell.default` setting saying the same thing, and two places claiming to name one shell can only ever disagree; one rule that is visible in the list is worth more than a second field. A config that carried the old setting has that entry moved to the top of the list, once, and the line removed - the value was the user's own statement of which shell they meant, so it is carried rather than dropped. Finding which entry it names is the same identity question the scan asks, not a string compare: the old setting was routinely a bare name where the scan had already stored the full path to the same file, and comparing the two as text put a SECOND copy of the user's default shell at the top of their own list, where the top is what "default shell" now means. An initial population is led by the shell the user logs in with, which is what makes the default right without their having said anything.

- Finding installed shells is background work that starts a few seconds after the WALLPAPER is genuinely on screen - not merely the window. Both are off-thread and both are slow in the same way, so overlapping them puts a stall in the one moment anybody is looking: the gap between the window appearing and the picture arriving in it. A wallpaper that never answers (a share on a dead mount) cannot hold the scan off forever - there is a deadline past which it runs anyway, since a terminal with no shells in its menu is worse than a terminal with no picture behind its text. It stats every directory on PATH and, on Windows, reads the registry - any of which can be a mount or a hive that answers slowly - so none of it may sit between launch and the first frame. It runs on its own thread and the result is folded in when it arrives, the same shape the wallpaper pipeline uses.

- **What a scan may do to the list is deliberately lopsided.** It may add a shell it found, and it may switch off one whose program has gone - keeping the entry, its title, its flags and its place, since a shell that is merely uninstalled is not a shell the user stopped wanting. It may never switch one back on, and never rewrite a command line. A scan cannot tell a program that came back from a switch somebody turned off on purpose, and the cost of guessing wrong runs one way: quietly re-enabling something the user disabled is worse than leaving them one tick to undo.

- Two shells count as one entry when they run the same program with the same arguments. Which program that is has to be resolved rather than compared as text, because the same shell is written several ways (`bash`, `/bin/bash`) and, on Windows, three environments ship a program called `bash` and they are not the same shell. Where a stored entry resolves nowhere at all, a bare name match is enough - that is what lets a reinstall re-arm the disabled entry it belongs to instead of landing beside it as a second copy.

- **The order a fresh list arrives in is stated outright, in one place, rather than falling out of the sequence the looking happens to run in.** Each find is put in a group and the whole set is sorted once at the end. On unix the user's own login shell leads - nothing may sort above it, since the top of the list is what "default shell" means - then the modern cross-platform shells, the language REPLs, and the rest of the POSIX family. Windows has no user shell, so it is stated instead: PowerShell 7, the modern shells, the WSL distributions, the three POSIX-environment bashes, PyCmd, the language REPLs, Windows Cmd, and last the two Windows PowerShell 5 entries - the ones you reach for when something needs them rather than the one you open a terminal to get. Groups that hold shells of equal standing sort alphabetically inside themselves; groups that hold one shell built several ways keep a curated order, which is why MSYS2's full bash is offered above the mini one Git ships.

- The login shell's twin sits directly below it and starts without reading its startup files. Each shell spells that its own way (`--norc`, `--no-rcs`, `--no-config`, `-NoProfile`), so the flag is per shell and the twin only exists where there is one. Only the login shell gets a twin; every shell having one would double a list nobody asked to be long. It arrives switched OFF: it is what you reach for when your own rc file is the thing you are debugging, not a second copy of your shell in the menu every day. `cmd.exe` is deliberately not on that table even though it has such a flag (`/d`, no AutoRun): it is what Windows reports as the command processor, so it is the login shell on every Windows box, and a second "Command Prompt" in everyone's menu costs more than a rarely-set AutoRun key is worth.

- WSL distributions are read from the registry, never by asking `wsl.exe`. A WSL2 distribution lives in a virtual disk, and listing what is installed must not be the thing that boots a virtual machine - that would be slow, surprising, and arguably a security problem for the user. Each distribution is offered whole, running its own default shell; anyone who wants a particular shell inside one edits the entry to say so. Its generation is part of its name (`WSL2; Ubuntu`), because that is the whole difference between two rows that would otherwise read identically, and the WSL2 ones are offered above the WSL1 ones. Both are offered where both exist: a WSL1 distribution is installed and usable, and hiding one because a newer-generation one sits beside it is not a call a scan gets to make. The generation is a bit in the distribution's registry flags - the `Version` value beside it is the registration format's version and reads 2 for a WSL1 distribution just as happily.

- **The Shell tab is the one place allowed to write the list, and a scan landing while it is open is folded into it rather than fought with.** Everywhere else the dialog carries the live list through untouched on Apply: a dialog that opened before a scan landed would otherwise write back the empty list it copied then, emptying the menu for the rest of the session while the file on disk still had every shell in it. The tab needs to write it, so instead the scan is folded into BOTH of the dialog's copies - the edited one, so the user sees what turned up, and the baseline, so the fold does not read as an edit they made. Because a scan only ever appends and switches off, folding it into work already done cannot undo any of it.

- The grid edits every field in the row rather than through a popup: it costs fewer clicks and reuses the field machinery the dialog already has. Four columns are fixed-width and the command takes whatever width is left, since it is the one value that is routinely too long to read at a glance. "Last seen" is read-only - it is the program's own note about the entry, and it is what makes a switched-off shell explicable. The command is required, which is enforced in the two places it can be broken: emptying the field leaves the stored command standing, and an entry that never got one is dropped on the way out of the dialog rather than written as a shell that names nothing to run.

- **Reordering is a mouse gesture on a grip, not four buttons.** Every line carries a drag handle at its left edge and the list reorders under the pointer as it travels, rather than on release - the line being dragged is the line that is seen to move, which is the whole reason to prefer a grip over arrows. It costs four Tab stops per line, and that is the trade taken knowingly: reordering has no keyboard equivalent now. The grip is therefore not a stop at all, since a focus ring on a control that Space cannot work would be worse than no ring.

- **Remove sits between the command and the date, and is drawn in red.** It is the one control in the dialog that destroys something, so it is deliberately kept off the right-hand edge that a pointer travels down on its way to the checkboxes. The red is chrome rather than a theme colour: "this deletes something" is a fixed meaning, and a theme whose accent happened to be red would say it about every control at once.

- **How "where is this shell now" is answered has two halves: what the OS can see, and what the shell says - and the second wins.** Unix reads the link at `/proc/<pid>/cwd`. Windows has no equivalent and no API that reports another process's directory, so it is read out of the shell's own process memory, where SetCurrentDirectory keeps it; the result is checked for still being a directory first, so a layout that ever moved would degrade to "don't know" rather than to a wrong directory. Neither can see a shell that keeps its own idea of where it is - PowerShell's `Set-Location` never tells the OS - which is why the shell is also given a way to say so directly, in the escape sequence every terminal reads for this (OSC 7, and the ConEmu OSC 9;9 spelling that Windows Terminal documents). A report is preferred to the OS answer because it comes from the one that knows; a report naming a directory that is not here, or a machine that is not this one, is dropped and the OS answer stands.

- **The snippet is put into PowerShell profiles automatically, and what that licenses is deliberately narrow.** Asking every user to paste a block into a file before new tabs open in the right place is a poor trade when the block can be put there for them - but writing to somebody else's shell profile has to earn it. So: only a profile that reports nothing at all is touched (our marker or anyone else's OSC 7 / OSC 9;9 means it is in hand); it is appended to, never rewritten, after a copy is saved beside it; the marker makes a second launch a no-op; deleting the block is how it is switched off, and nothing puts it back; the prompt is not replaced, only wrapped where there is no other hook; and a shell whose execution policy would refuse to load the profile is left alone with a line saying why, because a file the shell cannot read is worse than no file. One setting switches the whole thing off before it starts.

- **The listening is done by wrapping the PTY, not by forking the VT parser.** The sequences arrive as bytes and the parser we use handles neither, so the obvious route was a fork of it. The route taken instead is that the PTY is an interface rather than a concrete type: a wrapper sits in front of the real one and scans what it reads on the way past, leaving the byte stream untouched. It costs a single pass looking for one byte, and it means no second fork to carry.

- **Where a shell starts is answered by four things in a fixed order, and the setting is the last of them.** A `--directory` on the command line wins outright; failing that a new tab, pane or window inherits the directory of the pane it came from; a SilkTerm that a shell launched keeps the directory that shell was in; and only what is left over - a launch from the desktop, a menu or a shortcut, where the inherited directory is an accident of whoever started us - reads `shell.startup_directory`. It ships as the literal home token a person on that platform would type (`$HOME`, `%USERPROFILE%`), because a setting whose default is a blank box says nothing about what may be put in it.

- The test for "did a shell launch us" is whether standard input is a terminal, and it is the same question on both platforms. Asking about parent processes would be more direct and costs a process-table walk on Windows, which is not something to put on the path to the first frame. Measured there: launched from a console the standard handles are the console's, launched the way Explorer and the Start menu do it they are null. A release build owns no console of its own either way, so the window-handle and attach-to-parent answers are both wrong for this question.

- Removing an entry asks first, and moving one does not. Doing the opposite undoes a move; nothing undoes a removal.

### What a pane's shell inherits

- A pane's shell inherits the environment SilkTerm itself was launched with. That is deliberate for anything the user set - an activated virtualenv, a PATH they added, a variable they exported before starting the terminal - and it is what makes a terminal opened from a shell behave as a continuation of that shell.

- It is wrong for the bookkeeping a shell keeps for ITSELF. PowerShell 7 prepends its own module directories to the module search path that every version of PowerShell shares, so a Windows PowerShell 5.1 pane opened anywhere below one resolves PSReadLine to PowerShell 7's copy rather than its own and is not allowed to load it - the pane then starts with an error and no line editing. The execution-policy variable is the same shape: one shell sets it and everything that shell starts inherits it, so a pane can run under a policy nobody chose for it.

- So a short list of shell-private variables is put back to what a freshly launched program would see, read from the machine at startup, and everything else is passed through untouched. Among the options it was decided that a narrow list is the only one that holds up: replacing the whole environment would discard the user's own exports, which is the one thing inheriting exists for, and editing the polluted value in place - dropping the entries that belong to the other shell - would depend on where that shell happens to be installed.

- This is not a defect in SilkTerm, and the fix is not a workaround for one. The same thing happens to a command prompt launched from PowerShell 7 with nothing of ours involved. But a pane should start the way it would from the desktop, and the terminal is the only place that can decide that once for every shell it opens.

- The list is not split by platform. PowerShell runs on Linux and macOS too and mutates the same variable there, so two installs side by side collide the same way; and the launching shell's `cd -` target is stale on every platform, since a pane opens somewhere else. What is deliberately left out is the class people reach for first - an activated virtualenv or conda environment - which a user wants a pane to keep, and which could not be removed honestly in any case, because the matching PATH edits would stay behind and leave the pane half-activated.

- Unix constrains the list in a way Windows does not. There is no call that says what a freshly launched program would see - that answer is composed by PAM, the session manager and the login shell between them and is never recorded - so the unix arm can only DROP a variable, never restore it. A name may therefore join the list only if a desktop session never sets it. That holds for all three today, and it is the rule to check before adding a fourth.

### A prompt is offered to bash, never installed (2026-08-30)

- SilkTerm ships x9ps1-git, a git-aware bash prompt, and hands it to the bash panes it starts. It shows the branch, whether the tree is clean, and how far ahead or behind its tracking branch it is. It is on by default.

- Among the ways to deliver it, it was decided to set `PROMPT_COMMAND` in the pane's environment. bash picks that up as a shell variable, and the user's own rc files run afterwards - so anyone who already has a prompt keeps it without knowing this exists, and anyone who does not gets a better one. Nothing is written into anyone's `.bashrc`, there is nothing to uninstall, and it cannot follow the user into a shell SilkTerm did not start.

- The alternative considered was the PowerShell approach: append a block to the rc file. That was rejected here because the PowerShell case has no other option - PowerShell cannot report its directory any other way - while bash has one that touches nothing. A prompt is also a matter of taste in a way a directory report is not, so the reversible answer wins.

- The script is written beside the config the first time a bash pane opens, and rewritten whenever it differs from the compiled-in copy, so an updated SilkTerm carries an updated prompt. The pane runs it through `$BASH`, which is bash's own path - no dependency on `PATH` and no execute bit needed.

- x9ps1-git is a separate MIT project of the same author. The in-repo copy is a vendored copy of its `bin/x9ps1-git`, and will go stale on its own if nobody looks - the version it carries is in its own header.

- PowerShell gets the same prompt, ported rather than shared, and delivered the other way. See below for why the two halves cannot use one mechanism.

### One tip system, four places that draw it (2026-08-30)

- Flyover help comes up in four places: a Settings row, a link in the About box, a tab in the strip, and a menu item. Two renderers and two fonts are involved, so the drawing was never going to be shared.

- What is shared is everything else, and it lives in `tip.rs`: how long the pointer rests before a tip appears, how the text is broken to fit a width, and where the box goes relative to what it describes. A tip that answered faster in one place than another would read as a different kind of thing, which is the reason the delay in particular is one number.

- There are two placement rules, not one, and which applies is a property of what is being described. A Settings row's tip goes under the control, flipping above it when there is no room - a footer button's tip clamped into the bottom edge would sit on the buttons it explains. A menu row's tip goes beside the popup instead, because a box under the row would cover the rows the reader is choosing between.

- A menu row gets a tip only when its label does not already say what it does. Copy and New tab explain themselves; Paste Selection, Read-only and Bare window do not. A tip on every row is noise a reader learns to skip past, which costs the ones that matter.

### Render Loop Sketch

- Frame: advance lerp -> cross-boundary check -> sync crate offset -> translate render -> draw cells (+overscan rows).

- Need: glyph atlas (rasterize font once, cache cells), cell metrics (width/height in px), vsync via wgpu surface.

### Output notices under a flood

- A pane's PTY reader finishes a read cycle roughly every 900 bytes when output is pouring in, and each cycle used to become its own window event. On 32 MiB of output that is about 20,000 events, and it was decided that the window should take delivery of at most one at a time: the notice carries nothing, so the window reads the grid as it stands whenever it gets round to one, and a queue of twenty identical notices only ever produced twenty identical reads.

- Measured on the Windows box, the folding costs nothing and saves a great deal: throughput is unchanged (11.8 against 11.7 MB/s over four alternating pairs) while the process burns a third less CPU and the window thread less than half - the 2.5 seconds that used to go into the operating system's message queue was more than parsing and drawing put together.

- The notice is re-armed BEFORE the window acts on it, so a read cycle landing mid-handling posts a fresh one rather than being dropped. That ordering is the whole safety argument, and it is what a unit test pins.

### Environment

- Target: Debian. The primary dev/reference environment is X11 (Compiz), but one Linux binary runs native on both X11 and Wayland. winit selects the backend at runtime, and X11/Wayland/GL are all loaded on demand. Windows and macOS are targets too, all with x86_64 and ARM64 variants.

	- The X11 path additionally uses a glutin GL context for per-pixel background transparency, because wgpu can't drive an ARGB surface on X11. Wayland uses the plain wgpu surface, which already does premultiplied alpha. Everything else - chrome, text, scrollback slide, background image + blur + scrim - is the shared native path on both.
	- On Windows, transparency means presenting through the desktop compositor: a DX12 swapchain on a DirectComposition visual, with no redirection surface under the window. A swapchain made straight from the window only composites opaque, and the backend picked by default varies per machine, so DX12 is pinned whenever the setting is on. Both are fixed at window creation, so the setting takes effect on the next launch there.

	- Wayland coverage: smooth scrolling is identical on both engines. The scroll regression harness runs its scenes a second time under a headless `cage` kiosk (`run.bash --wayland`). Per-pixel transparency and dialog stacking on Wayland are not yet exercised (follow-ups).

- Pixel-precise input: touchpad gives true pixel deltas; notched mouse wheel snaps to lines (clamp/accumulate notch deltas into smooth target).

### Startup and slow external resources

- Nothing on the path from launch to the first frame may read an external resource that isn't needed to draw that frame. A wallpaper folder can be a network share, a synced collection or anything else that answers slowly or times out, and a terminal that waits for it is a terminal that hasn't opened yet.

- The wallpaper is the whole of that category today: scanning the rotation folder, reading the shuffle history, decoding the image, blurring and contrast-flattening it, and reading its layout tags. All of it runs on a worker thread. The window opens and the shell starts immediately, and the wallpaper appears when it is ready. That visible gap is an accepted trade, since the alternative is a window that may never open at all.

- Each request gets its own thread rather than sharing a long-lived worker. A request stuck on a dead mount can never be canceled, so a shared worker would leave every later request stuck behind it. An abandoned thread costs almost nothing, and a stale result is discarded on arrival.

- The config file itself is a deliberate exception. Window size, font metrics and theme all come from it, and the window is held hidden until it can open at its final size. Reading it later would only trade a small local read for a visible resize flash.

- The same shape is intended for shell discovery when that lands: draw first, scan for installed shells afterwards, fold in what was found.

### Configuration format

- The user config uses SHCL (the sister project), replacing TOML. The file is `config.shcl`. The reference parser is a single zero-dependency crate, so dropping `toml`, `toml_edit` and `serde` made the shipped binary smaller rather than larger.

- The deciding property is forgiveness. A malformed line yields a diagnostic and is skipped, so one bad value costs only its own setting. Strict TOML could instead fail the whole document and sink every setting to its default. Forgiveness let two workarounds be deleted outright: a retry loop that blanked offending lines and reparsed, and a rewrite pass for leading-dot floats, which are valid here.

- Values are typed by the reader, not the file, so there is nothing to get wrong in the syntax and a value is stored back exactly as written.

### Variables in a setting (2026-08-30)

- A setting that names a path or a program is text SilkTerm reads. No shell ever sees it, so nothing else would expand a variable written there.

- Among the options, it was decided that all three spellings are accepted on every platform: `$NAME` and `${NAME}`, `%NAME%`, and `$env:NAME`. Which shell a person prefers should not decide whether their config works, and a config file gets carried between machines.

- A few names mean the same thing under a different spelling, and those are paired: HOME with USERPROFILE, USER with USERNAME, TMPDIR with TEMP and TMP. Native Windows sets no HOME and unix sets no USERPROFILE, so without the pairing a config written on one box goes quiet on the other.

- Names without an honest counterpart are not guessed at. An unpaired name that is unset expands to nothing, the way a shell does it, which the user can see. A wrong guess would be worse.

- A command is split into arguments before its words are expanded. That keeps a variable holding something like `C:\Program Files\...` as one argument.

### Command-line options

- Most options describe a window to open: a hierarchy of tabs and panes built with create/select verbs, with look and behavior cascading window -> tab -> pane.

- A second, much smaller family only prints something and exits. `--help`, `--syntax`, `--about`, `--donate` and `--version` never open a window, never read a config, and never touch a layout.

- Those flags are accepted in any position. The rest of the grammar cares a great deal about order, but answering a request for the help with a complaint about where the flag was written would be absurd.

- Output written for a person is padded with a blank line above and below, so it sits clear of the shell prompts either side of it. `--version` is the exception, and it exists to be captured by a script, so it stays a single flush line. `--ver` and `-v` are the same flag.

- Every build carries a build number, because a version cannot identify one. Two dogfood builds of the same release share a version, so a report of "it still does this on beta3" names something that could be any of a dozen binaries. The number is whole minutes elapsed since 2000 began, written in Crockford base 32 in lowercase: five characters until 2063, it sorts in the order the builds were made, and it decodes back to the minute one was built. Crockford's alphabet leaves out i, l, o and u, so nothing read off a screen and typed into a bug report can come back as a different character.
	- It appears in `--version`, in `--about`, in Help > About, and in the release notes.
	- The pipeline pins one number for a whole run, so the four cross builds of a release report the same build rather than four made minutes apart. Without that the release notes could not name one.
	- Unchanged sources keep the number they had. The binary did not change, so neither should what it calls itself.
	- The release notes take the number out of the artifact being published rather than computing it again, so the notes cannot name a build nobody can download.

- Where a shell starts is decided by four things, most deliberate first: `--directory` on the command line, then the directory inherited from the pane a new tab/pane/window came from, then the directory SilkTerm itself was launched from (only when a shell launched it), then the `shell.startup_directory` setting. `--directory` cascades window -> tab -> pane exactly as `--shell` does, so the flag that names a shell and the flag that says where it starts behave alike.

- `--about` reports what a bug report needs: version, build number, which of the cross builds this is, and the GPU the renderer picked. It asks the graphics stack for an adapter but never builds a device, which is the expensive half. A box with no usable adapter loses three lines and still prints the rest.

- On Windows a release build owns no console of its own, so printing has to join the one that launched it. This happens only on the paths that print and exit; a terminal window that held a console would die with the shell that started it.

- The contract on saving is that a user's comments and blank-line grouping survive. Layout may be tidied, meaning indentation and quotes that are not needed, but a value is never rewritten. The shipped template is deliberately spelled the way a save would spell it, so the first save is a no-op rather than a reflow of the file we just wrote.

- A save writes through a temp file and a rename, never in place, so a crash mid-save cannot leave a truncated config. If loading had to drop a line it could not place, the save is refused rather than quietly deleting it. One changed setting is not worth a line someone wrote.

- The template's sections follow the Settings dialog's tabs, in the same order, so a person who has learned one has learned the other. That order reaches a new file only. An existing config keeps whatever order it has, since the machinery that adds new settings places them but never moves what is already there.

- The file is organized as nested blocks, tab-indented, mirroring how the settings relate: `wallpaper` holds its children, with `rotate` and `contrast_mask` nested inside it. A setting can also be written as a single dotted line (`wallpaper.opacity: 0.1`) and reads identically. The block form is just the canonical spelling.

	- Each setting stands in its own blank-line-delimited section, comments directly above it: a short title, a description, and a range line where one applies. A commented-out line shows the built-in default and carries a `## Default` marker.

	- The in-place add/refresh passes stay line-oriented. They resolve each line's full path from the indentation around it, so a new setting is inserted inside the right block, beside its siblings.

- No migration from the old TOML configs: a fresh file is generated with defaults.

- When the flat naming gave way to nested blocks, an old config converts wholesale rather than being rewritten in place. The old file is kept alongside as a backup, a fresh current-format file is written, and every value the user had set carries over to its new place. Rewriting flat lines into blocks would have shredded the old file's comments. This way settings survive and the file's documentation is current.

- A config carries the commented default lines it was first given, so when a default changes those lines start describing the old behavior. Such a line is refreshed to the current default. The file may be corrected about what the program does on its own, but never about a value that was set by hand. A line the user activated, or annotated, is therefore left alone.

- Starting over is a rename rather than a delete. `--reset-config` moves the file aside and lets the next launch write a fresh one, so the previous settings stay recoverable.

- Some defaults are better inferred from the config directory than stated in the file. A folder of wallpapers sitting in the expected place is taken as wanting them rotated, without a setting to turn it on and without writing anything back. The inference yields to anything explicit: a wallpaper named in the config, or one given on the command line for a single run.

- A wallpaper image can carry its own layout and look in its XMP metadata, under a `wallpaper` namespace named for what the tags describe rather than for this program, so any tool can write them. `Fit` and `Anchor` are absolute, since how an image should be cropped is a property of the image. `Opacity` and `Blur` are absolute too, in the same units as the two settings, and replace them for that image. The shipped pack carries the program defaults on every image, so the two sliders only reach untagged images until the switch is turned off; that trade was accepted so an image's look means the same thing everywhere. Each pair has its own switch in Settings, on by default, and a missing or unreadable tag always falls back to the setting rather than failing the image.

## Delivery (CI/CD, branches, releases)

Guiding constraint: GitHub is dumb git hosting plus optional release storage, nothing more. No hosted CI, no Actions, as few third-party tools as possible; the whole pipeline runs locally (`cicd/cicd.bash`).

- Merge gate: `cicd.bash --gate` (fmt check, clippy with warnings as errors, tests) runs as the `pre-push` hook for pushes to main or dev. This is the local stand-in for a hosted CI workflow; feature-branch pushes are not gated.

- Version-bump guard: the same `pre-push` hook blocks a push to main unless its `source/Cargo.toml` version is a strict increase over the version already on main, by full semver precedence including prerelease ordering. So a release merge can't ship the same-or-lower version. It also requires the README Release badge to match that version, the same check `release.bash` makes, just earlier. It skips on the first main push and on branch deletes, and is overridable with `--no-verify` / `SKIP_GATE=1`.

- Branch flow: feature branches merge `--no-ff` into `dev` (the integration target). `main` is release-only: merging dev into main cuts a release.

- Releases: `cicd/utility/release.bash` tags the merge `v<version>` and can push the tag and attach the artifacts to a GitHub Release as plain uploads. The version comes from `source/Cargo.toml` alone. The tag is read from it and the build stamps from it, so they can never disagree. Version and README badge get bumped on dev before the release merge; nothing is ever committed directly on main.

- Build matrix (buildable from this Linux x86_64 box): Linux x86_64 (native) and, via `cargo-zigbuild` + `zig`, Linux ARM64, Windows x86_64 (mingw), Windows ARM64. macOS and BSD are deferred, since cross-building them needs an Apple SDK (osxcross, license-gated) or a FreeBSD sysroot, neither present here. The debug build is what the tests and profiler run against, and the optimized release builds feed packaging and dogfooding. ARM64 targets are on by default, since zig cross-builds aren't emulated and so are not much slower, and they drop out with `--no-arm`.

- Windows x86_64 toolchain for releases: the gnu (mingw) build is the canonical shipped Windows x86_64 binary, and the msvc build is deliberately left out of the Linux-cut release. The gnu build cross-builds from this Linux box in the same run as everything else, and is self-contained. msvc can only be built on Windows, since it needs `link.exe`, so folding it in would mean a Windows->Linux binary hand-off. Since the msvc build was made crt-static it no longer offers end users anything gnu doesn't. Its remaining edges, PDB/WinDbg debugging and a standard ABI, are dev-side only. `cicd-win.ps1` still builds msvc on Windows for local dogfooding and debugging. Two things would reopen this: Authenticode code-signing, whose natural home is Windows and would pull Windows-installer finalization onto that box; or evidence that mingw binaries trip antivirus reputation heuristics enough to matter. `makensis` itself is host-agnostic, so building the Windows installers on Linux is a non-issue independent of this choice.

- Packaging (pipeline stage 6, when `--quick` is not passed): built from the stage-5 release binaries, never rebuilt. Linux -> `.deb` (cargo-deb) and `.rpm` (cargo-generate-rpm) per arch, driven by `[package.metadata.deb]` / `[package.metadata.generate-rpm]` in `source/Cargo.toml`. Windows -> one self-contained NSIS installer `.exe` per arch (`cicd/packaging/windows/installer.nsi.in` + `makensis`). It upgrades an existing install in place by running the old uninstaller first, and needs no bundled runtime because the binary links only system DLLs. RPM versions can't contain `-`, so `1.0.0-beta1` is emitted as `1.0.0~beta1`. AppImage/Flatpak and the deferred macOS `.dmg` / BSD packages are future work.

- Artifact naming (stable; download links depend on it): `<exe>-<version>-<os-arch>[.exe]` for binaries, `<exe>-<version>-<os-arch>.{deb,rpm}` and `<exe>-<version>-<os-arch>-setup.exe` for packages, plus `<exe>-<version>-sha256sums.txt` (covers binaries and packages), all collected into `cicd/artifacts/release/`.

- Pinning: `rust-toolchain.toml` pins rustc/clippy/rustfmt and the cross targets. The cargo-installed helpers (cargo-deny, cargo-zigbuild, cargo-deb, cargo-generate-rpm) and makensis are pinned in `cicd/config.bash` (`TOOL_PINS`) with a non-gating drift warning. Dependency freshness is a periodic local `cargo update` pass, and cargo-deny advisories flag anything urgent in every run.

- README badges: static shields only (release, license, minimum Rust). No CI badge, since there is no hosted workflow to point one at.

- Wallpaper gallery on GitHub Pages: `docs/` is served from main, and holds one self-contained page - a thumbnail grid whose tiles open the wallpaper full size in place, with prev/next paging, a filter box and per-image provenance. A README cannot do this: GitHub renders no scripting and strips image maps, so a single contact sheet has no clickable tiles and there is no way to page through anything. It stretches the guiding constraint above and does so knowingly - Pages here is branch-served static files with no Actions workflow, GitHub builds nothing, and if it were switched off tomorrow the only casualty would be one README link. The page carries thumbnails only (about 1.4 MiB) and fetches each full image from the pack already in the repository, so the 60 MiB of wallpapers is never stored twice. Both it and the README contact sheet come out of `cicd/utility/wallpaper-gallery.bash`, which is deliberately one entry point: two rendered artifacts from one pack go stale together or not at all.

### A tab can be named by hand, and the window title follows the tab (2026-08-30)

Double-clicking a tab renames it in place. The strip has always drawn what the shell is doing, which is right most of the time and wrong when several tabs are running the same thing in the same tree.

- The edit starts with what the tab already says, all of it selected, so typing replaces it and any other key edits it. Enter or Tab keeps the change, Escape drops it, and a click anywhere else keeps it. Selection, Home and End, and paste all work; a pasted newline becomes a space, since a tab is one line high.

- Committing a title that matches what the tab would have said on its own puts it back to naming the shell. That is the way out of a hand-typed title, and it needs no extra control.

- A title left empty is kept, so a tab can be deliberately blank. The tab shrinks to its close box and is still selectable.

- Titles need not be unique. Two tabs called the same thing is a thing people do on purpose.

- The rename is keyed by the tab's position, so opening or closing a tab ends it - committed on an open, dropped on a close.

The window title is now assembled in one place, and always starts with the application name. A dogfood build says which one it is, since the pool holds several and they are otherwise indistinguishable in the taskbar.

- After the name comes, in order: a title typed on the tab, else the title the running program asked for, else what the tab says about the shell. So a program that renames the window (an editor, a build tool) reaches the title bar without touching the tab, and a hand-typed tab title outranks it.

- A tab deliberately blanked lets the program's title through, and with neither the title is just the application name. That is the one case where blank means "defer" rather than "show nothing".

- A `--title` given on the command line is still the whole answer, verbatim. It is an explicit request for exactly that string.

### Tabs report what they are running, and where (2026-08-21)

A tab used to say the application's own name on Windows and the shell's process name on unix. It now reports the shell by its FRIENDLY name - the one the Shells list carries, which is the name the user gave it - followed by what that shell is doing: the command in the foreground, or the last one it ran, or, having run nothing, the directory it is in.

- The shell a pane runs is resolved once, when the pane is spawned. Leaving it as "whatever the default shell is" let the answer change under a running pane, since the background scan fills the list seconds after launch and the Shells tab reorders it - so a pane could be labelled with a shell it was not running.

- The path is shortened the way PyCmd's prompt does it: directories above the current one drop to their initials, and only if that is still too wide does an ellipsis eat the middle. Two things survive every step, because they are what distinguish a location from a command - the anchor it starts from and the separator it ends with. Windows keeps its drive letter and gets no `~`, since neither shell there prints one.

- Tab width is two percentages of the window rather than a fixed cap, so the extra text has room on a wide display while a lone tab still reads as a tab. See the entry below for what those two now mean.

- When the tabs stop fitting, the strip shows a page at a time rather than shrinking them to nothing. The wheel over the tab bar turns the page, and switching tabs brings the new one onto it.

- A hover tip carries what the tab cannot: the shell's name, the command line behind it, whatever is running now, the whole path, and how long the tab has been open. It reads as a table - one `key: value` per line, every value starting in the same column - which is why it is the one piece of chrome drawn in the TERMINAL font rather than the interface one: the column is made of spaces, and spaces align nothing in a proportional face. A value carrying a space or a quote is quoted, so its edges are never in doubt; the quote picked is the one the value does not already contain, the same habit the config file has, so a Windows command line reads inside single quotes instead of fighting its own double ones. What is derived rather than quoted - the clock reading, and the note that no directory was reported - stays bare, since quoting those would say they were data.

### A tab is as wide as its own label needs (2026-08-23)

Tabs used to divide the bar evenly between a minimum and a maximum percentage of the window, so every tab was the same width whether it had anything to say or not. They now size themselves.

- The first percentage is the REGULAR width: what a tab is when nothing is pushing on it. It is a target, not a share - three tabs on a wide bar sit at it and leave the rest of the bar empty, rather than a couple of them stretching across the window.

- A tab whose label wants more room grows past it, up to the maximum. A crowded bar pushes every tab back below it. Everyone reaches the regular width before anyone grows past it, so a long path can never cost another tab its ordinary size, and under crowding each tab gives up the same fraction rather than the last few being starved.

- Defaults are 10% regular and 100% maximum. The old pair (8% and 26%) made sense when the bar was divided evenly; a maximum now only says how far one tab may grow when it has the room, which is worth allowing in full for a window holding a single tab.

- The floor a tab may not shrink past is its own shortest label - a short form of the shell's name and nothing else. Tabs past that point become a page.

- What a tab says now gives way in a fixed order, rather than only the path shortening: the shell's name shortens first, then the running command's name is truncated, then the path abbreviates, then the command goes, then the path, and what is left is the shortest form of the shell's name. The path is shown alongside the command now, where before a tab running something said only what it was running.

- Short shell names are hand-picked for the shells we ship ("Windows Cmd" reads "Cmd", "PowerShell 7" reads "PS 7") and derived for anything renamed, since nothing mechanical arrives at "Cmd" from "Windows Cmd". A derived name keeps its distribution rather than its family ("WSL2; Ubuntu" reads "Ubuntu") and marks a variant with a star, so "Zsh" and "Zsh*" at least say that one of them is not the ordinary one.

### PowerShell gets the same prompt bash does (2026-08-21, reworked 2026-08-30)

The integration block sets a prompt, but only where the prompt is still the one PowerShell ships, identified by the help link its own definition carries. Anything anybody else installed is left alone.

It began as a prompt that named the version, because two PowerShells look alike at a prompt. It now reads the same as the bash prompt described above: version, time, user, host, path, and in a git working tree the remote, the branch, and two marks for committed and level with the upstream. A PowerShell pane and a bash pane should look like the same terminal.

Two decisions came out of the port.

- It lives in the block rather than in a script beside the config, which is where the bash prompt lives. A prompt is drawn after every command, and a script would mean a process per prompt - cheap on unix, not on Windows. The block is already kept up to date in place, so it carries updates just as well as a file would.

- The block stays plain ASCII, and the check, cross and arrow are written as code points. A file with no byte-order mark is read as ANSI by Windows PowerShell 5.1, which would mangle a literal glyph on the one version that cannot be told otherwise.
- The console is put on UTF-8 at load. That is not what lets the prompt draw its glyphs, since PowerShell writes the prompt as wide characters and the code page has no say in it. What it buys is the decoding of output from `git` itself, where a branch name outside ASCII would otherwise arrive wrong.
- The second line is bare where the bash prompt puts an arrow. The `>` already says where the typing goes, and the arrow the bash version uses is a code point few fonts carry.

- The check is U+2713 rather than the U+2714 the bash prompt uses. U+2714 has an emoji presentation, so it is drawn by a color font in its own color and ignores the reverse-video the mark is set in. U+2713 is not an emoji code point at all, so it takes the color the way the cross beside it does.

Cost was the other thing the port had to answer, since three `git` calls per prompt is invisible on unix and not on Windows. The search for the working tree is done in the shell rather than by asking git, so a directory outside a repository costs no process at all, and inside one a single `git status --porcelain=v2 --branch` answers branch, clean and upstream together. The remote URL is read once per repository and remembered.

The block is also kept up to date in place from then on, between its two markers. It gains things over time, and an install that only ever appended would leave everyone who already had it on the first version forever. That edit is safe only because the region is delimited by markers we wrote - which is exactly the signal the stored shell list lacks, and why that list may still only ever be added to.
