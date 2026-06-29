<!-- markdownlint-disable MD007 -- Unordered list indentation -->
<!-- markdownlint-disable MD010 -- No hard tabs -->
<!-- markdownlint-disable MD033 -- No inline html -->
<!-- markdownlint-disable MD055 -- Table pipe style [Expected: leading_and_trailing; Actual: leading_only; Missing trailing pipe] -->
<!-- markdownlint-disable MD041 -- First line in a file should be a top-level heading -->

<!-- TOC ignore:true -->
# Smooth-scrolling terminal - Backlog

This is a product backlog just for pre-v1.0.0 release. After that, bugs, features, and enhancements will be mananged in Github Issues, and/or [todo.md](../todo.md)

<!-- TOC ignore:true -->
## Table of contents
<!-- TOC -->

- [First steps](#first-steps)
- [Conventions](#conventions)
	- [Bugs](#bugs)
	- [New features and enhancements](#new-features-and-enhancements)
	- [Deferred](#deferred)
	- [Won't do](#wont-do)
- [Application name ideas](#application-name-ideas)

<!-- /TOC -->

## First steps

- [✔️] Pick name, create GitHub repo.
- [✔️] Cargo skeleton: `alacritty_terminal` + `wgpu` deps.
- [✔️] Glyph atlas + cell render.
- [✔️] Wheel input -> lerp target.
- [✔️] Boundary-cross sync to `scroll_display`.
- [✔️] Overscan rows for partial-row fill.
- [✔️] Output-scroll easing.
- [✔️] Verify smoothness on X11/Compiz.

## Conventions

In each section, items are listed approximately from newest to oldest.

Mark boxes with ✔️, 🚫, or ◐. Empty means not started, or WIP.

### Bugs

- [ ] Critical: Smooth-scrolling apparently just quits after using the terminal for a while. It seems to quit, if output is too fast for a while, but that could be a red-herring. Maybe it's just after any particular amount of general use.

- [✔️] Mouse wheel doesn't scroll back through the `stdout`/`stderr` buffer. It should do so, smoothly, and in proportion to how fast the mouse wheel is moved. But currently it moves the command history back. (20260626-104542)
	- Cause: `TermMode::ALTERNATE_SCROLL` (DECSET 1007) is default-on in alacritty_terminal, but the wheel handler used `ALT_SCREEN || ALTERNATE_SCROLL` as the cursor-key trigger - so on the *primary* screen the always-on flag made the wheel emit cursor-up/down (shell history recall) instead of scrolling scrollback.
	- Fix: gate the cursor-key path on `ALT_SCREEN` (now requires alt screen AND alternate-scroll AND no mouse mode). The primary screen always routes to the smooth scrollback (`Scroll::wheel`, already proportional to notches via `wheel_lines` + easing). Alt-screen apps (less/nano/vim) keep their cursor-key wheel. `app.rs` MouseWheel arm. Verified by root-cause + build (runtime wheel injection is unreliable here per xdotool notes).

- [✔️] Severe bug: Trying to open the settings dialog crashes the program. (20260625-150526)
	- Cause: with always-GL on X11 the main window holds a glutin GL/EGL context, and the pop-out dialog's `Gfx::new` created a second `wgpu::Instance::default()` (all backends, including GL); wgpu-hal's GL init then panicked in EGL teardown (`unmake_current().unwrap()`, "Another window API already has a current context"). Increment A/B tests used a native-Vulkan main (default config), which masked it; the transparent (GL) main hit it every time.
	- Fix: dialogs now create their `Gfx` via `Gfx::with_backends(window, Backends::PRIMARY)` (Vulkan/Metal/DX12, no GL) - opaque dialogs don't need GL, and avoiding it sidesteps the EGL conflict. Verified: Settings + About open over a transparent GL main with no crash; toggle on->Opacity enabled, off->greyed.

- [✔️] Mouse text selection, and double-click selection, quit working. (20260625-161509)
	- Cause: It was actually the selection highlight being invisible (input + copy-to-PRIMARY worked): the GL offscreen was `Rgba8UnormSrgb`, so the blit's `textureSample` decoded sRGB->linear, cancelling the blit's `lin2srgb`, and wgpu's GL backend doesn't sRGB-encode the offscreen write either - so all rect/text colors passed through as raw linear and rendered too dark (text ~64% looked "ok"; the dark `SELECTION_BG` (51,68,102)->(8,15,34) went invisible). Fix: make the GL offscreen plain `Rgba8Unorm` so shaders store their linear output raw and the blit's `lin2srgb` does the one true encode - uniformly for rects, glyphon text, and the bg image. Verified on-screen: SELECTION_BG renders (50,69,102), text is full-brightness (210). This also completes the earlier transparency sRGB fix (text was still ~164, now a true 210).

- [✔️] Smooth scrolling is broken. (20260623-194551)
	- Cause: the fix for the apt "bug". That fix made output easing snap whenever new lines arrived closer than 0.12s apart, to stop apt's status bar bouncing. But a command's output arrives from the PTY in one sub-millisecond burst, so essentially all multi-line output (the core demo) snapped instead of easing - smooth scroll gone. Any burst threshold above a frame breaks the feature.
	- Fix: Reverted the burst-snap entirely (`Scroll::nudge_output` back to always easing while following; dropped `output_age` / `OUTPUT_BURST_GAP_S`).
	- Verified: Smooth output scrolling restored. The apt status-line bounce is reopened below as its own item (needs a non-destructive approach).

- [✔️] "Close pane" menu items don't work.
	- Cause: The action itself works with multiple panes (verified: right-click and Panes-menu Close both closed a pane, 3->2->1). The dead case was the last pane: `MenuAction::Close` was gated on `panes.len() > 1`, so on a single pane (the startup state, where you'd first try it) it silently did nothing.
	- Fix: Now Close Pane on the last pane closes the tab (if >1 tab), else the window.
	- Verified: single pane + single tab -> Close Pane exits.

- [✔️] Text background colors, and the block cursor, appear to be aligned a line below where they should be.
	- Cause: Regression from the menu bar. Cell backgrounds, the cursor, and the bars are all rect quads in absolute framebuffer pixels (same space as the glyphon text viewport), but `rects.set_resolution` (and the bg-image shader) were being fed the content `area` height, which the menu bar made shorter than the window - so the shader's `px.y / resolution.y` mapping pushed every quad down relative to the text.
	- Fix: Pass the full window size (`gfx.config.width/height`) to both `set_resolution` calls.
	- Verified: ANSI bg-color spans sit exactly on their text and the block cursor is on its own row.

- [✔️] The text and UI elements in the settings dialog are misaligned. But before fixing it, make sure we're not going with egui.
	- Cause: the dialog vertically centered text with a baked-in 18px text height, so on fonts whose line height differs the labels/values didn't line up with their controls (and it used the mono font).
	- Fix (folded into the Settings enhancement): `SettingsDialog::texts(line_h)` now centers every label / value / hex field / button against the actual rendered line height (the app passes `cell_h`), and the app draws them with the proportional `sans_attrs()`.
	- Verified: labels, sliders, values, swatches, hex fields, and buttons all align. (Also decided, not going with egui.)

- [✔️] If the window isn't just the right hight, the last line of text is invisible. Not as in, below the visible line - but actually invisible. If you type, you can see that output happens, it's just not visible. Once it scrolls up even a single line though, it becomes visible. Adjust the hieght of the window just a tad, it "fixes" the problem. But at the default dimensions, the problem is apparent.
	- Cause: `Pane::build` lays out lines+1 rows into the pane's text buffer (the screen-row -1 overscan row above the viewport, plus rows 0..lines-1), so the bottom row sits at `y = lines*cell_h`. The buffer was sized to the content height (`lines*cell_h`), so when the window height made content an exact multiple of `cell_h` - which the default 152x48 does - that row landed right at the buffer's height limit and cosmic-text dropped it from layout (the cell bg/cursor quads use a separate renderer, so they still showed - hence "type and output happens but is invisible"). Scrolling/resizing shifted it back into range, "fixing" it.
	- Fix: size the pane buffer to `content_h + 2*cell_h` (overscan slack) in `spawn_pane`/`relayout`; `TextArea` bounds still clip drawing to the pane.
	- Verified: bottom prompt line renders at the default size.

- [✔️] There are weird spacing issues with the cursor. It appears too far after text. There are also weird text background color interactions with `ble`, which I suspect is caused by the spacing issue.
	- Cause (re-fixed): the earlier two-part fix below was incomplete because `cell_w` was rounded (measured pitch ~10.5px -> 11). Everything grid-positioned (cursor, cell bg, per-cell glyphs) is placed at `col*cell_w`, so a `cell_w` that's bigger than the text's actual advance drifts them right of the text, and the drift accumulates per column. The cursor sat further past the text the longer the line, and fallback glyphs landed on top of the next cell at higher columns (`set_monospace_width` doesn't snap here. Cosmic-text only snaps when the font's `monospace_em_width()` is `Some`, which system fonts often aren't, so text renders at its natural advance).
	- Fix: `measure_cell` now measures the real rendered pitch (`Shaping::Advanced`, averaged over 40 `M`s) and `cell_w` is not rounded -> it matches the text, residual drift is sub-pixel. Plus per-cell fallback glyphs are now fit to their cell box: `fill_glyph` returns the glyph's true ink width/offset (rasterized; advance != ink for `➡`/emoji) and `build` scales+centers each via `TextArea.scale` so an over-wide fallback can't spill onto its neighbour. Verified at runtime (pixel-measured): cursor sits one cell after `…$ ` with no drift; `A➡B➡C…` and `A😀B…` align at col 0 and col ~40; CJK/emoji = 2 cells; math/box-drawing stay in-buffer and crisp.
	- (Earlier partial fix, superseded by the above) 1) `set_monospace_width(cell_w)` in `TextCtx::new_buffer`; 2) pulling glyphs the primary mono face lacks out of the main buffer and drawing them per-cell. The extraction [2] is still in place; [1] is kept but is largely inert for system fonts.

- [✔️] Opacity should only affect the text rendering area, the actual terminal. Instead, it is also affecting the entire window including window decorations.
	- Cause: the early build leaned on whole-window opacity, which by definition dims the decorations and text too. What's actually wanted is per-pixel surface alpha, and wgpu can't drive that on X11 directly (its Vulkan swapchain forces an opaque surface; its GL backend won't bind the ARGB visual).
	- Fix: Done - solved via the glutin + wgpu-hal GL-interop path (see "True transparency" below). Opacity now affects only the terminal background; text, decorations, and chrome stay opaque. The old whole-window opacity route was removed.

- [✔️] Config file values don't work without a leading 0.
	- Cause: `.25` is invalid TOML, so the whole file failed to parse and every value reverted to default (hence "all values").
	- Fix: `config::lenient_floats` rewrites a bare-decimal value after `=` (`.25` -> `0.25`, `-.5` -> `-0.5`) before parsing.
	- Verified: `opacity = .25` now applies and other keys still load.

- [✔️] The font size is still smaller than the system monospace size.
	- Causes:
		1. `config.toml` pinned `font_size = 15.0` (from an older template), overriding the new follow-the-system default.
			- Fix: Commented it out so detection applies.
		2. "Use system monospace" had only meant cosmic-text's generic `Family::Monospace`, not the OS-configured family, so even at matching point size the glyphs looked smaller/different.
			- Fix: Now `sysfont::monospace()` also returns the configured family (Pango/`defaults` parse, style+size stripped) and `resolve_mono_family` pins it when it's actually installed (validated against the font db), else falls back to generic monospace.
	- Verified: renders Monaspace Argon at 13pt (cell 11x21, window 1680x1016).

- [✔️] Text sometimes renders in a different font (e.g. when running `source x9ps1-git; export X9PS1_STANDARD=1`). It seems that some color control codes causes the font change.
	- Cause: the prompt sets bold (`ESC[01;..m`), and cosmic-text's generic `Family::Monospace` resolves the best face per query, so a bold run landed in a different family than the regular run.
	- Fix: resolve the concrete monospace family name once at startup (`text::resolve_mono_family`) and pin `Family::Name` for every weight, so bold/italic stay in it.

- [✔️] Text size is smaller than system default monospace.
	- Fix: Default font size now follows the OS's monospace/fixed-pitch size instead of a fixed 15px, via per-platform detection in `src/sysfont.rs`: Linux `gsettings` -> `fc-match`; macOS `defaults read -g NSFixedPitchFontSize`; Windows `SystemParametersInfo` message-font (windows-sys FFI). Points->px at 96 DPI. `font_size` in the config is commented out by default (follow system); set it to pin a size. Falls back to 17px when detection fails.
	- Verified: Linux at runtime (13pt -> 1528x1016), Windows cross-build compiles; macOS path is std-only subprocess, not run-tested here (no mac target).

- [✔️] Native keybindings for `less` don't work.
	- Fix: `less` enables application-cursor-keys mode (DECCKM); arrow / Home / End are now encoded as `ESC O x` instead of `ESC [ x` when that mode is active. The mouse wheel also now drives full-screen apps: when the alternate screen / alternate-scroll mode is active it sends cursor-key presses instead of moving the (nonexistent) scrollback.

### New features and enhancements

- [ ] Cursor:
	- [ ] Smooth-scroll (to the right) also.
	- [ ] Blink at the same rate, but "phase" between of and on, not just on or off.

- [ ] CI/CD scripts:
	- [ ] Build alternate targets in parallel.

- [◐] Menu bar: (issue #t6thx, 20260626-132615)
	- [ ] Auto-adjust height based on menu font size.
	- [✔️] Make menu gray, with white text. (For both light and dark themes.)
		- The menu / tab-bar / context-menu chrome consts (`MENU_*`, `TAB_*`) are now neutral grays with near-white text, fixed across modes (per #166 default).
	- [ ] Menu color is user-adjustable, even per-theme. It's just that all themes by default use the same menu colors.

- [◐] Themes:
	- Status part 1: Done. (`src/theme.rs`): theme foundation + terminal palette done. A `Palette` (bg/fg/cursor/focus + 16 ANSI) x a `Theme` (dark+light pair); `theme` + `theme_mode` config keys resolve the active palette, which `palette.rs` + the renderer read. The `[colors]` keys still override per-colour. 3 built-ins (SilkTerm, Matrix, Retro Amber), each dark+light.
		- Verified: Matrix = green-on-black incl. green-toned ANSI; SilkTerm light = dark-on-light.
	- Status part 2: Done: chrome/dialog theming + System mode. Dialogs (Settings + About) adapt via `config::is_dark()` - dark-gray panel/text for dark, light-gray for light (a `Dlg` dark/light set in settings_ui); the menu/tab chrome is a fixed neutral gray (#165). "System" mode follows the OS via winit's `Window::theme()` at startup + `WindowEvent::ThemeChanged` -> `config::reapply_for_os` (falls back to dark where the OS reports no preference, e.g. X11).
		- Verified: light mode -> light-gray Settings dialog with dark text; menu is gray; system mode launches clean. still TODO: config-defined `[themes.*]`; the Settings theme dropdown + its own tab; clearing per-colour overrides on re-select; per-theme menu colour (#166); more themes (Pastel, Solarized).
	- [◐] Provide a set of about 3 or 4 themes, each that support "Dark" or "Light" mode (or "System"). - 3 built-ins with dark+light done; "System" (OS-follow) + a 4th theme pending.
		- Dark mode means the background is dark, text light - both for the terminal, and dialogs.
			- But dialogs have a different color than terminal background. E.g. the existing dark gray for Dark mode, light gray for Light mode.
		- Light mode means light background, dark text.
		- "System" means whatever mode the system is using.
		- Theme definitions should be put in the default config file.
		- Selecting a theme overrides custom color settings, but those can then be individually tweaked as overrides (until a theme is chosen again and tweaks overwritten).
		- Themes and colors should probably go on their own settings tab.
		- User can add themes in the config file. Theme dropdown in Settings UI pulls from those updates.
		- Example themes:
			- Matrix (bright green on black). Light mode: dark green on light gray.
			- Retro amber (Orange on black). Light mode: dark orange on light gray.
			- Pastel (a pleasing light pastel color, on dark gray background that has a subtle tint of complementary pastel).

- [ ] Settings dialog:
	- [ ] Some more space between sections, so otherwise it seems run together.
	- [ ] As the number of settings may grow, we need a way to manage increasing length. Can't go beying about 1048 pixels high, including window decorations. (So roughly 1010 pixels total to be safe.) Options: (20260626-102933)
		- Make the Settings window shrinkable and then add scrollbars only when necessary, so that it won't render beyond allowable space.
		- Group sections into logical "super-sections", and put them into tabs.
		- Better yet, both solutions.
	- [ ] Every setting in Settings dialog should have a clickable icon to "Revert to default". This icon should also indicate if the setting is default, and only be clickable if it's not. (20260626-102000)
		- In the config file, if user clicks "Revert to default" in settings, set the value to default and comment it out.
	- [ ] "Use system font" boolean should be visible checked, if using it.
	- [ ] Clicking on the font text field, should immediately clear "(system default)". But it should reappear when focus lost, if nothing entered.
	- [ ] Editable fields should have a visible cursor when focused, and respond to standard text-editing key controls.
	- [ ] Full keyboard control, e.g. tab order, full text field editing, alt+down for dropdowns, space to toggle booleans, etc.
	- Note: It might be best to defer some of these, until after (and if) native window controls are implimented.

- [ ] "Reload config" should re-read the background image too. In case user changed the image and kept it the same name. (20260626-102603)

- [ ] About dialog:
	- Include the version, build, copyright, and license.

- [◐] Menu (part 2):
	- [✔️] When a menu is open, keyboard arrow should work on them, not on the active terminal pane.
		- Fix: An open menu (context menu or menu-bar dropdown) now captures navigation keys: Up/Down move a highlighted item (`ContextMenu::step`, wraps, skips separators, reuses the `hover` field/render), Enter activates it, Esc closes, Left/Right cycle between menu-bar dropdowns.
		- Verified: arrows highlight (separators skipped), Enter->New Tab opened a 2nd tab, Esc closed.
	- [◐] When 'Alt' Pressed, keyboard accelerators should become visible on the menu (traditionally with underscores). - Open dropdowns now underline each item's first letter and a letter-press activates the first item starting with it (verified: 'n' -> New Tab). Alt+F/E/V/T/P/H already open the bar menus. still
		- [ ] TODO: show the underline on the bar titles on Alt-hold (a redraw-on-Alt + char-measure pass).
	- Note: the cross-platform-windowing-widget question (the `[🚫]` note under "Setting dialog (part 2)") is now decided - chrome stays hand-rolled (egui declined after a real spike). So the bar-title Alt underline is just a normal hand-rolled task.

- [◐] General configuration:
	- status: the default-shell behavior is done; the named shell list + its selection UI (grid editor + Tab/Pane menus) are now active hand-rolled work - the egui chrome migration was declined (see the `[x]` note under "Setting dialog (part 2)").
	- [◐] Ability to define shells to launch in a new tab or pane.
		- [✔️] By default, new tab launches the default shell for the window. - `new_tab` + the non-CLI startup pane use `config::default_shell_argv()`.
			- [✔️] By priority: Global command shell override, non-empty shell specified in config file, or system default shell. - resolution order is CLI window `--shell` -> config `default_shell` -> system; per-pane it's pane `--shell` -> the pane it forked -> tab -> window -> `default_shell` -> system. Verified: a `default_shell` in config runs on the startup pane.
		- [✔️] By default, new pane launches same shell as the pane the new one was forked off of. - `Pane` stores its launch argv; interactive splits inherit it (done with the CLI work).
	- [◐] The shell configuration is stored in the config file as a simple key:value list of shell names and command lines. Command lines may have spaces, single quotes, and/or double quotes in them.
		- Done so far: a single `default_shell` string key (argv-split via `cli::shell_split`, handles spaces/quotes). The named key:value `[shells]` list + its consumers (the grid editor + Tab/Pane menus below) are now hand-rolled work (egui declined).
		- [ ] In the settings dialog, this is accessed from a button that loads an additional modal dialog on top, with a 2*n grid of values. (That is editable like a typical database or spreadsheet grid.) This editable grid UX should be reusable for other potential future features. - Hand-rolled (egui declined): build it as a dynamic list of name|command rows + add/remove, reusing the dialog's existing text-field editing and the multi-window `DialogWin` infra.
	- [ ] The "Tab" and "Pane" menus (both on the main menu and popup menu sections) should both have dedicated sections to select the shell, both pulling from the same list of shells in the config. (With "[SilkTerm default]" always the first if one is defined in the config, and "[system default]" always the last no matter what). - Hand-rolled (no egui); follows the named-shell list above.
	- [ ] If bash is available on the system, add a shell option just above "[SilkTerm default]": "bash --norc". - Deferred with the hand-rolled shell menu above.

- [◐] Setting dialog (part 2):
	- [✔️] A radio button for background image, to stretch or zoom. - New `Kind::Radio(&[..])` in the settings dialog (reusable N-option control: indicator box per option, fills the selected, click-to-pick); a "Bg image fit" row bound to `background_fit` (Stretch/Zoom). Verified: renders with Stretch selected by default; clicking Zoom switches it; `background_fit` persists + re-fits the image on Apply.
	- [ ] Flyover help text when mousing over elements. (Make this a reusable feature.)
	- [✔️] "Default shell": A command line to launch by default for new windows, tabs, and panes, if nothing else specified. Leave blank to use system default. - New "Shell" section in Settings with a "Default shell" text field bound to the existing `default_shell` config (empty shows "(system default)"; argv-split applies to new tabs/panes). Verified the field renders.
	- [✔️] Size: A boolean setting to "Remember last size" - `remember_size` config + dialog toggle; on launch it uses `remembered_columns`/`remembered_rows` instead of columns/rows. The remembered pair is updated on every manual window resize (startup/programmatic resizes are skipped via a `size_tracked` flag set after the first frame, so they don't clobber it) and is not shown in the dialog. Columns/Rows grey out when on. Verified: manual resize -> remembered_columns/rows persisted; relaunch with remember_size=true used the remembered size (712x504, not the 160-col default); dialog shows the toggle checked with Columns/Rows greyed.
		- Overrides explicit numeric size.
		- Explicit numeric size fields disabled and grayed out.
		- "Remembered" values stored separately in config, so that user can uncheck the boolean and revert to previous numericly defined size. These "remembered" values are not exposed in the settings dialog, only exist in config file. Always update to last manual window resize, whether boolean is yes or no.
	- [ ] Should be able to tab between settings (and dialog buttons - in a loop).
	- [ ] All values, including slider numbers, should also have directly editable fields (that are part of the tab order).
	- [✔️] A little more vertical space between the section headings, and the corresponding horizontal line. - Taller heading row (`HEADER_H` 34->42); the heading text is top-aligned and the rule sits near the bottom, leaving a clear ~7px gap (was overlapping). Verified in the dialog.
	- [🚫] Adopt a cross-platform GUI / windowing widget toolkit (e.g. egui) for Settings, About, the main menu, and the context menu instead of hand-rolling them.
		- **No**. Results of spike (branch `spike/egui-dialog`): The upside is that egui 0.35 rides our exact wgpu 29 + winit 0.30 (no downgrade, shares our graphics stack) and integrated easily.
		- Drawbacks to egui: it adds ~32% to the release binary for what is secondary chrome, against the minimal-binary-size priority. Hand-rolling also keeps one unified colour/theme + native-OS-font system across the terminal and the chrome. egui would need a separate egui-`Visuals` theme kept in sync, plus its own bundled fonts).
		- Decision: Chrome stays hand-rolled.

- [◐] Command-line options:
	- Status part 1 (options engine): Done.
		- Full parser (create/select model, cascading style, shell-word-split, unit-tested).
		- `--help`/`--version`/`--syntax`; `--config` alternate file
		- Window options `--columns/--rows/--pixel-width/--pixel-height/--background-opacity/--hide-windowframe/--hide-menu/--fullscreen/--title` (window-only-after-marker errors).
		- Layout `--new-tab/--tab=/--new-pane/--pane=/--splits/--down|up|left|right/--size` building real tabs/panes (targeted splits -> arbitrary trees, smart default direction, percent/cell sizes).
		- Per-pane `--shell` (argv-exec, cascade pane->split-source->tab->window->config `default_shell`; interactive splits inherit too).
		- Config `command_line` applied when launched with no args (any real CLI argument overrides it entirely - verified both directions).
		- Tab `--title` override (`PaneManager::title_override`, shown in the tab bar - verified).
		- Deferred (queued): per-pane visual style (`--font-name/--font-size/--background-color/--foreground-color/--background-image*`) needs a per-pane renderer that the single-`TextCtx` architecture lacks. Revisited later (using hand-rolled chrome without egui).
		- `--keep-open` (needs exit-status display in a dead PTY).
		- Per-pane `--title` (no per-pane title is displayed yet - reserved).
		- Finer field-level CLI/config negotiation (current rule: presence of any CLI arg ignores the config command line wholesale).
	- General notes:
		- Command-line options override any config setting, but only while that window is alive.
		- As suggested in the main enhancement bulletpoint above, a command line can also be specified in the config file (and exposed in "Settings").
			- If the user launches the program also with command-line options:
				- Window-level options specified on the command-line at launch, override same command-line options stored in the config. (In other words, window-level options are "negotiated" between user-specified and config.)
				- If a single hierarchical option is specified by the user on the command-line at launch time, all hierarchical options from the config file are ignored.
	- [ ] General format (unless we already inherited one):
		- `--option[=| ]value` | `-o value`
		- `--unary-flag` | `--unary-flag[=| ]\(true|t|yes|y|Y|1|false|f|no|n|N|0\)` | `-u` | ...etc.
		- In other words, even unary flags can be treated as options, and important options have single unique "short" versions.
	- [ ] `--config[=| ]"alternate config file location"`
		- When active per-session, settings dialog should save to defined alternate.
		- All launches without this flag should default to existing config.
		- Configs are per-window, not per-tab.
		- Multiple windows can all have different configs specified and active. When a tab is undocked and moved to a different existing window, it automatically changes to that Window's config.
	- Window-level options (all options only apply to a single window per launch):
		- General:
			- Specifying window-level options after any tab/pane marker (`--new-tab`, `--tab`, `--new-pane`, `--pane`) should exit with an error.
		- [ ] `--columns[=| ]<n>`
			- Primary way to specify window width
		- [ ] `--rows[=| ]<n>`
			- Primary way to specify window height
		- [ ] `--pixel-width[=| ]<n>`
			- Alternate way to specify window width
		- [ ] `--pixel-height[=| ]<n>`
			- Alternate way to specify window height
		- [ ] `--background-opacity[=| ]<n>`
		- [ ] `--hide-windowframe[[=| ]bool]`
		- [ ] `--hide-menu[[=| ]bool]`
		- [ ] `--fullscreen[[=| ]bool]`
		- [ ] `--help` | `-h`
			- Show program {name, version, and build#}, copyright and license, and list options and meaning.
		- [ ] `--syntax`
			- Similar to `--help` but just list options and meaning.
		- [ ] `--version`
			- Just show program name, version, and build#.
	- Hierarchical options:
		- General notes:
			- There is always an implicit first tab and first pane, each addressable by ID "0" or "main"; a window can never have zero tabs, nor a tab zero panes.
			- Create vs. select: `--new-tab` / `--new-pane` create a new tab/pane; `--tab=<id>` / `--pane=<id>` select an existing one. ID is required on a select - there is no naked `--tab` / `--pane`. Whatever was just created or selected becomes the "current" tab/pane, and subsequent options (and `--new-pane`s) apply to it until the next create/select.
			- Selecting an ID that doesn't exist is an error.
			- All options are logically under a single implicit 'window' (it can't be specified; it just means all options apply to one window).
			- Inheritance (most-specific wins): a pane's effective value = explicit on that pane, else inherited from the pane it splits (recursively up that chain), else its tab, else the window. A tab's = explicit on the tab, else the window. Flow: window -> tab -> [pane it splits, recursively] -> pane. Handles, title, and size are non-inheritable; direction inherits along the split chain, and the style options below inherit down the whole flow.
			- Order matters: options apply to the current tab/pane at the point they appear. You may re-select an earlier entity (e.g. `--tab=0`) later in the same command line to add panes to it or change its settings.
		- [ ] `--new-tab[[=| ]<handle>]`
			- Create a new tab and make it current. Optional handle names it (unique within the window) for later `--tab=<handle>`. The implicit first tab (ID "0"/"main") always exists, so N `--new-tab`s => N+1 tabs.
		- [ ] `--tab[=| ]<id>`
			- Select an existing tab (ID "0"/"main" or a handle) and make it current - to add panes or change its settings. ID required; selecting a nonexistent tab errors.
		- [ ] `--new-pane[[=| ]<handle>]`
			- Create a new pane (splitting `--splits`, default = the current pane) and make it current. Optional handle names it (unique within the tab) for later `--pane=<handle>` / `--splits=<handle>`. The implicit first pane (ID "0"/"main") always exists and is never created by `--new-pane`.
		- [ ] `--pane[=| ]<id>`
			- Select an existing pane (ID "0"/"main" or a handle, within the current tab) and make it current. ID required; selecting a nonexistent pane errors.
		- [ ] `--title[=| ]<"Display title">`
			- Before any tab/pane marker: replaces the default window title. After a tab marker (`--new-tab`/`--tab`): replaces that tab's calculated title. After a pane marker: ignored (reserved for a possible future per-pane use; not an error).
			- Display only; not a handle, not inheritable.
		- [ ] `--splits[=| ]<pane id to split>` (alias `--splits-pane`)
			- Only valid with `--new-pane`; error otherwise.
			- Optional. Default = the current pane in the current tab (resets to "0"/"main" after every tab create/select). Splitting the implicit first pane is fine - that's the first split.
		- [ ] `--down` | `--up` | `--right` | `--left` `[[=| ]bool]`
			- Where the new pane goes relative to the pane it splits: `--down`/`--up` stack it below/above; `--right`/`--left` place it to the right/left.
			- Only valid with `--new-pane`; error otherwise.
			- Inheritable along the split chain: a later pane that splits this one reuses this direction unless it sets its own (handy for stacking a run of panes the same way).
		- [ ] Default direction when a `--new-pane` gives none and has nothing to inherit: "right" or "down", whichever has more space. ("Save layout" always emits an explicit direction rather than relying on this.)
		- [ ] `--size[=| ]<(n columns or rows | n%) of the split (parent) space in the split direction>`
			- Defaults to 50%.
				- Exception: a run of same-direction splits with no explicit size redistributes those adjacent undefined-size panes to ~equal in that direction.
			- Only valid with `--new-pane`; error otherwise. Not inheritable.
		- [ ] `--shell[=| ]"command"`
			- Can contain escaped single and/or double quotes, as logically required by whatever quotes are used around the whole command.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--keep-open[=| ]bool`
			- Keep pane|tab|window open after shell command exits, showing exit value.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--font-name[=| ]"string"`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--font-size[=| ]<n>`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--background-color[=| ]<hex>`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--foreground-color[=| ]<hex>`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--background-image[=| ]"path"`
			- No value = no background image.
			- Option not included = fall back to config value.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--background-image-stretch[[=| ]bool]`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--background-image-zoom[[=| ]bool]`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- [ ] `--background-image-opacity[=| ]<n>`
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).

- [ ] Additional "File" menu option: "Save entire current layout to config".
	- Including window, tab, shell, and pane layout and configurations - everything.
	- Possibly to make this easier, store non-default per-tab and per-pane configurations as a "command line" in the config, that each override all other config settings.
	- Emits the create/select form: `--new-tab` / `--new-pane` (with explicit `--splits`, direction, and non-default `--size`) for structure, plus `--tab=<id>` / `--pane=<id>` for per-entity overrides. Always writes explicit directions and sizes (never the "more space" default) so a saved layout reproduces regardless of window size.

- [◐] Tab interface: single-window core done (`Tabs` in app.rs: each tab owns a `PaneManager`; tab bar shown with >1 tab, click to switch; pane area reduced by the bar). Detach/dock deferred (need multi-window). Verified: new tab, switch (content swaps), close (bar hides).
	- [✔️] New tab (CTRL+T by default)
	- [✔️] Close tab (CTRL+w, CTRL+F4)
		- Notes:
			- Ctrl+W also shadows shell word-erase; deferred, revisit if necessary.
			- Decided to not have ANY tab close hotkeys for now.
	- [✔️] Change tab (CTRL+page up, down)
	- [✔️] Move tab order (Shift+CTRL+Page up, down)
	- [ ] Detach tab to new window with mouse (deferred: needs multi-window)
	- [ ] Dock tab to different existing window with mouse (deferred: needs multi-window)

- [ ] When running `sudo apt update`, the progress bar at the bottom bounces about halfway below the render area, as lines above it scroll up. This seems to be a side-effect of smooth-scrolling. Is there a way to prevent that from happening, without fundamentally breaking the very concept of smooth scrolling?
	- Reopened: The first attempt (snap output easing during line bursts) broke smooth scrolling for all normal output and was reverted (see the smooth-scrolling-regression bug above).
		- Diagnosis still stands: apt reserves the bottom line as a status bar via a scroll region (DECSTBM `0..N-1`); printing each log line scrolls that region. Because the region starts at line 0, alacritty_terminal grows scrollback (`Grid::scroll_up` only calls `increase_scroll_limit` when `region.start == 0`), which fires our output easing - and the ease shifts the whole grid down by up to a cell, dragging the fixed status bar below the viewport = the bounce. A correct fix needs to know a partial scroll region is active so it can suppress easing only then, but `alacritty_terminal` doesn't expose `scroll_region` (private, no getter). Options for later: (a) patch/fork the crate to expose the region; (b) tee the PTY stream and parse DECSTBM ourselves; (c) accept as a known limitation like full-screen apps (`nano`).

- [✔️] Change license from MIT to "GNU General Public License v2.0 or later", SPDX "GPL-2.0-or-later", reference https://spdx.org/licenses/GPL-2.0-or-later.html.
	- Status: Done. `license.md` now holds the canonical, verbatim GPL-2.0 text from gnu.org, in a markdown fenced block. `Cargo.toml`, `license = "GPL-2.0-or-later"`. README badge -> GPL v2+ and the license blurb updated; every `.rs` file (src + examples, 18) carries an `// SPDX-License-Identifier: GPL-2.0-or-later` + copyright header. Builds + 19 tests pass. The only remaining "MIT" string is in the README's commented-out badge palette, left intact.
	- The reason it was MIT before, was due to the misunderstanding that derived works have to also be MIT. But that's not the case, MIT allows relicensing derived works.
	- GNU General Public License v2.0 or later offers more protections, while being compatible with the Linux kernel and Darwin.
		- Also, some included libraries are Apache, which is compatible with GPLv3 (and therefore GPLv2+), but not bare GPLv2.

- [✔️] Smooth-scroll enhancement: (20260626-100721)
	- Status: Done. `scroll_tau_ms` is now the initial (slow, smooth) speed; under output bursts the visual backlog accumulates (capped at 16 lines) and the ease dynamically ramps faster (down to 8ms tau) to keep up, then eases back to the slow speed once output stops. The speed change is itself smoothed (ramps up over ~90ms, back down over ~450ms) so it never jumps; the ramp only applies while following the bottom (wheel/scrollback keeps the plain ease). Settings control renamed "Initial scroll speed" (shown 1..100, higher=faster; stored as tau). Verified: 60/300/2000-line bursts all settle correctly at the bottom; wheel scrollback unaffected; no crash.
	- The fundamental challenge with smooth-scroll (and why it was abandoned it the late 80s), is that if the scroll is too smooth, then fast output will get backlogged in the buffer, and risk overflowing that buffer.
	- Solution:
		- By default, use a slower, smoother scroll. (E.g. for the case of the user typing one command at a time and sporadically scrolling lines up infrequently.)
		- But if the buffer starts filling up, dynamically ramp up the scroll in real-time to be faster; as fast as necessary to keep up.
		- Once fast-scrolling output stops, go back to the default slower, smoother scroll defined in config & settings.
			- Rename this setting for the user's benefit, "Initial scroll speed".
		- The change in scroll speed should itself be smooth, rather than immediate. But also dynamic, e.g. if needed to not get too far behind and a slow ramp-up to top speed isn't proving to be fast enough.
	- Example scenario:
		- Using `tail -f` to monitor the log output of a running background process. Such output can go one line at a time randomly occasionally; then suddenly have a long sustained burst of high-speed output. And everything in-between. Scrolling should dynamically adjust to be smooth at slower output, and fast at faster output.
	- [✔️] Set default "Initial scroll speed" to 25. - `scroll_tau_ms` default is now 230ms (= speed 25 on the 1..100 scale) in `Settings::default` + the config template. Verified: a fresh config + the dialog both show 25.

- [✔️] Config file: Separate different grouped setting comments and settings (which are good to keep together), by an empty newline. Keep individual settings and comments together though. (20260625)
	- The `DEFAULT_CONFIG` template is now grouped consistently (each setting with its own comment; `line_height_scale` no longer rides the font-size group. The three background-image keys split into their own comment groups. `backfill_config` is group-aware: `setting_groups` tags whether each template setting starts a new group (preceded by a blank/table), so a re-inserted key carries its comment block and different groups are blank-separated, while same-group keys (e.g. columns+rows, the scroll-feel keys) stay together. A boundary double-blank is de-duped. Note: only affects freshly-written or newly-backfilled keys - an existing file's already-present bare keys aren't reformatted (regenerate for the clean layout).

- [✔️] When double-clicking to select text, if the rule about quotes and brackets is in effect, and there are nothing but spaces in between selectable text and the matching quotes or brackets - then don't include the spaces in the selection. For example: " Now is the time. " - exclude the spaces between the symbols and the open and close quotes, in the selection. (20260625)
	- Status: Done. `pair_inside` now trims runs of spaces directly against the delimiters (interior spaces kept): `" Now is the time. "` selects `Now is the time.`, `[  hi  ]` selects `hi`. All-spaces inside falls back to the full inside span. Unit-tested (`pair_trims_adjacent_spaces`).

- [✔️] Optimize compiled binaries to balance executable size and speed (slight nod to size), without the risk of triggering antivirus.
	- Status: Done. `[profile.release]`: `lto = "fat"` (whole-program inlining - smaller and usually faster than thin), `panic = "abort"` (drops unwinding tables - sizable shrink, fine for a GUI app), kept `codegen-units = 1` + `strip = true`, and opt-level stays 3 so renderer/PTY hot paths aren't slowed (the size improvement comes from the free wins, not from `opt-level=s/z`). Deliberately no UPX/packer - packers routinely trip AV heuristics. - Result: Linux binary is ~13% smaller, no runtime-speed tradeoff; verified still runs.

- [✔️] Local CI/CD pipeline, one command, fail-fast, reusable across projects (`cicd/`). (20260628)
	- Expand the scope of existing `cicd.bash` copied from a sister project.
	- Solution:
		- One command (`cicd/cicd.bash`) runs the whole release end to end: format the code, debug build, run the tests, take a profiler snapshot, build all the release targets (native + cross), install the native build into a local bin dir ("dogfood"), then back up and publish to git. It prints the plan and the paths it will use first, and stops at the first problem.
		- Reusable in other projects: copy the `cicd/` directory and edit just `cicd/config.bash`. The engine itself stays generic.
		- Can run fully unattended with `-y` (give the publish commit message up front with `-m "..."`), so it formats, builds, tests, releases, and publishes without stopping to ask. Any stage can be skipped (`--no-fmt`, `--no-cross`, `--no-profile`, `--no-dogfood`, `--no-publish`).
		- The profiler stage is informational, not a pass/fail gate: it runs the real app under heavy load for a few seconds and saves a flamegraph - a single SVG you open in a browser to see where the time goes. It only aborts the run if the app itself misbehaves, not for environmental reasons like no display.
		- Old profiler snapshots and git backups are both trimmed to about 30 files by one shared routine, keeping a time-spread history: the most recent handful, plus the newest of each recent hour/day/week/month/year, plus the very first.
		- The fuller details (profiler tooling, the dedicated build profile, the rotation rules and tuning knobs) are documented in the `cicd/` scripts themselves.

- [✔️] Background image:
	- [✔️] By default unless overridden, look in ~/.config/silkterm/backgrounds/background.* - Status: Done. `resolve_bg_image` now auto-detects `backgrounds/background.{png|jpg|jpeg}` under the config dir (explicit `background_image` paths unchanged). Verified: image in that dir auto-loads.
	- [✔️] Change default from "zoom" to "stretch". - `Settings::default` + the load fallback + the config template default are now `stretch`. Verified: auto-detected image fills the window (aspect ignored).
	- [✔️] Add to background settings: Gaussian blur radius. - `background_blur` config (sigma in px, 0 = none; default 0) applied at image load (`image::imageops::blur` in `load_bg_image`), plus a "Bg image blur" slider in Settings (`bg_image_changed` re-loads on change). Verified. Note: blur is applied in source-image space (the shader still does the stretch/zoom fit), so not literally "after fit" - fine for a decorative low-opacity background; a true post-fit blur would need a 2-pass GPU blur (follow-up if wanted).
		- [✔️] Results in pronounced color banding. Look into higher-quality blur filter, higher bit-depth for intermediate calculation, and/or dithering.
			- Cause. Mostly bit depth: the GL offscreen was 8-bit linear (`Rgba8Unorm`).
			- Fixes:
				1. Offscreen is now `Rgba16Float`, high-precision linear intermediate; the blit still does the single linear->sRGB encode into the 8-bit fbo 0.
				2. The blit adds TPDF dither (~1 LSB, per-pixel hash) before the 8-bit write, breaking residual banding scene-wide.
				3. The blur now runs in linear light (decode sRGB -> blur in f32 -> re-encode) so edges are gamma-correct.
			- Verified on a dark gradient. Visibly smooth. `dump_offscreen` updated to decode f16.

- [✔️] Text readability glow:
	- [✔️] When enabled, this setting adds some blurry background color, behind each glyph. In Photoshop, it's called "Outer Glow". - Done via `src/glow.rs` (`Glow`): the scene's text is rendered to a texture, blurred with a 2-pass separable Gaussian (`text_glow_radius` sigma), then composited (tinted the bg colour, `srgb_f32(bg)`) under the crisp text. Ping-pong f16 textures; intensity boost (`GLOW_INTENSITY=6`) so the blurred coverage is solid near glyphs. Gated `config.text_glow` (default off -> render path unchanged). Verified: light text on a light background is unreadable without it, clearly readable with it (dark halo). Implements exactly the suggested approach (render-bg-colour -> blur -> crisp on top), using the glyph alpha as the glow mask so no separate glow-coloured buffers are needed.
	- One possible way to do this - and there may be other, better ways:
		- Render the text exactly as normal, except in the background color. (As if background were 100% opaque.) On a fully transparent temporary canvas (at least conceptually - not necessarily literally).
		- Blur that rendered text with a gaussian blur, according to the specified blur radius in settings.
			- We may need to scale the radius value the user sees and adjusts, x*10, for cleaner integer values, then n/10 to use in code.
		- On top on that blurry background-color text, render the actual text in normal crisp text color.
	- The end result will be:
		- Even if the background is 0% opaque and effectively invisible, and the screen background is very light (like the terminal text color), the text will still be readable because it will have a dark (or background-colored) "glow" around it.
		- Even if the background is 100% opaque but the background image is very light (like the terminal text color), the text will still be readable - for the same reason.
	- [✔️] Expose config value in settings dialog:
		- [✔️] Blur radius: Boolean to enable, slider + number field to adjust.
			- "Text glow" toggle + "Glow radius" slider in Settings -> Appearance; the radius is greyed out/inert when the toggle is off (same `disabled()` mechanism as the Opacity slider). Verified in the dialog. (Editable numeric field is part of the deferred dialog-part-2 work.)
		- [✔️] Softness/intensity control. Maybe "Softness" as the name. - New `text_glow_softness` (0..1, default 0.4) + a "Softness" slider in Settings (greyed when Text glow is off). Maps to the glow's coverage boost: 0 = hard/solid/strong halo (x10), 1 = soft/faint (x1). Verified: softness 0.1 = bold dark halo, 0.9 = gentle faint glow. (If the high=softer direction reads backwards, it's a one-line flip.)
	- [✔️] Visual bug: When background glow is applied to characters that have a per-character(s)-box different background, and the foreground color is similar to the global background for that character(s), then the character is a blurry mess. (E.g. the global background is dark, but some characters are rendered one-off with dark text and light background, then it's not readable.)
		- [✔️] The solution is, if a character has a different background color than global, use that one-off background color as the glow color for that character. - Done: the glow is now coloured by a per-pixel "bgcolor" texture (cleared to the global bg, with the per-cell bg rects drawn over it) instead of a single global tint; the composite multiplies the blurred glyph coverage by that local colour. So a glyph on a colored cell gets a halo matching its own cell bg (harmless), while global-bg cells keep their readability halo. Verified: dark text on a light cell over a dark global bg renders clean (no dark blur), global-bg text keeps its glow.

- [✔️] Config file: When reading a value from the config file, if the entry doesn't exist, insert the setting into the file using hard-coded defaults, in an approprite section. (While not overwriting other existing values, comments, space formatting, etc.) Make this a reusable feature.
	- Status: Done. `config::backfill_config` (run in `load` after the file exists) inserts any setting the `DEFAULT_CONFIG` template defines that the user's file lacks, using the template's own line - so follow-system keys (font_size, font_family, background_*) stay commented (behavior unchanged) and active keys get their default value. Top-level keys go before the first table; `[colors]` keys under that header. Existing values/comments/formatting are preserved (insert-only). Reusable helpers: `setting_lines`/`line_table`/`line_setting_key`.
	- Verified: a partial config gets the missing keys (commented vs active per template), custom `opacity`/`foreground` preserved, re-run idempotent.

- [✔️] When double-clicking to select stuff backwards and forwards to defined delimiters: Ignore delimiters if inside a consistent pair of single or double quotes, or paired (), [], <>, or {}. In those cases, select everything inside those (but not including).
	- Implied: `Pane::pair_span` + pure `pair_inside`/`distinct_pair`/`same_char_pair` (pane.rs, unit-tested). On a double-click the app first checks `pair_inside`; if the click is inside a matched pair it selects the contents (a `Simple` range), else falls back to the normal `Semantic` word select. Single-line only (multi-line pairs not handled).
	- [✔️] But if the double-click happened outside such consisten parings, then ignore that logic (and the selection might include such characters depending on defined delimiters).
		- Falls back to `Semantic`.
	- [✔️] The order of pair inclusion precedence: ``, "", '', {}, (), [], <>. - first enclosing pair in that order wins (so inside `()` selects the `()` contents even when `[]` is nested within). Verified by the precedence/quote-beats-paren tests.
	- [✔️] List of delimiters should also be read from config file.
		- Status: Done. `word_separators` (config) feeds alacritty's `semantic_escape_chars`; backfilled if missing.
	- [✔️] The list of selection inclusion pairs should be read from the config file.
		- Status: Done. new `selection_pairs` config key (default `` `` "" '' {} () [] <> ``), parsed by `config::selection_pairs()`; backfilled (commented) if missing. Not in the Settings dialog.

- [✔️] Build targets, listed in order of importance: (20260626-091500)
	- [✔️] Linux x86_64 (aka AMD64, but name everything referred to as "x86_64" for consumers/readers sake because "AMD64" is visually confusable with "ARM64").
		- Done. Native: `cargo build --release`. (Naming already consistent: no "AMD64" anywhere in code/docs/build config.)
	- [✔️] Linux ARM64: `cargo zigbuild --release --target aarch64-unknown-linux-gnu` (cargo-zigbuild + zig 0.13). Built clean; binary is ELF aarch64.
	- [✔️] Windows x86_64: `cargo build --release --target x86_64-pc-windows-gnu` (mingw). PE32+ x86-64.
	- [✔️] Windows ARM64: `cargo zigbuild --release --target aarch64-pc-windows-gnullvm`. Built clean; PE32+ ARM64.
	- [🚫] macOS ARM64: Deferred. cross-compiling Linux->macOS needs Apple's SDK (osxcross), which is license-gated; do it on a Mac / in CI.
	- [🚫] macOS x86_64: Deferred. (Same; Mac/CI.)
	- Toolchain setup + commands are in `build.md`; one-time: install zig + `cargo install cargo-zigbuild` + `rustup target add aarch64-unknown-linux-gnu aarch64-pc-windows-gnullvm`. No ARM64 system libs needed (X11/EGL dlopen'd at runtime).

- [✔️] True transparency:
	- Bug (fixed): Adjusting the transparency affects only the overall terminal background (including image which already has it's own correctly functioning opacity).
	- Transparency should not affect the Window decorations, menu, focus, or - critically - terminal text.
	- Status: Done. Opt-in `transparent_background = true`; `opacity` is the background alpha; text, decorations, and the menu/tab bars stay opaque. Verified on X11/Compiz/NVIDIA, decorated and borderless. Default (`false`) path unchanged (native wgpu).
	- How: wgpu can't get per-pixel alpha on X11 by itself (its Vulkan swapchain forces an opaque surface; its GL backend won't bind the 32-bit ARGB visual). So on X11 we create the window + a transparent GL context with glutin and run wgpu on top of it via hal interop (`Gfx::new_gl_transparent`), render the scene to an offscreen texture, then blit that into the GL framebuffer. Off X11 (e.g. Wayland) the plain wgpu surface already does premultiplied alpha. `Gfx` is a two-backend enum (native wgpu / GL). No wgpu downgrade, no renderer rewrite.
	- The hard part (cost ~a day; a web search cracked it - gfx-rs/wgpu #8675 + #8676): on NVIDIA/Linux glyphon renders no text on a GL context below 4.2, because wgpu silently no-ops drawing into a texture view there (that's how glyphon builds its atlas). Fix: request GL 4.6, falling back down to 3.3. Diagnostics: `SILK_DUMP=1` dumps the offscreen to `/tmp/silk_offscreen.png`; `diagnostics/glyphon_gl.rs` is a headless probe.

- [✔️] Make both the main menu, and the right-click menu appearances more traditional:
	- [✔️] Use the system proportional font, rather than monospace font. - New `text::sans_attrs()` (cosmic-text `Family::SansSerif` -> the system default proportional font); the menu bar titles, dropdowns, and the right-click menu all use it.
	- [🚫] Use the system menu background and text color if reasonably feasible in a cross-platform way.
		- Canceled. There's no clean cross-platform API (Windows has `GetSysColor(COLOR_MENU/COLOR_MENUTEXT)`, but Linux/GTK needs CSS-theme parsing and macOS needs `NSColor`/objc). Kept the existing tasteful dark menu palette.
	- [✔️] No indented items.
		- All labels start at a common x after a fixed checkmark gutter (`MENU_GUTTER`); a `✓` is drawn in the gutter for active toggles, so checkable and plain items align.
	- [✔️] Group items logically, and use faint horizontal lines and extra space to separate the logical groupings, as has been standard for menus since early Macintosh and Windows.
		- Menu entries are now `Entry::Item`/`Entry::Sep`; separators render as a faint 1px line (`MENU_SEP`) with row spacing (`MENU_SEP_H`). Right-click groups: clipboard | read-only | tab/split/close | window toggles | config/settings. Verified.

- [✔️] Format the "Help|About" widget better.
	- [✔️] Use system proportional font.
		- Done. `text::sans_attrs()`, one buffer per line.
	- [✔️] Add space between sections.
		- Done. `open_about` lays lines out with a section gap (`MENU_SEP_H`) before Info, the link, and the hint.
	- [✔️] Put system info under an "Info" heading.
		- Done. "Info" heading with Renderer / Backend / Acceleration indented under it.
	- [✔️] In addition to GPU info, note if using GPU acelleration or not.
		- Done. "Acceleration:" line from `adapter_info.device_type`: Hardware (discrete/integrated/virtual GPU) vs Software (CPU).
	- [✔️] Add clickable github URL.
		- Done. Repo URL (from `CARGO_PKG_REPOSITORY`) drawn in the link color + underline; click within its rect runs `open_url` (xdg-open / open / start). Hit-rect verified; browser-launch not runtime-tested (would pop a browser).
	- [✔️] Separate modal window rather than an embedded widget.
		- Done. About is now a real pop-out OS window sized to its content (`src/dialog.rs` `DialogWin::new_about`), via the new multi-window foundation (`App.dialog`, event-dispatch by `WindowId`, rendered in `about_to_wait`. Window creation signaled from `State` since it needs the event loop). Esc / window-close dismisses it. The repo link is clickable. The old in-surface overlay path is superseded; its dead code has now been removed (branch `rmoverlay`).
	- [🚫] Use the system window background and text color if reasonably feasible in a cross-platform way.
		- Canceled. Same as the menus: no clean cross-platform API. Kept the dark palette.

- [✔️] Settings dialog:
	- [✔️] Use the system proportional font.
		- Done. Dialog text now uses `text::sans_attrs()`, centered against the real line height (also fixed the misalignment bug above).
	- [✔️] Allow selection of terminal background image (or none).
		- Done. A "Background image" text field (Kind::Text): type/paste a path; empty shows "(none)" and clears it. Live-edited (`reparse_edit` -> `background_image`), persisted by `config::persist` (sets the key, or removes it for none). Apply reloads the image. No native file picker in this stack, so a path field.
	- [✔️] Allow setting font and size to "System default".
		- Done. A single "Use system font" checkbox (Kind::Toggle): when on it clears `font_family` and adopts the detected size live, and Apply removes `font_family`/`font_size` from config (`config::remove_keys`) so launches follow the OS; dragging the Font size slider turns it back off (explicit).
	- [✔️] Make settings dialog a separate modal window rather than an embedded widget.
		- Done. Settings is now a pop-out OS window (`DialogWin::new_settings`, `Content::Settings(SettingsDialog)`), content-sized (~540x800) and non-resizable, so the whole dialog is visible regardless of the main window size (the requirement). Full interaction in-window: sliders (drag/click), text/hex fields (type), color swatches, Cancel/Apply/OK + Esc. Apply/OK live-apply to the main window via `App::apply_dialog_settings` -> `State::apply_settings_values` (config persist + rebuild). Verified: slider->Apply persisted `opacity` to config; OK closes; main survives. (The old in-surface overlay paths have now been removed in a dedicated cleanup, branch `rmoverlay`: `open_about_overlay`/`open_settings_overlay`/`apply_settings`/`handle_dlg_action`, the `AboutBox`/`AboutLine` structs, the `about`/`settings_dlg` fields, and all their render/event branches; ~278 lines. The live pop-out path and menu overlay are untouched.)
	- [🚫] Use the system window background and text color, if feasible in a cross-platform way.
		- Canceled. No portable API; same as the menus/About.

- [✔️] Allow common menu accelerators (e.g. Alt+F for File menu).
	- Done: Alt+F/E/V/T/P/H open the matching top-level menu (first-letter match against `MENU_BAR`), when the menu bar is shown. note: this deliberately shadows the shell's Meta+<those letters> (e.g. Meta-f word-forward) - the standard menu-bar tradeoff (GNOME Terminal does the same).
	- Verified.

- [✔️] Tab titles:
	- If a non-shell program is currently running, display: "shell [program]", where 'program' is the name of the running program.
	- If only the shell is running, display: shell [last: program]
		- [ ] bug: If I run for example `ls`, The title isn't updated to "shell [last: ls]".
			- It seems to hinge on how long the command takes to execute. If the code is doing some kind of frequent sampling to get the program name, and if that could impact performance, then let's get rid of the " [last: <program>]" requirement and just show "shell". Otherwise if there is a more reliable alternate method to always know the last program that was run, that doesn't hurt performance (e.g. by requiring a watcher loop), let's try that.
	- Just the executable name for both, not the full command-line
	- Implemented:
		- `TermInstance` captures the PTY master fd + shell pid at spawn (before the event loop takes the pty). `tab_title()` reads the foreground process group via `libc::tcgetpgrp(master_fd)` and its `/proc/<pid>/comm` (executable basename), comparing to the cached shell name: a different program -> "`<shell> [<program>]`" (and remembers it); only the shell -> "`<shell> [last: <program>]`" or just "`<shell>`". Polled when the tab bar is built (renders happen on output). Unix only (`#[cfg(unix)]`); other platforms fall back to the app name. New direct dep `libc` (unix).
		- Verified: `dash` -> `dash [sleep]` -> `dash [last: sleep]`. Tab titles also use the proportional font now.

- [✔️] Window title: Just "SilkTerm", plus the icon in assets/logo.png (for display in alt+tab).
	- `update_title` now sets the window title to just `APP_NAME` (per-program info stays in the tab titles). The window icon is loaded from `assets/logo.png` (`include_bytes!`, decoded + downscaled to 64x64 via the `image` crate) in `load_icon` and set with `with_window_icon`. Verified: window name = "SilkTerm", `_NET_WM_ICON` is set.
	- [ ] Updated requirement: Window title: Either use top-level `--title=`, or fallback to default, which is "SilkTerm - XYZ"; where 'XYZ' is the title of the current tab.

- [✔️] No hotkeys for pane management except. Minimal hotkeys overall, except for window, tab, menu, and clipboard managent.
	- Done. Removed the pane hotkeys: Ctrl+Shift+R/D (split V/H), Ctrl+Shift+W (close pane), Ctrl+Shift+Tab (cycle focus). Pane management is menu-only now (Panes menu / right-click; focus by clicking). `cycle_focus` deleted. Remaining hotkeys are window (F11), tab (Ctrl+T new, Ctrl+PageUp/Down change, +Shift move), menu (Alt accelerators, Menu key, Ctrl+,), clipboard (Ctrl+Shift+C/V).

- [✔️] Changed mind about "close tab" hotkey: none. Use right-click or main menu, or just exit command.
	- Done. Removed the Ctrl+F4 close-tab hotkey. Close a tab via the Tabs menu ("Close Tab") or by exiting the shell.

- [✔️] Menu keyboard key should activate right-click menu on active pane.
	- Done. The Menu/Apps key (`NamedKey::ContextMenu`) opens the context menu anchored near the focused pane's top-left. Verified.

- [✔️] Group Settings items into logical sections.
	- Done. Added section headers (`Kind::Header`, bold + a faint rule): Appearance / Font / Window / Scrolling / Colors. `row_y`/height now sum per-row heights (headers are taller). Verified at runtime.

- [✔️] Need a way to specify the font in the Settings dialog.
	- Done. "Font family" text field (empty = "(system default)"). Applies live: `MONO_FAMILY` changed from a write-once `OnceLock` to a re-settable `RwLock<Option<&'static str>>` (`pin_mono_family` re-resolves on each `TextCtx` rebuild and leaks the name for the `'static` `Attrs`), so the dialog's font family / "Use system font" take effect on Apply, not just restart. Persisted by `config::persist`. Also fixed: the spacebar is `Named(Space)` (not a Character), so font names / paths with spaces now type correctly into dialog fields. Verified: set "DejaVu Sans Mono" -> persisted with spaces, applied live, text renders.

- [✔️] Add window dimensions to Settings dialog.
	- Done: Columns (20-400) and Rows (6-120) sliders in the new "Window" section. On Apply, if they changed, the window is resized to the new cell grid (`request_inner_size` from `cols*cell_w` / `rows*cell_h` + margins + menu bar). Persisted.
	- Verified: Columns slider -> window 1703->818px, `columns = 76` written.

- [✔️] Make "Settings" title on dialog more prominent. (Bigger bolder font. Same with "About" dialog - but give it a title first.)
	- Done. `TextItem`/`AboutLine` gained `bold` + `scale`; the app applies `Weight::BOLD` and `TextArea.scale`. The "Settings" title is bold + 1.4x; the About box now leads with a bold + 1.5x "About SilkTerm" title (it had no real title before).
	- Verified.

- [✔️] Menu content change: No tab or pane setting under the "File" menu. "Panes" can be it's own top-level menu item, between "Tabs" and "Help".
	- Done. Menu bar is now File / Edit / View / Tabs / Panes / Help. File = Reload Config, Settings..., Quit (no tab/pane items). Tabs = New/Next/Previous/Close Tab. Panes (new, between Tabs and Help) = Split Vertical, Split Horizontal, Close Pane (moved out of View). View = Fullscreen, Hide window frame, Menu bar. Verified at runtime: bar shows the six menus, File has only app-level items, Panes holds the split/close actions.

- [✔️] Right-click menu options (with logical grouping):
	- [✔️] Copy; selection -> CLIPBOARD
	- [✔️] Paste; CLIPBOARD -> pane (bracketed-aware)
	- [✔️] Paste selection; PRIMARY -> pane
	- [✔️] Read-only (accept no input or interruption, but mouse selection and copy still work; toggle with checkmark)
	- [✔️] New tab
		- Done. Right-click "New Tab" (`MenuAction::NewTab` -> `App::new_tab`); same as Ctrl+T.
	- [✔️] Split vertical (already exists)
	- [✔️] Split horizontal (already exists)
	- [✔️] Hide menu (toggle with checkmark)
		- Done. View -> "Menu bar" (✓) and the right-click menu both toggle `menu_bar` (`MenuAction::ToggleMenuBar`); hidden = content to the top edge, re-show from the right-click menu.
	- [✔️] Hide window frame (toggle with checkmark)
		- Done. `window.set_decorations`; verified frame extents 39px->0. Also the route to content-only transparency (bug 1).
	- [🚫] Hide scrollbar (toggle with checkmark)
		- Canceled. No scrollbar exists for smooth-scroll.
	- [✔️] Fullscreen (toggle with checkmark)
		- Done. `window.set_fullscreen(Borderless)` + F11. Code path verified called; Compiz on this box doesn't honor the request (env, like the F11 grab), works on a compliant WM.
	- [✔️] Settings
		- Done. Right-click "Settings..." opens the Settings dialog (`MenuAction::Settings`). Also Ctrl+,. Plus "Reload Config" to apply hand-edits.

- [✔️] Some way to auto-apply settings after editing config file, without watching it. Maybe an internal command.
	- Done. Right-click menu -> "Reload Config" re-reads `config.toml` from disk and live-applies it (`config::reload_from_disk` + the shared `App::apply_new_settings`, the same rebuild path the Settings dialog uses: re-creates `TextCtx` + relayout on metric changes, reloads the bg image, re-sets window opacity). No file watcher; the file is the source so nothing is persisted back.
	- Verified.

- [✔️] Change default columns = 160. Default margin = 8.
	- Done: `Settings::default()` and the `DEFAULT_CONFIG` template now use `columns = 160`, `margin = 8.0`. Existing config files keep their own values (defaults only seed fresh configs). Verified: fresh config -> window 1703x1024 (160 cols), content inset 8px.

- [✔️] A window menu with typical menus items and actions (File, Edit, View, Tabs, Help)
	- Done. Menu bar across the top (`MENU_BAR_H`, shown by default; `area()` insets the pane area below it, stacked above the tab bar). Click a top-level title to open its dropdown; hovering another title with one open switches to it; click the title again or click away / Esc to dismiss. Dropdowns reuse the existing `ContextMenu` widget (`bar_menu_items(idx)` builds each; `open_bar_menu`). Contents: File (New/Close Tab, Close Pane, Reload Config, Settings..., Quit), Edit (Copy/Paste/Paste Selection, Read-only ✓), View (Split V/H, Fullscreen ✓, Hide window frame ✓, Menu bar ✓), Tabs (New/Next/Previous/Close Tab), Help (About...). Help->About opens the About dialog (originally a centered overlay, since reworked into a pop-out window - see the Help/About item). New `MenuAction`s: CloseTab, NextTab, PrevTab, ToggleMenuBar, About, Quit. Initial window height adds `MENU_BAR_H` so the default row count still fits.
	- Verified: bar renders, dropdowns open/switch, About shows "NVIDIA ... - Vulkan", Menu bar toggle hides the strip (content to the top edge).

- [✔️] Render area shouldn't have a blue line (or any line) around it. When Window decorations are turned off, it should be background all the way to the last pixel of the edge.
	- The "blue line" was the focus ring drawn around the focused pane, which with a single pane traces the whole content edge. `App::render` now draws the ring only when the current tab has >1 pane (it exists to tell panes apart), so a single pane reaches the window edge with just background. Verified: single pane has no ring (only the cursor is bluish); after a split the ring returns around the focused pane.

- [✔️] Add adjustable background image opacity to config file, and make default about 33%. This is independent of "see-through" opacity. The "opacity" should be relative to the background color. 0% = all background color, 100% = all image.
	- Done. `background_opacity` already provided this (0 = all bg color, 1 = all image); changed the default to 0.33. Independent of `opacity` (see-through).

- [✔️] CTRL+shift+C and CTRL+shift+V should work as clipboard commands.
	- Done. Ctrl+Shift+C copies the focused pane selection to the CLIPBOARD; Ctrl+Shift+V pastes it (`handle_hotkey`). Verified at runtime.

- [✔️] Double-clicking selects a word up to user-tweakable delimiters (sane defaults; full paths stay whole).
	- Done. Double-click (<400ms, same cell) starts an alacritty `SelectionType::Semantic`. New `word_separators` config sets the delimiters; default = alacritty's (keeps /.-_~ as word chars). Verified: double-click selected a whole path.

- [✔️] Settings GUI dialog with organized main tunables, with primary buttons: Cancel, Apply, OK. Default=OK.
	- `src/settings_ui.rs`: modal overlay (second pass, like the context menu) opened via Ctrl+, or right-click "Settings...". Sliders for opacity / bg-image opacity / font size / line height / margin / scroll-tau / wheel-lines, and swatch + hex field for the 4 colors. Cancel / Apply / OK (Enter=OK, Esc=Cancel). Live-apply: opacity re-sets window opacity, colors re-render, font/metrics rebuild the TextCtx + relayout; persisted in place via toml_edit (only changed keys, comments preserved, floats rounded). Foundation: `config::settings()` is now a swappable `Arc<Settings>` (`config::update`/`config::persist`). Verified: slider drag + Apply changed live opacity and persisted; hex typing recolored the swatch live; font-size change rebuilt text live without crashing. Not yet exposed (field table is trivially extensible): font_family, scrollback, alt/output scroll lines, background_fit, columns/rows, word_separators.

- [✔️] If hardware acceleration is not available, use software rendering. Also need a way to tell which the application is using. Maybe in "help/about".
	- `Gfx::new` requests a GPU adapter, then retries with `force_fallback_adapter` (a CPU/software adapter) if that fails. The renderer (name / backend / device-type) is logged at startup, and the Help/About dialog shows it (Renderer / Backend / Acceleration) from `adapter_info`. Verified: logs "NVIDIA GeForce RTX 3060 Ti [Vulkan / DiscreteGpu]".

- [✔️] Make it easy to change the program name, in project and code files
	- Display name centralized in `APP_NAME` (`src/config.rs`); `utility/rename.sh NewName` rewrites the name + lowercase id across Cargo.toml, sources, and docs in one shot. Not a runtime/user setting.

- [✔️] Local config file with tunables, somewhere under ~/.config
	- `$XDG_CONFIG_HOME/silkterm/config.toml` (-> `~/.config/...`), auto-created with commented defaults on first run. Tunables: font, size, line height, margin, scrollback, scroll feel, colors (`#rrggbb`). Malformed/unknown entries fall back to defaults.

- [✔️] Use system monospace font by default
	- Default font is the OS-configured monospace family (e.g. Monaspace Argon from GNOME settings) when it's installed, else cosmic-text's generic `Family::Monospace`. `font_family` in the config overrides it by name.

- [✔️] Slightly More (and user-adjustable) margin between output and window border.
	- `margin` config option (logical px, default 4), DPI-scaled, inset on all sides of each pane's content.

- [✔️] Default to all black background, and 152 columns by 48 rows
	- Solution: Default `background` is now `#000000`. New `columns`/`rows` config options (default 152x48) size the initial window: after cell metrics are known the window requests `cols*cell_w + 2*margin` x `rows*cell_h + 2*margin` px, so `content_dims` floors to exactly the requested grid. Existing config files keep their own colors (defaults only apply to freshly generated configs).

- [✔️] Some unicode glyphs don't render, most likely due to inadequate font coverage rather than a bug. Need fallback fonts just for glyphs that don't render, similar to how other terminals and text editors work. Don't need to expose fallback fonts as tunables (other terminals and text editors don't).
	- Solution: Switched pane text shaping from `Shaping::Basic` to `Shaping::Advanced`, which does per-glyph font fallback (CJK, emoji, math symbols, RTL) instead of rendering tofu, while keeping monospace alignment via cosmic-text's monospace-fallback path. Uses installed system fonts; no tunable. This was basic because an earlier cosmic-text 0.18 hung on real output here; 0.18.2's fallback loop is bounded and was stress-tested. Glyphs with no font on the system (e.g. powerline/nerd PUA with no nerd font installed) still fall back to whatever claims them - the user installs the relevant font, as with any terminal.

- [✔️] Ability to select text by partial lines, with left mouse button.
	- Solution: Left-press maps the pixel to a grid `Point`+`Side` (`Pane::point_at`) and starts an alacritty `Selection::Simple`; drag extends it; release copies `selection_to_string()` to the X11 PRIMARY selection. Selected cells are highlighted (`config::SELECTION_BG`) via `SelectionRange::contains`. A click with no drag clears the selection.
	- Verified.

- [✔️] Ability to select text with in a grid-aligned rectangle, with CTRL+left mouse button.
	- Solution:  Same path with `SelectionType::Block` when Ctrl is held at press. Verified: Ctrl+drag yields a rectangular block (cols 2-4 across 3 rows), not a span.

- [✔️] Copy & paste selected text to current cursor location, via middle mouse button.
	- Solution: Copy-on-select writes to the primary selection (copypasta `X11ClipboardContext<Primary>`, held for the app's lifetime so ownership persists). Middle-click reads primary and writes it to the pane under the cursor, wrapped in bracketed-paste when the app enabled it. Verified: primary -> middle-click -> bytes reached the shell. `src/clipboard.rs`.

- [✔️] Use mouse to resize panes by grabbing on to separater line.
	- Solution: Each `Split` already carried a `ratio`; `divider_at` hit-tests the gap strip (+/-`DIVIDER_GRAB_PX`) and returns the split-tree path, `drag_divider` walks that path and sets the ratio from the cursor (clamped 0.05-0.95) then relayouts. Left-press on a divider starts a resize instead of a selection; hovering one shows a `ColResize`/`RowResize` cursor.
	- Verified.

- [✔️] Ability to re-order panes with drag-n-drop mouse (possibly "grabbing" via shift-primary mouse button - and drop targets highlight themselves under mouse).
	- Solution: Shift+left-press grabs the pane under the cursor (Grabbing cursor); the pane currently under the cursor is tinted as the drop target (`config::DROP_TARGET`, alpha 0.3); releasing swaps the two leaves in the split-tree (`swap_panes` -> `swap_leaves` + relayout).
	- Verified: Shift+dragged left pane onto right, the two panes (distinct shell PIDs) swapped positions.

- [✔️] Ability to make terminal area transparent (from 0-100% opaque). Ignore if compositing is not supported.
	- Solution: Tunable `opacity` (0..1, default 0.95) sets the terminal-background alpha (opt-in `transparent_background`). On X11 the per-pixel route (glutin + wgpu-hal GL interop) makes only the background translucent - text and chrome stay crisp and opaque. On Wayland the native wgpu surface already exposes premultiplied alpha. Without a compositor it's a no-op. Full detail in the "True transparency" item above.

- [✔️] Ability to set an image as background, with adjustable visibility from 0-100%. That also works with transparency.
	- Solution: `src/bgimage.rs` ImageRenderer: a full-window textured quad drawn over the pane fill, under cells/text (premultiplied, composes with window opacity). `background_image` (absolute or filename relative to the config dir), or auto-detect background.{png,jpg,jpeg} in the config dir. `background_opacity` (visibility) and `background_fit`. `image` crate decodes png/jpeg.
	- Verified: auto-detected background.jpg renders; opacity 0.3 dims it.
	- [✔️] Render options: Stretch-to-fit, Zoom-to-fit.
		- Done. `background_fit` = "stretch" | "zoom"; default zoom/cover.

### Deferred

### Won't do

- [🚫] Terse `--layout` DSL as optional sugar over the window/tab/pane CLI model (not a replacement). One compact string for quick splits; lowers to the exact same internal layout the hierarchical flags produce, so it inherits per-pane targeting "for free."
	- Operators (mnemonic = the divider they draw): `|` side-by-side (vertical divider), `-` stacked (horizontal divider); `(...)` to nest (a group is uniform - mix directions by nesting); `;` separates tabs; `.` = one default pane.
	- Leaf = `.` (default shell) | command-alias name (from a `[commands]` config table, keeps the string quote-free) | `{raw command}` (opaque span so an inner `|` pipe isn't parsed as a split; `\}` escapes a brace). Optional fixed-order suffixes: `@dir` (cwd), `:weight` (size), `!` (keep-open).
	- Example: `silkterm --layout '(.|.)-. ; nvim|{git log} ; btop'` -> tab1: two-on-top/one-below; tab2: nvim beside a git-log pane; tab3: btop. Same string is accepted in `layout = "..."` in the config.
	- Trade-off vs the flags: far terser for hand-typed/quick layouts, but less self-documenting; the flags stay the canonical form (and what "Save layout" emits). DSL is purely a convenience front-end.

- [🚫] In `nano`, scrolling isn't smooth, it jumps line-by-line like traditional terminals. Is that just an artifact of the way `nano` specifically works?
	- Observation: `nano` (like `vim`, `less`, etc.) runs in the alternate screen and repaints the visible region in place; it keeps fixed chrome (title bar, shortcut bar) and rewrites the text rows itself. There is no terminal-level scroll (`display_offset` stays 0, no scrollback growth) for the renderer to ease, so the content snaps. The wheel now at least drives nano's own (line-by-line) scrolling via alternate-scroll. Making full-screen apps scroll smoothly would require the terminal to detect a vertical content shift within the app's scroll region frame-to-frame and animate it - a heuristic, app-fragile feature (nano's fixed bars break a naive whole-grid diff). Left as a future enhancement rather than a fragile hack.

## Application name ideas

- SilkTerm ["silk" is saturated but otherwise as a whole pretty unique, no world-language problems]
- FlowTerm [already an existing terminal]
- Velumi [many existing brands and .com]
- FluxTerm ["flux" is very crowded]
- GissaTerm [first actual project name but doesn't flow off the tongue well]
- Glissando [sounds like music software]
- Glidra [sounds like something on a drug store shelf]
- Velumux
- Velora, Seluvo, Movia, Eluvo, Sorilo, Volira, Lunavo, Novelo, Orivo, Silora, Avelo, Rovio, Meluvo, Zelio, Scrollo, Veloterm, Paneva, Tabelo, Fluxio, Termio, Velio, Siloterm, VelumiX, VelumiTab, VelumiPane, Velumux, Termumi, Termilo, Termora, Lumiterm, Termelo, Gliderm, Scrollio, Scrolumi, Veloflow, Glidia, Movira, Avelio, Levumi, Rivio, Aroyo, Fluvio, Lumora, Cursora, Paneo, Tabio

Decided: "SilkTerm". (Started as "GlissaTerm".)
