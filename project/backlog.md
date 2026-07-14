<!-- markdownlint-disable MD007 -- Unordered list indentation -->
<!-- markdownlint-disable MD010 -- No hard tabs -->
<!-- markdownlint-disable MD033 -- No inline html -->
<!-- markdownlint-disable MD055 -- Table pipe style [Expected: leading_and_trailing; Actual: leading_only; Missing trailing pipe] -->
<!-- markdownlint-disable MD041 -- First line in a file should be a top-level heading -->

<!-- TOC ignore:true -->
# Smooth-scrolling terminal - Backlog

This is a product backlog just for pre-v1.0.0 release. After that, bugs, features, and enhancements will be mananged in Github Issues, and/or [todo.md](todo.md)

<!-- TOC ignore:true -->
## Table of contents
<!-- TOC -->

- [Conventions](#conventions)
- [Backlog](#backlog)
	- [Non-code to-do](#non-code-to-do)
	- [Bugs](#bugs)
	- [New features and enhancements](#new-features-and-enhancements)
	- [Done](#done)
		- [First steps](#first-steps)
		- [Done - Bugs](#done---bugs)
		- [Done - new features and enhancements](#done---new-features-and-enhancements)
	- [Future and/or deferred](#future-andor-deferred)
	- [Canceled](#canceled)
- [Application name ideas](#application-name-ideas)

<!-- /TOC -->

## Conventions

In each section, items are listed approximately from newest to oldest.

| Icon | Status
| :--: | :--
| 🔘   | Not started
| 🛠️   | Started, and/or partially complete
| ✋   | Defer
| ✅   | Complete
| 🚫   | Canceled

## Backlog

### Non-code to-do

- 🔘 Enable GitHub Sponsors profile so the Sponsor link goes live.

- 🔘 Fill in the FUNDING.yml handles.

### Bugs

- ✅ Config file rewriting is proving problematic.
	- For example, when user makes a "non-standard" change (e.g. some extra comments), they get removed in the background, and the editor notices the file changed.
	- Fix: Only *write* to the file when A) Settings updated, or B) New options are added to the program. And in either case, first try to make sure nothing else has the file open for editing. If something else has it open:
		- If in settings, warn and don't close settings. (Force user to cancel, or abort other editing first.)
		- If writing new or changed program config settings, abort the write attempt, and output a non-alarming FYI to stderr.
	- Done: dropped the launch-time reorder/comment-refresh pass entirely - that was what stripped hand-added comments and reformatted the file behind your back. Launch now only rewrites for a real reason: a renamed/removed option (migrate) or a genuinely new one (backfill), both of which only add/rename and never touch your other lines. A hand-edited file (extra comments, reordered keys) is now left byte-for-byte alone on relaunch.
	- Done: before any write (launch-time migrate/backfill, remembered-size auto-save, and Settings save), a best-effort check sees whether another program has the file open (Linux, via /proc). If so: launch-time writes skip with a non-alarming stderr FYI; the Settings dialog leaves itself open on OK instead of closing over an unsaved change (the values still apply live for the session).
	- 🔘 Follow-up: make the "config is open elsewhere, not saved" signal visible IN the Settings dialog (a small banner), not just a stderr FYI + the dialog staying open.
	- Note: the open-elsewhere check only catches editors that hold the file descriptor open; an editor that opens/closes per save won't trip it, but in that case a write is harmless (backfill only appends).

- ✋ The dreaded "Nano Bounce Bug" is back. This will be the official bug report for it, but it is referenced elsewhere and I've taken multiple cracks at it - all unsuccessful and possibly red-herrings. It obviously must be related in some way to smooth scrolling (the next time it happens I'll try turning it off to make sure). So let's get back to basics of what I know, and don't know:
	- Steps:
		- Run nano. On any file, or with no file.
		- Observe: It "pops" onto the screen, but "wobbles", "violently", for maybe a second or two. If I recall, the wobbling is vertically up and down only - but my memory may be biased by what I believe "should" only be possible given the design and code. But at this point - who knows.
			- Note: It's short enough that it's livable (kind of cool even), but it's still a jarring effect for what is supposed to be a highly-polished terminal. (And by "kind of cool", I mean, if it were an opt-in, always happened "Compiz"-like "open-wobbly" effect. But we don't want that. We want stability.)
	- It's hard to recreate, so I don't know the steps to do it. But once it happens once, it seems easy to repeat. It only seems to start happening after a while - so maybe related to lots of input and/or more likely, output. And/or many switching of modes? Or just time?
	- ✋ Delay this to see if other fixes, fix this.

### New features and enhancements

- Rolling epic "GPU FX": Take more advantage of fundamental nature of underlying GPU terminal (all with non-GPU fallbacks - including no feature at all if necessary):
	- Note: These effects should come in "prepackaged effects" that can be applied to similar other types of on-screen elements.
		- Ideally as packaged plug-ins (think shader kits or something that be traded online and dropped into a directory for auto-discovery).
		- Reasonably easy for others to write new effect plugins that can be dropped-in, discovered at silkterm startup, loaded, and avaiable as an option.
		- Security model. Some plugins may need access to screen contents, others may not. If access to contents, make sure it can't do anything else - e.g. write to the filesystem, network, etc. Also, no reading from the filesystem, network, sockets - anything - except own config file.
	- 🔘 Effect 1: When a "copy on output" or "copy on select" happens, make the relevant checkbox and label gently burst with a glow and tiny fine sparkles for about a second - as if a fairy just blinged it with a magic wand in a movie.
		- Needs to be subtle and non-annoying over long-run, but definitely noticeable.
		- Tunable in config.
		- If it doesn't work well on non-GPU acellerated platforms, just some kind of noticeable blink. But still need visual feedback.
			- Need to decide what kind of feedback if not practical on non-GPU.
	- Effect 2: When a command or program returns to the prompt, give a burst of visual feedback, with a strength linearly proportional to the amount of time it took.
		- With an upper limit of course - say, an hour, config-tunable.
		- Config-tunable selection of predefined burst effects.
		- Default (and so far only): A glowing bright gold pulse that the cursor gives off upon landing back at the shell prompt, as if a yellow sun that shed an outer layer of blasma in a burst.

- ✋ Config file: For each feature listed below, allow user to list programs (comma-delimited), that, when running, temporarily disable:
	- Smooth scrolling. (Comma-delimited.)
	- Smooth cursor movement and blink. (Comma-delimited.)
	- Text scrim and outline
		- Note: Should not affect existing still-visible text renedered before the program's output, or new output following the output from the affected program that is still visible. (Comma-delimited.)
	- ✋ Deferred (design intent clarified): the scrim disable is meant to apply *only to that program's own output within a pane* - NOT per-pane / per-tab / per-window - so surrounding text (the prompt above, the resumed prompt below, unrelated scrollback) keeps its scrim. That is the hard part: the scrim is a single window-global pass (all glyphs -> one coverage texture -> blur -> composite), with no per-region concept. Honoring "just this command's output" for a normal-screen command like `ls` needs (a) tracking each command's output boundaries in the byte stream (start when the fg pgid becomes the command, end when it returns to the shell - the copy-on-output machinery), (b) mapping those logical lines onto current grid rows and re-mapping them every frame as things scroll and scrollback evicts, and (c) excluding exactly those cells from the coverage source. Fullscreen apps (vim/nano/less/htop) are the easy sub-case (the whole pane is their output), but the requested normal-screen case is not. Do NOT implement this as per-pane scrim on/off - that is a different, unwanted behavior.
		- Smooth-scroll and smooth-cursor disable are individually tractable (per-pane, gated on the foreground program) if ever wanted on their own; only the scrim sub-item is the blocker. Kept as one deferred item.

- ✅ Smooth cursor movement should speed up, if it falls too far behind where it actually is.
	- Done: the horizontal slide's time-constant now shrinks with the gap, so the cursor accelerates the farther it trails its real column (a fast burst / paste catches up instead of dragging across the line), while a single-cell move keeps the gentle slide. A hard cap also keeps it from ever sitting more than a handful of cells behind. Internal tunables (`CURSOR_CATCHUP` / `CURSOR_MAX_LAG`); feel-test on real HW and tweak if wanted.

- 🛠️ Scroll-on-output enhancement: One additional setting: (20260629)
	- 🔘 In-view fast output scroll speed. (E.g. for a short directory listing that doesn't exceed a single pane height.)
		- Faster than initial scroll speed, but ramps up slower, and top speed is slower than current.
	- 🔘 Once the top line of new output scrolls above and off the screen, then scroll speed ramps up as fast as necessary to fully keep up.
	- ✋ Held for your feel-test - scroll feel can't be judged headless, and this re-tunes the just-bounce-fixed output ease, so a blind guess risks the flagship smooth scroll. Proposed approach (not yet built): the ease already does a depth-based version of this (a short listing stays a shallow backlog near the slow "initial" speed; sustained output pins the backlog at the cap and ramps to the fast catch-up). To match the two named regimes, pass the pane height into `Scroll` and split the speed curve: while the cumulative advance since the burst started is under the pane height ("in-view"), use a middle tau that starts a touch quicker than initial, ramps gently, and caps below the current top speed; once it exceeds the pane height ("top line scrolled off"), ramp to the full fast catch-up. Expose the in-view start/cap/ramp as config knobs so you can dial the feel without a rebuild. Say the word and I'll build it with conservative defaults.

- ✅ New setting: Background image contrast mask - flatten the image's contrast so it stops competing with text.
	- Done: applies uniformly across the whole image, baked at load in linear light. A main on/off (default on) plus three 0..1 knobs (default 0.5 each): `size` = the flatten scale, the localMean radius (1.0 = half the longest pixel dimension, so the image collapses toward one tone; small = only fine detail flattens); `strength` = how far each pixel is pulled toward that local mean; `auto` = blends the manual knobs with values derived from the image's own busyness (1.0 = full auto, 0.0 = manual only, 0.5 = average). Config keys `background_contrast_mask` / `_size` / `_strength` / `_auto`; a Settings toggle + three sliders (sliders grey out while the mask is off).
	- Verified: on a busy wallpaper the mask visibly lowers image contrast while overall brightness stays put (a flatten toward the mean, not a darkening).

- ✅ Option to rotate background images from a folder; in order, or randomly. At startup, or on a timer.
	- Done: three config keys - `background_folder` (a folder, absolute or relative to the config dir; overrides `background_image` while set), `background_rotate_random` (filename order vs. random, never repeating the current image), and `background_rotate_interval_s` (seconds between swaps; 0 = pick one at startup only). Images are the formats the loader already decodes (png/jpg/webp/bmp/gif/tiff). Live swap reuses the existing wallpaper path, so it re-blurs and applies without a relaunch; a missing/empty folder just leaves the feature off.
	- Verified: cycled a folder of three solid-colour images on a 2s timer and confirmed the background changed in order.

- 🔘 Text fields in Settings dialog need to support standard editing functions. (Right-click, editing hotkeys, etc.)

- 🔘 After startup and enough time to settle down, auto-detect shells in the background. Dynamically pre-populate (or verify) the list of available shells, with user-friendly names. Bash, Dash, Ash, ZSH, PowerShell, Cmd, WSL2 Debian, Fish, PyCmd, YSH, Korn - do a web search for other common shells that might be installed.

- 🔘 Hyperlinks:
	- 🔘 Clickable - e.g. Ctrl+click, or right-click then includes "Copy link" and "Open link".
	- 🔘 Auto-underline when mouse is underneath.

- ✅ Tabs: Include a subtle 'X' icon in right edge of tab, to close with mouse.
	- Done: each tab reserves a right-edge close region with a dimmed "x" glyph; the tab title clips before it. A left click in that region closes the tab, elsewhere selects it.
	- Verified: the close glyph renders subtly at each tab's right edge; clicking it closes that tab, clicking the tab body selects it.
	- 🔘 Improve:
		- 🔘 Make the 'X' bigger or bolder, and put it inside a button outline nicely balanced within top, right, and bottom margins.
			- Done: the close "x" is now bold and centered inside a 1px outlined square button with equal top/right/bottom margins (the slack falls to the left, separating it from the title). The button box, its glyph, and the click region share one geometry helper so they stay aligned.
				- 🔘 X still too small and not centered in the box.
		- 🔘 Provide brief visual feedback on click - as the tab closes. Maybe the terminal area can close immediately while the tab lingers just enough milliseconds for human perception to notice the click feedback, if that doesn't require rejiggering the whole pipeline.
			- Note: two candidate approaches - a press-arm highlight (light on the button while pressed, close on release) that fits the existing input path, or the lingering-tab timed close described above (a short animation, more involved and feel-sensitive). Light on the button while pressed, close on release, is going to be the easiest, that's the winner.

- 🔘 Ctrl+Shift+N: New window on same directory.

- 🔘 Main menu and right-click menus:
	- 🔘 Accellerators need to be unique. If running out of memorable word/accelerator keys, remove accellerators from the least-used or least-important items, especially ones that already have hotkeys.
	- 🔘 List the hotkeys to activate the same function, if they exist. Keep in mind there might be a dynamic hotkey system soon.

- 🔘 Change wording of "background image opacity" to "background image visibility" (text and setting), to reflect that it's not just opacity. Still directly controls image/background color mix, but ALSO the contrast and saturation.

- 🔘 (Originally filed as bug): At high blur radius and low softness, the blur has boxy artifacts.
	- Cause: the scrim is a separable blur with a truncated kernel. The hard cutoff leaves a faint edge that low softness amplifies into a visible square, and the linear and s-curve falloffs are not true Gaussians, so their support reads as a diamond or box rather than a circle. The fix is a look-versus-performance tradeoff (wider extent, more taps, or a windowed kernel) that wants eyeballing. Deferred to a visual pass.
	- 🔘 New feature: Adjustable blur quality in settings:
		- High: Very high quality, may require a higher-end GPU, no visible artifacts at all.
		- Medium (default): The current quality.
		- Low: Trash quality, only looks OK at small blur radii. For VMs or remote sessions with punishing graphics. (In fact maybe this should be auto-detected...)

- 🔘 When reducing background image opacity, also reduce contrast and saturation. Add tunable parameters to the config file:
	- 🔘 Minimum contrast % (at 0% background image opacity - not useful but establishes the floor). Lets try a default of 50%.
	- 🔘 Maximum contrast % (at 100% background image opacity). Default 50%.
	- 🔘 Similar settings and defaults for saturation.

- 🔘 Need a way to detect maximum and average brightness of background image - or some human hueristic of "perceived brightness", and apply a variable ramp to background image visibility, so that it gets darker quicker, as the % goes down.
	- 🔘 Really what I'm after, is this resulting effect. The implimentation is up to research:
		- 🔘 At 100% background image visibility, it's just the image as-is.
		- 🔘 But below that, the opacity % scales with human perception.
			- 🔘 In other words, at say 90%, it is actually scaled to some average of ([perceived brightness], [brightest pixel]).
			- 🔘 As an example, 50% for a very bright image, may be significantly darker than 50% for a very dark image.
		- 🔘 And the inverse, for light-mode themes.
		- 🔘 Need a config file name and a default value for the resulting strength of this calculation.

- 🔘 Testing:
	- 🔘 Also try menus and dialogs with 125% larger font than current - independent of existing HiDPI tests.
	- 🛠️ Do full regression testing (and try to keep the tests updated as new features and bugs are added), and against library code as well.
		- Done: scrolling is covered by library tests encoding the per-app matrix (less/vim slide, nano/muffer hard-cut) plus normal-output invariants and easing monotonicity, and a harness that drives deterministic full-redraw scenes in the pipeline (skipped under `--quick`). Still to broaden: other features, and fuzz/security below.
	- 🔘 Add fuzz and security testing suites. Not just for SilkTerm code, but against library code too, so that we can find and patch critical bugs there too.

- 🛠️ Option to copy all output (`stderr` and `stdout`) to desktop clipboard automatically. (For security reasons this may need to be an always-visible checkbox on the right-side of the main menu, as well as accessible from the right-click menu.)
	- 🔘 Add Windows support.

- 🛠️ Tab interface:
	- Done: single-window core. Each tab owns a PaneManager; the tab bar shows once there's more than one tab, click to switch, and the pane area shrinks to make room for the bar.
	- Verified: new tab, switch (content swaps), close (bar hides).
	- Note: detach and dock are deferred - they need multi-window.
	- ✅ Close tab (CTRL+Shift+w, CTRL+F4)
		- Done: both shortcuts close the current tab, matching the menu.
		- Note: keeps at least one tab open. Shift on W leaves plain Ctrl+W for the shell.
	- 🔘 Detach tab to new window with mouse
		- Note: deferred, needs multi-window.
	- 🔘 Dock tab to different existing window with mouse
		- Note: deferred, needs multi-window.

- 🔘 Ability to change hotkeys, and/or assign new ones dynamically. Including a "capture" dialog.

- 🛠️ Themes:
	- Done (part 1): theme foundation and terminal palette. A Palette (bg/fg/cursor/focus + 16 ANSI) times a Theme (a dark+light pair); the theme and theme_mode config keys pick the active palette, and the [colors] keys still override per-colour. Three built-ins: SilkTerm, Matrix, Retro Amber, each dark and light.
		- Verified: Matrix is green-on-black including green-toned ANSI; SilkTerm light is dark-on-light.
	- Done (part 2): chrome/dialog theming plus System mode. Settings and About adapt to dark/light; the menu and tab chrome stay a fixed neutral gray. System mode follows the OS at startup and on theme-change, falling back to dark where the OS reports no preference (e.g. X11).
		- Verified: light mode gives a light dialog with dark text; system mode launches clean.
		- Note: still open - config-defined [themes.*], the Settings theme dropdown and its own tab, clearing per-colour overrides on re-select, per-theme menu colour (#166), more themes (Pastel, Solarized).
	- 🛠️ Provide a set of about 3 or 4 themes, each that support "Dark" or "Light" mode (or "System").
		- Done: three built-ins with dark and light.
		- Note: System (OS-follow) and a 4th theme are still pending.
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

- 🛠️ General configuration:
	- Done: the default-shell behavior.
	- Note: the named shell list and its UI (grid editor, Tab/Pane menus) are still to build by hand. The egui chrome migration was declined - see the note under "Setting dialog (part 2)".
	- 🛠️ Ability to define shells to launch in a new tab or pane.
		- ✅ By default, new tab launches the default shell for the window.
			- Done: new tabs and the startup pane use the default shell.
			- ✅ By priority: Global command shell override, non-empty shell specified in config file, or system default shell.
				- Done: order is the window --shell, then config default_shell, then system. A new pane also inherits from the pane it forked, its tab, then the window first.
				- Verified: a default_shell in config runs on the startup pane.
		- ✅ By default, new pane launches same shell as the pane the new one was forked off of.
			- Done: a pane stores its launch command, and interactive splits inherit it.
	- 🛠️ The shell configuration is stored in the config file as a simple key:value list of shell names and command lines. Command lines may have spaces, single quotes, and/or double quotes in them.
		- Done: a single default_shell string key, argv-split so it handles spaces and quotes.
		- Note: the named key:value list and its consumers (the grid editor and Tab/Pane menus below) are still hand-rolled work.
		- 🔘 In the settings dialog, this is accessed from a button that loads an additional modal dialog on top, with a 2*n grid of values. (That is editable like a typical database or spreadsheet grid.) This editable grid UX should be reusable for other potential future features.
			- Note: hand-rolled. Build it as a dynamic list of name/command rows with add and remove, reusing the dialog's text-field editing.
		- 🔘 The "Tab" and "Pane" menus (both on the main menu and popup menu sections) should both have dedicated sections to select the shell, both pulling from the same list of shells in the config. (With "[SilkTerm default]" always the first if one is defined in the config, and "[system default]" always the last no matter what).
			- Note: hand-rolled, follows the named-shell list above.
		- 🔘 If bash is available on the system, add a shell option just above "[SilkTerm default]": "bash --norc".
			- Note: deferred with the hand-rolled shell menu above.

- 🛠️ Setting dialog (part 2):
	- 🔘 Flyover help text when mousing over elements. (Make this a reusable feature.)
	- ✅ Size: A boolean setting to "Remember last size".
		- Done: remember_size config plus a dialog toggle. On launch it uses the remembered columns and rows. The pair updates on every manual window resize; startup and programmatic resizes are skipped so they don't clobber it. Columns and Rows grey out when on.
		- Verified: a manual resize persisted the remembered size, relaunch used it instead of the default, and the dialog shows the toggle checked with Columns and Rows greyed.
		- "Remembered" values stored separately in config, so that user can uncheck the boolean and revert to previous numericly defined size. These "remembered" values are not exposed in the settings dialog, only exist in config file. Always update to last manual window resize, whether boolean is yes or no.
			- 🔘 "Remembered" values always active, never commented out. But only valid if 'remember_size' is true.
	- ✅ All values, including slider numbers, should also have directly editable fields (that are part of the tab order).
		- Done: each slider has a numeric field you can click or type into, with the value clamped to the slider's range.
		- Note: the field joins the Tab order along with the rest of the dialog.
		- Verified: unit tests for editing and clamping, plus a render check.

- 🛠️ Command-line options:
	- Done (part 1, the options engine):
		- Full parser: create/select model, cascading style, shell-word-split, unit-tested.
		- --help / --version / --syntax, and --config for an alternate file.
		- Window options: columns, rows, pixel-width, pixel-height, background-opacity, hide-windowframe, hide-menu, fullscreen, title. A window option after a tab/pane marker errors.
		- Layout: --new-tab/--tab=/--new-pane/--pane=/--splits with direction and --size, building real tabs and panes (targeted splits into arbitrary trees, smart default direction, percent or cell sizes).
		- Per-pane --shell (argv-exec; cascades pane, split-source, tab, window, then config default_shell; interactive splits inherit).
		- Config command_line applied when launched with no args. Any real CLI argument overrides it entirely (verified both ways).
		- Tab --title override, shown in the tab bar (verified).
		- Window-level visual style: font, size, colours, and the background image with its stretch/zoom/opacity fold into the live settings at startup.
			- Note: per-pane scope is still deferred. It needs a per-pane renderer the single-TextCtx architecture lacks, so these flags apply to the whole window but don't yet vary per pane (hence 🛠️).
		- Note: still open - --keep-open (needs exit-status in a dead PTY), per-pane --title (reserved, none displayed yet), and finer field-level negotiation (today any CLI arg ignores the config command line wholesale).
	- General notes:
		- Command-line options override any config setting, but only while that window is alive.
		- As suggested in the main enhancement bulletpoint above, a command line can also be specified in the config file (and exposed in "Settings").
			- If the user launches the program also with command-line options:
				- Window-level options specified on the command-line at launch, override same command-line options stored in the config. (In other words, window-level options are "negotiated" between user-specified and config.)
				- If a single hierarchical option is specified by the user on the command-line at launch time, all hierarchical options from the config file are ignored.
	- 🔘 General format (unless we already inherited one):
		- `--option[=| ]value` | `-o value`
		- `--unary-flag` | `--unary-flag[=| ]\(true|t|yes|y|Y|1|false|f|no|n|N|0\)` | `-u` | ...etc.
		- In other words, even unary flags can be treated as options, and important options have single unique "short" versions.
	- 🔘 `--config[=| ]"alternate config file location"`
		- When active per-session, settings dialog should save to defined alternate.
		- All launches without this flag should default to existing config.
		- Configs are per-window, not per-tab.
		- Multiple windows can all have different configs specified and active. When a tab is undocked and moved to a different existing window, it automatically changes to that Window's config.
	- Window-level options (all options only apply to a single window per launch):
		- General:
			- Specifying window-level options after any tab/pane marker (`--new-tab`, `--tab`, `--new-pane`, `--pane`) should exit with an error.
		- 🔘 `--columns[=| ]<n>`
			- Primary way to specify window width
		- 🔘 `--rows[=| ]<n>`
			- Primary way to specify window height
		- 🔘 `--pixel-width[=| ]<n>`
			- Alternate way to specify window width
		- 🔘 `--pixel-height[=| ]<n>`
			- Alternate way to specify window height
		- 🔘 `--background-opacity[=| ]<n>`
		- 🔘 `--hide-windowframe[[=| ]bool]`
		- 🔘 `--hide-menu[[=| ]bool]`
		- 🔘 `--fullscreen[[=| ]bool]`
		- 🔘 `--help` | `-h`
			- Show program {name, version, and build#}, copyright and license, and list options and meaning.
		- 🔘 `--syntax`
			- Similar to `--help` but just list options and meaning.
		- 🔘 `--version`
			- Just show program name, version, and build#.
	- Hierarchical options:
		- General notes:
			- There is always an implicit first tab and first pane, each addressable by ID "0" or "main"; a window can never have zero tabs, nor a tab zero panes.
			- Create vs. select: `--new-tab` / `--new-pane` create a new tab/pane; `--tab=<id>` / `--pane=<id>` select an existing one. ID is required on a select - there is no naked `--tab` / `--pane`. Whatever was just created or selected becomes the "current" tab/pane, and subsequent options (and `--new-pane`s) apply to it until the next create/select.
			- Selecting an ID that doesn't exist is an error.
			- All options are logically under a single implicit 'window' (it can't be specified; it just means all options apply to one window).
			- Inheritance (most-specific wins): a pane's effective value = explicit on that pane, else inherited from the pane it splits (recursively up that chain), else its tab, else the window. A tab's = explicit on the tab, else the window. Flow: window -> tab -> [pane it splits, recursively] -> pane. Handles, title, and size are non-inheritable; direction inherits along the split chain, and the style options below inherit down the whole flow.
			- Order matters: options apply to the current tab/pane at the point they appear. You may re-select an earlier entity (e.g. `--tab=0`) later in the same command line to add panes to it or change its settings.
		- 🔘 `--new-tab[[=| ]<handle>]`
			- Create a new tab and make it current. Optional handle names it (unique within the window) for later `--tab=<handle>`. The implicit first tab (ID "0"/"main") always exists, so N `--new-tab`s => N+1 tabs.
		- 🔘 `--tab[=| ]<id>`
			- Select an existing tab (ID "0"/"main" or a handle) and make it current - to add panes or change its settings. ID required; selecting a nonexistent tab errors.
		- 🔘 `--new-pane[[=| ]<handle>]`
			- Create a new pane (splitting `--splits`, default = the current pane) and make it current. Optional handle names it (unique within the tab) for later `--pane=<handle>` / `--splits=<handle>`. The implicit first pane (ID "0"/"main") always exists and is never created by `--new-pane`.
		- 🔘 `--pane[=| ]<id>`
			- Select an existing pane (ID "0"/"main" or a handle, within the current tab) and make it current. ID required; selecting a nonexistent pane errors.
		- 🔘 `--title[=| ]<"Display title">`
			- Before any tab/pane marker: replaces the default window title. After a tab marker (`--new-tab`/`--tab`): replaces that tab's calculated title. After a pane marker: ignored (reserved for a possible future per-pane use; not an error).
			- Display only; not a handle, not inheritable.
		- 🔘 `--splits[=| ]<pane id to split>` (alias `--splits-pane`)
			- Only valid with `--new-pane`; error otherwise.
			- Optional. Default = the current pane in the current tab (resets to "0"/"main" after every tab create/select). Splitting the implicit first pane is fine - that's the first split.
		- 🔘 `--down` | `--up` | `--right` | `--left` `[[=| ]bool]`
			- Where the new pane goes relative to the pane it splits: `--down`/`--up` stack it below/above; `--right`/`--left` place it to the right/left.
			- Only valid with `--new-pane`; error otherwise.
			- Inheritable along the split chain: a later pane that splits this one reuses this direction unless it sets its own (handy for stacking a run of panes the same way).
		- 🔘 Default direction when a `--new-pane` gives none and has nothing to inherit: "right" or "down", whichever has more space. ("Save layout" always emits an explicit direction rather than relying on this.)
		- 🔘 `--size[=| ]<(n columns or rows | n%) of the split (parent) space in the split direction>`
			- Defaults to 50%.
				- Exception: a run of same-direction splits with no explicit size redistributes those adjacent undefined-size panes to ~equal in that direction.
			- Only valid with `--new-pane`; error otherwise. Not inheritable.
		- 🔘 `--shell[=| ]"command"`
			- Can contain escaped single and/or double quotes, as logically required by whatever quotes are used around the whole command.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🔘 `--keep-open[=| ]bool`
			- Keep pane|tab|window open after shell command exits, showing exit value.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--font-name[=| ]"string"`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--font-size[=| ]<n>`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--background-color[=| ]<hex>`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--foreground-color[=| ]<hex>`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--background-image[=| ]"path"`
			- Note: window-level applied, per-pane deferred.
			- No value = no background image.
			- Option not included = fall back to config value.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--background-image-stretch[[=| ]bool]`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--background-image-zoom[[=| ]bool]`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).
		- 🛠️ `--background-image-opacity[=| ]<n>`
			- Note: window-level applied, per-pane deferred.
			- Inheritable unless overridden (for panes, to any pane declaring this pane as its `--splits`).

- 🔘 Additional "File" menu option: "Save entire current layout to config".
	- Including window, tab, shell, and pane layout and configurations - everything.
	- Possibly to make this easier, store non-default per-tab and per-pane configurations as a "command line" in the config, that each override all other config settings.
	- Emits the create/select form: `--new-tab` / `--new-pane` (with explicit `--splits`, direction, and non-default `--size`) for structure, plus `--tab=<id>` / `--pane=<id>` for per-entity overrides. Always writes explicit directions and sizes (never the "more space" default) so a saved layout reproduces regardless of window size.

- 🔘 When running `sudo apt update`, the progress bar at the bottom bounces about halfway below the render area, as lines above it scroll up. This seems to be a side-effect of smooth-scrolling. Is there a way to prevent that from happening, without fundamentally breaking the very concept of smooth scrolling?
	- Opening `nano` can occasionally result in wild vertical jelly-like bouncing around for about a second. (Obviously something to do with smooth-scroll-on-output.) It doesn't seem repeatable though. Usually it opens just fine.
		- Maybe disable smooth scroll if direct raw access is detected?
	- Reopened: The first attempt (snap output easing during line bursts) broke smooth scrolling for all normal output and was reverted (see the smooth-scrolling-regression bug above).
		- Diagnosis: apt reserves the bottom line as a status bar via a scroll region, and each log line scrolls that region. Since the region starts at line 0, alacritty grows scrollback, which fires our output easing. The ease shifts the whole grid down by up to a cell and drags the fixed status bar below the viewport - that's the bounce.
		- Note: a proper fix needs to know a partial scroll region is active so it can suppress easing only then, but alacritty_terminal doesn't expose the scroll region. Options for later: patch the crate to expose it, tee and parse DECSTBM ourselves, or accept it like other full-screen apps.
	- Update: This actually seems to have fixed itself with some other work. Keep on backlog just in case.

- ✅ Build packages when cicd.bash `--quick` isn't specified:
	- ✅ .deb(s) + .rpm(s), per-architecture (cargo-deb / cargo-generate-rpm; metadata in source/Cargo.toml).
	- ✅ Windows installer .exe(s), per-architecture (single self-contained NSIS setup; upgrades in place). The release binary links only system DLLs, so no runtime is bundled.
	- Done: new stage 6 (Packages) builds from the stage-5 release binaries (never rebuilt). x86_64 always; ARM64 too unless `--no-arm`. Packages fold into the sha256sums. `--no-package` skips the stage.
	- ✋ Deferred (no cross toolchain on this Linux box): macOS `.dmg` (needs an Apple SDK / osxcross - license-gated) and BSD packages (needs a FreeBSD sysroot). AppImage/Flatpak also future.

### Done

#### First steps

- ✅ Create name and GitHub repo.
- ✅ Cargo skeleton: `alacritty_terminal` + `wgpu` deps.
- ✅ Glyph atlas + cell render.
- ✅ Wheel input -> lerp target.
- ✅ Boundary-cross sync to `scroll_display`.
- ✅ Overscan rows for partial-row fill.
- ✅ Output-scroll easing.
- ✅ Verify smoothness on X11/Compiz.

#### Done - Bugs

- ✅ Settings dialog changes not remembered after relaunch (surfaced as "Scrim falloff not saving"). The change showed live in the running app, then reverted on the next launch.
	- Cause: `persist` (and `revert_keys`) parsed config.toml with strict TOML, while the loader tolerates a bare-decimal float (`.1` with no leading zero). Any such value in the file made every save bail early and silently write nothing - so no dialog change stuck. Not falloff-specific.
	- Fixed: both now read through the same lenient pass the loader uses, so a save no longer aborts on a file the app reads fine. A malformed float is normalized in place on the next save. Regression test added.

- ✅ Some output, like debug output will bounce badly. I'm not sure how to reliably reproduce it on any machine.
	- Description:
		- Fast output (that nevertheless changes speed frequently) will scroll up the screen.
		- Suddenly it will "bounce" very far back down the screen, then scroll back up. Sometimes, the same content will repeat this process repeatedly.
		- The result is a flickering appearance, especially on fast output.
	- Cause: once the scrollback buffer is full, the output-ease infers how far the view advanced by matching row fingerprints against the last frame. That matcher demanded a pixel-clean translate of the whole retained region, so a single off cell - a redrawn prompt or spinner, a rewrapped line, or a multi-frame gap when a fast burst held the terminal lock - made it give up and report the full backlog cap instead of the true small advance. The cap snapped the view up about a screenful and eased it back; on fast, speed-varying output it misfired every few frames, so the view bounced far down and scrolled back up over and over.
	- Fixed: the matcher now tolerates a few off cells and picks the shift that best explains the frame, so a small advance reads as small. In-place redraws and static/blank fields still report no scroll, and a genuine full turnover still ramps to catch up. Regression tests added.

- ✅ Two new command-line options:
	- Change the wallpaper of the current window.
	- Reload settings for the current window
	- Done: `--wallpaper [PATH]` (no value = none) and `--reload-settings`, run from a shell inside a window. Each window exports a control socket to its shells (`SILKTERM_SOCKET`); the flags send a command to that window and exit. Wallpaper change is live-only (window-scoped, not saved to config); reload is the same as Menu > Reload config. Linux/Unix only for now (Windows has no such socket; the flags report that).

- ✅ Terminal is sometimes completely black after coming back from a long session. It responds to input, it just can't be seen - all the input and output is black. In some cases, the cursor, and cells with individually-colored backgrounds, are visible. (20260630)
	- Cause: when the glyph atlas fills up during a long, varied session, text preparation fails and rendering bailed out before the per-frame atlas trim. The atlas never recovered, so text stayed black. The cursor and per-cell backgrounds use a separate renderer, so they kept showing.
	- Fixed: trim the atlas on the prepare-failure path, so the next frame re-prepares with room and recovers.
	- Note: could not force an atlas-full for a live repro; the trigger needs a genuinely long session.
	- Verified: a sustained high-rate unicode flood stayed visible throughout with no black-out. The exact atlas-full trigger is still unreproduced, since the available fonts can't fill the atlas.
	- Resolution: leave open until confirmed on long-running terminals.
	- Verified: Days-long running shells are now fine.

- ✅ When switching fonts then hitting "OK", the font changes but not the blur. An exit and reload is required to sync them up.
	- Must have been incidentally fixed as part of some other thing, doesn't do this anymore.

- ✅ When the terminal is completely is full of text, it's slows noticeably even on a high-end gaming rig from 4 years ago. Not sure if unicode fallback is part of that problem, and/or a full buffer, it might be.
	- Steps to reproduce: `cat /bin/Thunar | convert-base-v2 --from binary --to 256jc1`
	- Cause: it is the unicode fallback, not the full buffer. Each cell whose glyph the primary mono font lacks was re-shaped from scratch every frame - through the full font-fallback matching path - even though the same character shapes identically each time. A screen filled with mixed-script glyphs meant thousands of redundant per-cell shapes per frame. A flamegraph of the repro put this single step (`fill_glyph`) at ~16% of all CPU, while the main text shape was under 1% (fallback cells are placeholders in the main buffer), ruling out the "full buffer" theory.
	- Fixed: shape each distinct glyph (keyed by character + bold + italic) once and cache it per pane, tinting per cell at draw time; the cache drops on a font or size change. A re-profile of the same flood dropped `fill_glyph` from ~16% to ~0.2% and the whole build step from ~17% to ~1%.
	- Verified: pixel-identical output vs the pre-change build on a colored + bold mixed-script scene (no visual change), plus the before/after flamegraphs above.

- ✅ Choosing "Tabs|New Tab" the first time, opens a second tab. Doing it again, changes to the first tab, rather than opening a third tab.
	- Cause: a dropdown opens flush under the menu bar, so its top item ("New Tab") sits in the tab-bar band. The mouse handler checked the tab-bar hit before the open-menu hit, so once more than one tab existed (tab bar shown) the tab bar stole the click and selected a tab instead of firing the item. The first New Tab worked only because there was no tab bar yet.
	- Fixed: skip the tab-bar click handler while a dropdown is open, so the click reaches the menu.
	- Verified: repeated "Tabs|New Tab" now grows the tab count instead of toggling back to the first tab.

- ✅ Bug #t78br: "The Notorious 'Bouncing Shadow' nano bug" (which we'll call this subset) is still still there. (At least the wobblyness seems to be fixed, which is why this now gets its own issue.):
	- Steps to reproduce:
		- Open nano with a long file - say, ~/.config/silkterm/config.toml.
		- Observe:
			- A sipgle-line bar at the top, rendered with terminal's text color as the bar's background color, and (apparently) the terminal's background color as the bar's text color. It says "GNU nano 8.7.1" on the left, and the open filename in the center. This bar never moves or scrolls, for as long as nano is open. For reference, we'll call this UI element, 'TIMMY THE TOP BAR'.
			- Nano has reserved three rows at the bottom of the terminal, for itself as fixed, non-scrolling UI areas. The bottom two rows show the user what hotkeys they can use - both in the same inverse text style as 'TIMMY THE TOP BAR', and also regular terminal text. For reference, we'll call this UI element: 'BILLY THE BOTTOM AREA'
			- The area that file content is rendered in, and the user can move the cursor around and edit in, we'll call 'THE EDIT AREA' for reference.
			- The entire terminal, in vertical terms, is composed of - by the definition of our words, from top-to-bottom: 'TIMMY THE TOP BAR', 'THE EDIT AREA', and 'BILLY THE BOTTOM AREA'.
		- Action:
			- Now contiuously hold down the 'down arrow' key to move "down" the file contents.
			- When the cursor get to the bottom edge of 'THE EDIT AREA', keeep holding down 'down arrow'.
		- Observe:
			- When nano pushes the content from below its view up into view, what appears to be the dark outer glow + outline effect from the text on 'TIMMY THE TOP BAR', visually "bounces" down from the top, visually into 'THE EDIT AREA'.
			- For reference, we'll call that text 'TIMMYS TEXT SHADOW',
			- When you stop scrolling, 'TIMMYS TEXT SHADOW' gradually "settles" back "under" 'TIMMY THE TOP BAR'.
		- Observe:
			- You can make the same thing happen when pressing the down-arrow key one at a time, it's just not nearly as pronounced of an effect.
		- Observe:
			- You can make the same thing happen when scrolling the text in the same direction by using the mouse wheel quickly (which in nano is rewired to drive just the cursor, not 'THE EDIT AREA' - but with fast enough mouse wheel moves, the effects observed above can be much more dramatic.
		- Action:
			- Move all the way to the bottom of the file, so we can test the same thing as above but in reverse.
			- Now contiuously hold down the 'up arrow' key to move "up" the file contents.
			- When the cursor get to the bottom edge of 'TIMMY THE TOP BAR', keeep holding down 'up arrow'.
		- Observe:
			- The same thing that happened to 'TIMMYS TEXT SHADOW' previously, happens in the reverse vertical direction now only involving the inverse text in 'BILLY THE BOTTOM AREA'. It visually bounces UP into 'THE EDIT AREA'.
			- At the same time and synchronized with, visually identical copies of the normal text in 'BILLY THE BOTTOM AREA' also bounce up into 'THE EDIT AREA'. Together they seem to exhibit the same movement behavior as 'TIMMYS TEXT SHADOW', except flipped vertically.
	- Cause: the sliding draw is the whole frame translated by the eased offset, clipped only at the band boundaries - so the top bar's row translated down (and the bottom area's rows translated up) landed inside the scroll-region clip and rendered as translated text copies riding the ease. Text and its glow only (cell backgrounds are placed per row), which is why it reads as a text shadow at the top and as text copies at the bottom. (20260708)
	- Fixed: the region clip now welds to the shifted content's own edge; the strip fills the gap on the far side of the weld, and translated band rows can no longer enter. (20260708)
	- Verified: reproduced the ghost in mid-slide frame dumps before the fix, gone after; scroll harness all four scenes pass; 113 lib tests. Feel-test passed; merged with the parent spike. (20260708)

- ✅ A bad config value could kill the whole terminal. Setting `output_ease_lines` above 16 aborted on the first scrolling output, every launch. (20260707)
	- Found: code review, 20260707.
	- Cause: the value was never range-checked at load. The scroll code uses it as the lower bound of a clamp, and a lower bound above the cap makes that clamp abort.
	- Fixed: the value is clamped at load. The scroll code also guards itself now.
	- Verified: reproduced the abort, then re-ran the same setup on the fix. Covered by a unit test.

- ✅ "Copy output" copied the wrong text once scrollback was full. The first lines of a command's output were silently missing from the clipboard. (20260707)
	- Found: code review, 20260707.
	- Cause: the capture start was saved as a line index counted from the oldest line in the buffer. At the scrollback cap every new line evicts the oldest, so the index drifts while the command runs.
	- Fixed: the capture now remembers the prompt line's content and re-finds it when the command settles. The saved index is only a fallback.
	- Verified: a regression test replays the eviction case and checks the full output comes back.

- ✅ Moving the mouse over a full-screen app that tracks the mouse re-rendered everything. (20260707)
	- Found: code review, 20260707.
	- Cause: each motion report also flagged a full redraw, so every pane re-shaped its text once per cell the pointer crossed.
	- Fixed: motion reports go to the app only. Nothing local changes, so nothing redraws.

- ✅ Menu-bar and tab text was re-shaped from scratch every frame. Constant background work during any animation, even the idle cursor pulse. (20260707)
	- Found: code review, 20260707.
	- Fixed: shaped menu titles, tab titles, and the tab close icon are kept between frames. A tab title re-shapes only when it changes. Everything drops on a font change. Measured label widths are cached the same way.

- ✅ `--background-image` with no value swallowed the next option as its path. (20260707)
	- Found: code review, 20260707.
	- Fixed: a bare flag now means "no image" and a following option is left alone. Both `=path` and a separate path still work. Covered by a unit test.

- ✅ Launching with only `--config` ignored that config's `command_line`. (20260707)
	- Found: code review, 20260707.
	- Cause: any argument at all disabled the fallback. But `--config` picks which config to read, it isn't a layout choice.
	- Fixed: the fallback still applies when the only arguments are `--config`. Covered by a unit test.

- ✅ "Copy output" could silently skip a command. (20260707)
	- Found: code review, 20260707.
	- Cause: arming the capture at Enter gave up if the terminal was briefly busy, with no retry.
	- Fixed: arming now waits the moment out instead of giving up.

- ✅ Releasing a different mouse button than the one held confused mouse-tracking apps. (20260707)
	- Found: code review, 20260707.
	- Cause: any button release was treated as the release of the held one. That cleared its state and sent the app a release it never saw pressed.
	- Fixed: only the matching button's release is reported. Other buttons keep their normal handling.

- ✅ Bug in double-click to select (then Ctrl+shift+C).
	- Steps to reproduce: The specific command was `zpool status`. Trying to double-click on a member by label (e.g. "zfs-..."), or "ONLINE", results in something else being selected. It appears to actually select something to the right. But if you can guess correctly on your aim, then hit the copy hotkey, it does correctly copy the text. (Just not the text that's highlighted.)
	- Cause: `zpool status` indents its config section with a literal tab. The raw tab was passed through to the shaper, which expands it to a full 8-column stop. That shifted the row's visible text several columns right of the grid the selection uses. The highlight and copy stayed correct but no longer lined up with the on-screen text, so clicking a visible word selected a cell several columns away. Only tab-indented output was affected.
	- Fixed: render any control character in a cell as a plain one-cell space, so the tab cell advances one column and the row stays grid-aligned.
	- Verified: on tab-indented output, double-clicking a word now selects that word. Covered by a unit test.

- ✅ Inverted text (e.g. Nano headers) is thin and hard-to-read.
	- Cause: this was the actual nano complaint (the "shadow jump" language was describing it). Reverse video (dark on light) renders visually thinner than the same-weight light-on-dark text, an inherent effect that other terminals also show. The glow only boosts light-on-dark text, so inverse text got no readability help.
	- Fixed: a new `embolden_inverse` config bool (default true) renders reverse-video runs bold so they read as strongly as normal text. Verified: inverse text is visibly thicker with it on, though the delta is modest with the default font. Needs a feel-test; if too subtle, the next step is faux-bold (stroke dilation).

- ✅ The Notorious "Bouncing Shadow in Wobbly Nano" bug [20260707]:
	- **NOTE**:
		- The "Bouncing Shadow" portion of this has been moved to #t78br, "The Notorious 'Bouncing Shadow' nano bug", to tackle independently.
		- The "wobbly nano" portion of is fixed.
		- **Overall, this was documented with a poor (but growing) understanding of both, so is not the best representation of either. Closing it for good. If regressions occur, they'll get new issues.**
	- Originally: Smooth app-scroll (`smooth_scroll_apps`) left a blank band above/below the text that grew with scroll speed, and stepped one line at a time before easing. (20260703)
	- Cause: the slide shifted the scroll region by several lines but only one row was ever drawn, so the revealed strip was bare background. The scrolled-off lines are gone from the grid, so there was nothing real to fill it with.
	- Fixed: retained-frame slide. The pane keeps the previous frame's text and draws it, clipped to the revealed strip, so the strip fills with the real outgoing content while the current frame slides in over it.
	- Verified: across continuous multi-line slides the content fills top to bottom with no blank band.
	- Verified:
		- Works perfectly in `less`.
		- `nano` exhibits none of the bugs listed above, but it also doesn't scroll smoothly, either with the mouse wheel or via cursor. (In fact, the mouse wheel just moves the cursor up and down. That's standard `nano` behavior, but the note is that scrolling isn't smooth. The cursor vertical movement also isn't smooth (horizontal is). Nano doesn't neeed to have a per-app fix, if it can even be "fixed".
	- 🛠️ muffer now scrolls smoothly on output - but still not mouse wheel.
		- Cause: a wheel notch makes the app repaint a bigger jump than line-by-line output, past the detection window, so it read as not a clean scroll and hard-cut. Raised the detection cap (gated by `smooth_scroll_apps`).
		- Note: the slide retains only the single previous frame, so fast wheeling can still lag about one step (looks like snapping). Smoothing that fully needs retaining more frames, a bigger change. Feel-test the cap first.
	- 🛠️ Static-top-band fix (nano/muffer wheel = no change; less fine). Dogfood: the cap-24 bump didn't help nano or muffer on the wheel (muffer wheels 1 line/notch, well inside the window - so it was never a cap problem).
		- Cause: the shift detector only matched a run anchored at the top row, and the renderer slid the whole pane from its top. `less` fills from the top with only a bottom status line, so it worked. `nano` and `muffer` keep a static title bar at the top; its unchanging first row broke the top-anchored match, so no slide engaged, and even if it had the title would bounce.
		- Fixed: the shift detector now matches wherever the most rows translate, tolerating static bands at both ends, guarded so a static or blank screen can't false-trigger. A static top band is detected and its title bar redraws unshifted while the region below it slides. Apps with no top band are unchanged, and app-scroll stays alt-screen only, so apt is unaffected.
		- Pending: a feel-test - nano and muffer wheel one notch should ease, not snap, the title bar should stay put, and less should be unchanged. Still gated by `smooth_scroll_apps`.
	- ✋ Residual band jitter during a slide (nano; "almost perfect" otherwise). Two symptoms, different causes:
		- Text moving up (content scrolls up): the drop-shadow under the inverse-video header title jumps down.
			- Note: a partial fix stopped the glow from applying over any cell with its own solid background (reverse video, coloured background, selection), since those already have full contrast. This removed the header's static halo but did not fix the reported symptom, which is a motion artifact.
			- Cause: the retained-frame slide fills the revealed strip with the previous frame's text but does not glow that strip. During a down-slide the rows just below the header lose their readability backing, and as the slide settles the backed and unbacked boundary marches down - that is the shadow jumping down.
			- Fixed: the glow pass now also glows the previous-frame strip, so revealed rows keep their readability backing and the boundary no longer sweeps. Guarded so it only applies when the relevant static band is detected, which clips the previous frame's header and status out of the glow.
			- Verified: the header stays clean and the strip is glowed, with no blobbing in the edge case. Needs a feel-test on real nano to confirm the wheel and cursor feel.
		- Text moving down fast: the bottom two lines jump up. Likely the same un-glowed-strip issue at the bottom edge, now covered by the same fix. If any residual jump remains after the feel-test, the leftover is band re-detection mid-ease; the fix would be to hold band sizes stable across an in-progress ease.
		- Note: freezing the band sizes did not help (re-tested: looks the same as before). The bands were already stable, so band jitter was never the cause. The real signal was the scroll offset itself oscillating frame to frame, which is the bounce.
		- Note: an accumulation attempt made it worse (re-tested: jumps much farther). Accumulating the offset for the current content was right, but accumulating the strip fill from one stale snapshot was wrong - when the shift outgrew the scroll region the snapshot was re-captured, jumping the reveal strip by a whole screenful. That periodic jump was the farther bounce.
		- Fixed: keep the offset accumulating for smooth content, but re-snapshot the previous frame every step so the strip is always one fresh step back. One retained frame only fills a one-step strip, so a fast burst could still open a blank band; a lag ramp on the ease bounds that by easing faster as the lag grows. A regression harness measured no content bounce and no band-boundary jumps across gentle, fast, and wheel scrolling, with the blank band shrinking to about one line. But a residual on real nano over a background image was still visible.
		- Deferred: title-bar apps hard-cut for now - the smooth slide only engages when there is no static top band, so `less` still slides and nano and muffer just page-redraw as before, with no slide and so no bounce. The enter and exit hard-cut fixes are untouched. Re-enabling the slide for title-bar apps needs multi-frame retention so the reveal strip always fills regardless of lag. Verified: title-bar apps hard-cut while `less` still eases smoothly.
		- ✅ Re-enabled the slide for title-bar apps, replacing the retained-frame fill with a scrolled-off strip. (20260707)
			- Cause of the residual: filling the reveal from one retained frame is structural bounce. The fill could trail the ease by a few lines - a bare, un-glowed band whose height varied step to step, the pulsing shadow under the title over a background image - and the fill repositioned at every re-capture.
			- Fixed: each frame the styled rows are snapshotted, and the rows a detected step pushes out of the region are kept in a small strip, drawn welded to the content edge and riding the same eased offset. The gap is always exactly filled, nothing repositions, and the strip carries its own cell backgrounds and glow. Band bleed is impossible by construction (only region rows are ever captured), so the old glow guards went away.
			- Fixed alongside: sliding rows' background rects and the cursor now clamp to the scroll region, so an inverse-video or coloured row can't poke into the title/status bands mid-slide.
			- Verified: headless scroll harness - all four scenes (less, vim, nano, muffer) slide monotone with zero bounces and correct bands; 112 library tests including strip ordering, trimming, direction flip, and row selection.
			- Feel-test passed after the #t78br band-ghost fix; merged to main. (20260708)

- ✅ "Right-click bug" clarification.
	- Cause: a mouse-tracking app (muffer/vim/tmux) grabs the mouse, so the right-click was forwarded to it (muffer pastes on right-click) instead of opening our menu; and a click meant for an open menu was being reported to the app underneath, so menu items did nothing. `nano`/`less` don't grab the mouse, hence unaffected.
	- ✅ Fixed: right-click is now reserved for our own context menu and never forwarded to the app; and while any menu is open a click operates/dismisses the menu instead of going to the app. Left/middle/wheel still forward, so apps keep normal mouse use.
	- Verify on hardware: right-click in muffer opens our menu (no paste), and menu items work.
	- Steps to reproduce:
		- Open terminal.
		- Run `muffer`.
		- Right-click on terminal.
		- Observe: A *clipboard paste* occurs.
		- Try to do anything with the menu.
		- Observe: A menu can open, but nothing else.
		- Switch to another application, then return.
		- Observe: Menus work, until you right-click.
		- Note that you may only to do this once or twice - until menu actions stop working pemanently.
			- However, CTRL+Shift+T can open a new tab, and everything works fine for that tab.
		- If you exit `muffer`, some things work and some things don't.
			- Split vertical works
			- Split horizontal works
				- Split vertical then works in both panes.
	- None of these issues present in `nano` or `less`.

- ✅ Mouse-scroll doesn't work in Muffer (running inside SilkTerm). (20260703)
	- Cause: SilkTerm implemented no mouse reporting at all - clicks, motion, and wheel were only handled locally, never encoded to the PTY. So when an app turns on mouse tracking (DECSET 1000/1002/1003, e.g. Muffer enabling it to receive wheel events), it got nothing and its scroll did nothing; the wheel just drove SilkTerm's own scrollback.
	- Done: added standard mouse reporting (`input::mouse_report`, SGR 1006 + legacy X10). When the focused pane has tracking on, wheel reports as button 64/65, and clicks/release/drag/motion report too; Shift is the local-action override (select/paste/menu/scrollback still work). Wheel sends one notch-event per line (capped), de-duped motion per cell.
	- Verified: unit tests cover the SGR + X10 encodings, wheel, modifiers, and the no-tracking case; live-verify by scrolling in a mouse-tracking app.

- ✅ Now there's too much space below the tab text and top menu text. (Ironic since earlier there was too little.) It should be vertically centered.
	- ✅ Proper fix: Size both the menu and the tabs according to the font height (plus extra), then *vertically center* the text within that area. If the font was created poorly centered, which may are, then there may be nothing to do about that - but the current font seems properly designed elsewhere.
		- Done: both bars center the text on its real visible box using the UI face's actual ascent/descent, instead of the old hand-tuned padding that left titles riding high.
		- Note: tab bar padding dropped to match the menu bar now that centering handles descender clearance.
		- Verified.

- ✅ Menu bar and tab fonts: (#1n45bca, 20260629-103822)
	- ✅ Tab font doesn't have enough space on the bottom. Tab height should adapt to tab font size. (20260630)
		- Done: the bar and tab height scale with the menu font. Descenders were sitting tight against the button bottom, so the vertical padding was bumped up a couple px to clear them.
		- Verified: descender-heavy tab titles clear their descenders.

- ✅ Menu bar and tab fonts: (#1n45bca, 20260629-103822)
	- ✅ Currently using "system sans serif", but if system proportional font is serif, the menu font is incorrect. For example my system proportional font is a Serif font, not sans serif. (20260629)
		- Cause: chrome used generic `Family::SansSerif`. fontdb's generic-sans default is "Arial"; when that's absent (typical on Linux) the query falls through to whatever matches - here the GNOME *document* font, which is a serif (GentiumAlt). (fontconfig's actual sans-serif on this box is Noto Sans.)
		- Fix (first pass): pin a concrete sans family, mirroring the mono pin - resolved the OS sans-serif (`fc-match sans-serif`), else a curated list, validated against the db. Got "Noto Sans" - still a sans, which missed the point below.
			- ✅ Not fixed: Still using system *sans serif*, rather than just system font generally. (Which on my system is a *serif* font.)
				- Fixed: chrome now follows the desktop interface font - family, size, weight, slant, serif or not. It's read natively per platform, and the whole chrome sizes from the real rendered text, so a large or wide font grows the chrome instead of truncating.
				- Verified: menu bar, dropdowns, and Settings render the desktop's bold serif font; terminal text is unaffected.
		- ✅ Verify that menu bar height adjusts based on menu font.
			- Verified: the bar heights equal the menu font's line height plus padding, so a larger menu font grows the bars.
		- ✅ Still sans-serif after the 20260701 fix (reported: bold + bigger took, family didn't).
			- Cause: cosmic-text only uses the requested family when a face matches the requested weight exactly, and GentiumAlt ships no Bold face. So asking for bold silently ejected the family and a bold sans rendered instead - which is why bold and size took but the family didn't.
			- Fixed: pin the font db's canonical family spelling and snap the requested weight and slant to a face the family actually has, so family wins over weight. A shaping test guards it.
			- Verified: menu bar and Settings render the serif family at its closest weight; cosmic-text doesn't synthesize bold.

- ✅ Outer glow should only apply to terminal text - not tab titles or the menu bar. (20260630)
	- Cause: the glow composite covered the whole window, so the halo showed behind the menu and tab titles too.
	- Fixed: clip it to the content area below the chrome, so only terminal text glows.
	- Verified: with strong glow, chrome text stays crisp with no halo.

- ✅ High severity: Typing "exit" in tab, closes the whole application. It should only close that tab. Doesn't do that for panes, only tabs. Closing a tab via menu only closes that one tab. (20260629; real cause found + fixed 20260630)
	- Cause: the shell-exit handler (`UserEvent::Exit(id)` in app.rs) just called `tabs.cur_mut().close(id)` and quit the app whenever that returned true. So the last pane of a tab killed the whole app when other tabs existed; worse, a background tab's shell exiting ran `close(id)` on the *active* tab (which doesn't own that pane) -> returns true -> app quit. The Close-Pane menu had the right pane->tab->window cascade; the exit path didn't.
	- Fix: `UserEvent::Exit` now finds the pane's owning tab (`position(|pm| pm.panes.contains_key(&id))`) and applies the same cascade - >1 pane in that tab closes the pane; else >1 tab closes that tab (`close_tab_at(idx)`, generalized from `close_tab`); else (last pane of last tab) exits. Handles background-tab exits and keeps `active` pointing at the same tab.
	- Verified: with a second active tab whose shell self-exits, the app stays alive past the exit (the tab closes, the window survives) instead of quitting.
	- Re-verified fixed on current main (20260630): the app survives the tab's shell exiting in all three cases - active-tab exit, background-tab exit, and typing `exit` interactively in the active tab of a two-tab window. If it still happens, the running build predates the fix; rebuild or reinstall, then retest.
		- ✅ Still not fixed. With three tabs open, for example:
			- Type "exit" in the anything but the last tab, it closes ALL tabs, except for one. Sometimes, the program becomes unresponsive then and has to be killed.
			- Type "exit" in the last tab, it closes the program.
			- With four tabs open, and type "exit" from the third, closes the first two tabs (and not the third).
		- ✅ REAL cause (20260630): pane ids collided across tabs. Each tab is a separate PaneManager that assigned ids from its own counter (first pane always id 1), so the shell-exit event (carries only the id) resolved to the WRONG tab - the first one with that id - and closed it; dropping that tab's term fired another Exit -> cascade (closed all but one, sometimes hung), exactly as reported. The earlier fix (find the owning tab + cascade) was right in shape but the id lookup was ambiguous. Fix: `alloc_pane_id()` - one global counter, so every pane is unique everywhere. Verified: an exit in a background tab closes exactly that tab, with no cascade and the window staying alive.

- ✅ Cursor: (20260629)
	- ✅ Smooth-scroll (when moving to the right).
		- Done: the cursor slides to its target column as you type, snapping on a newline. Idles at 0% CPU.
	- ✅ Blink at the same rate, but "phase" between of and on, not just on or off.
		- Done: a smooth cosine fade, on by default. A render refactor skips re-shaping text on cursor-only frames, so blinking no longer pegs the CPU. The cursor_blink config disables it.

- ✅ Setting dialog: (20260629)
	- ✅ Setting Bg image fit to "Zoom", then Apply works. But back to "Stretch", then Apply, doesn't.
		- Cause: the dialog's baseline was captured when it opened and never refreshed, so a second Apply diffed against the open-time snapshot and re-selecting the original value read as no change.
		- Fixed: reset the baseline after each Apply. This fixes every setting, not just fit.

- ✅ Critical: Smooth-scrolling apparently just quits after using the terminal for a while. It seems to quit, if output is too fast for a while, but that could be a red-herring. Maybe it's just after any particular amount of general use.
	- Cause: output-easing was triggered off scrollback *growth* (`grid.history_size()` rising). That growth flatlines once the scrollback buffer fills (default 10k lines) - old lines drop off the top as fast as new ones arrive - so after enough output the growth reads 0 every frame and `nudge_output` never fires again. Smooth output scroll dies "after a while", and sooner under fast output (which fills the 10k buffer faster). Manual scrollback (wheel) was unaffected, which is why it looked like only the smooth *output* scroll quit.
	- Fix (`pane.rs`): keep growth as the primary signal (unchanged pre-cap, so the verified feel is untouched), and at the cap fall back to inferring the viewport advance from row fingerprints - how far last frame's on-screen rows reappear shifted up this frame (`scroll_shift`). An in-place bottom-row redraw (e.g. apt's status line, no newline) shifts nothing, so it still doesn't nudge (no bounce); a full-screen burst reports the backlog cap so the ease ramps to full catch-up. 6 unit tests cover no-scroll / in-place / shift-by-k / full-turnover / empty.
	- Verified: 26 unit tests pass; ran past the 10k cap (20k-line flood + drip) with no crash, rendering on the GL backend. Smooth-scroll *feel* past the cap is best eyeballed in normal use.

- ✅ Mouse wheel doesn't scroll back through the `stdout`/`stderr` buffer. It should do so, smoothly, and in proportion to how fast the mouse wheel is moved. But currently it moves the command history back. (20260626-104542)
	- Cause: `TermMode::ALTERNATE_SCROLL` (DECSET 1007) is default-on in alacritty_terminal, but the wheel handler used `ALT_SCREEN || ALTERNATE_SCROLL` as the cursor-key trigger - so on the *primary* screen the always-on flag made the wheel emit cursor-up/down (shell history recall) instead of scrolling scrollback.
	- Fix: gate the cursor-key path on `ALT_SCREEN` (now requires alt screen AND alternate-scroll AND no mouse mode). The primary screen always routes to the smooth scrollback (`Scroll::wheel`, already proportional to notches via `wheel_lines` + easing). Alt-screen apps (less/nano/vim) keep their cursor-key wheel. `app.rs` MouseWheel arm. Verified by root-cause + build (runtime wheel injection can't be scripted reliably).

- ✅ Severe bug: Trying to open the settings dialog crashes the program. (20260625-150526)
	- Cause: on X11 the main window holds a GL context, and the pop-out dialog created a second graphics instance that also tried to init GL, which panicked because a GL context was already current. It only showed with a transparent (GL) main window, so a default-config main masked it.
	- Fix: dialogs now create their `Gfx` via `Gfx::with_backends(window, Backends::PRIMARY)` (Vulkan/Metal/DX12, no GL) - opaque dialogs don't need GL, and avoiding it sidesteps the EGL conflict. Verified: Settings + About open over a transparent GL main with no crash; toggle on->Opacity enabled, off->greyed.

- ✅ Mouse text selection, and double-click selection, quit working. (20260625-161509)
	- Cause: It was actually the selection highlight being invisible (input + copy-to-PRIMARY worked): the GL offscreen was `Rgba8UnormSrgb`, so the blit's `textureSample` decoded sRGB->linear, cancelling the blit's `lin2srgb`, and wgpu's GL backend doesn't sRGB-encode the offscreen write either - so all rect/text colors passed through as raw linear and rendered too dark (text ~64% looked "ok"; the dark `SELECTION_BG` (51,68,102)->(8,15,34) went invisible). Fix: make the GL offscreen plain `Rgba8Unorm` so shaders store their linear output raw and the blit's `lin2srgb` does the one true encode - uniformly for rects, glyphon text, and the bg image. Verified: SELECTION_BG renders (50,69,102), text is full-brightness (210). This also completes the earlier transparency sRGB fix (text was still ~164, now a true 210).

- ✅ Smooth scrolling is broken. (20260623-194551)
	- Cause: the fix for the apt "bug". That fix made output easing snap whenever new lines arrived closer than 0.12s apart, to stop apt's status bar bouncing. But a command's output arrives from the PTY in one sub-millisecond burst, so essentially all multi-line output (the core demo) snapped instead of easing - smooth scroll gone. Any burst threshold above a frame breaks the feature.
	- Fix: Reverted the burst-snap entirely (`Scroll::nudge_output` back to always easing while following; dropped `output_age` / `OUTPUT_BURST_GAP_S`).
	- Verified: Smooth output scrolling restored. The apt status-line bounce is reopened below as its own item (needs a non-destructive approach).

- ✅ "Close pane" menu items don't work.
	- Cause: The action itself works with multiple panes (verified: right-click and Panes-menu Close both closed a pane, 3->2->1). The dead case was the last pane: `MenuAction::Close` was gated on `panes.len() > 1`, so on a single pane (the startup state, where you'd first try it) it silently did nothing.
	- Fix: Now Close Pane on the last pane closes the tab (if >1 tab), else the window.
	- Verified: single pane + single tab -> Close Pane exits.

- ✅ Text background colors, and the block cursor, appear to be aligned a line below where they should be.
	- Cause: a regression from the menu bar. Cell backgrounds, the cursor, and the bars are positioned in full-window pixels, but the resolution was being fed the shorter content-area height, so every quad was pushed down relative to the text.
	- Fix: Pass the full window size (`gfx.config.width/height`) to both `set_resolution` calls.
	- Verified: ANSI bg-color spans sit exactly on their text and the block cursor is on its own row.

- ✅ The text and UI elements in the settings dialog are misaligned. But before fixing it, make sure we're not going with egui.
	- Cause: the dialog vertically centered text with a baked-in 18px text height, so on fonts whose line height differs the labels/values didn't line up with their controls (and it used the mono font).
	- Fix (folded into the Settings enhancement): `SettingsDialog::texts(line_h)` now centers every label / value / hex field / button against the actual rendered line height (the app passes `cell_h`), and the app draws them with the proportional `sans_attrs()`.
	- Verified: labels, sliders, values, swatches, hex fields, and buttons all align. (Also decided, not going with egui.)

- ✅ If the window isn't just the right hight, the last line of text is invisible. Not as in, below the visible line - but actually invisible. If you type, you can see that output happens, it's just not visible. Once it scrolls up even a single line though, it becomes visible. Adjust the hieght of the window just a tad, it "fixes" the problem. But at the default dimensions, the problem is apparent.
	- Cause: `Pane::build` lays out lines+1 rows into the pane's text buffer (the screen-row -1 overscan row above the viewport, plus rows 0..lines-1), so the bottom row sits at `y = lines*cell_h`. The buffer was sized to the content height (`lines*cell_h`), so when the window height made content an exact multiple of `cell_h` - which the default 152x48 does - that row landed right at the buffer's height limit and cosmic-text dropped it from layout (the cell bg/cursor quads use a separate renderer, so they still showed - hence "type and output happens but is invisible"). Scrolling/resizing shifted it back into range, "fixing" it.
	- Fix: size the pane buffer to `content_h + 2*cell_h` (overscan slack) in `spawn_pane`/`relayout`; `TextArea` bounds still clip drawing to the pane.
	- Verified: bottom prompt line renders at the default size.

- ✅ There are weird spacing issues with the cursor. It appears too far after text. There are also weird text background color interactions with `ble`, which I suspect is caused by the spacing issue.
	- Cause (re-fixed): the earlier two-part fix below was incomplete because `cell_w` was rounded (measured pitch ~10.5px -> 11). Everything grid-positioned (cursor, cell bg, per-cell glyphs) is placed at `col*cell_w`, so a `cell_w` that's bigger than the text's actual advance drifts them right of the text, and the drift accumulates per column. The cursor sat further past the text the longer the line, and fallback glyphs landed on top of the next cell at higher columns (`set_monospace_width` doesn't snap here. Cosmic-text only snaps when the font's `monospace_em_width()` is `Some`, which system fonts often aren't, so text renders at its natural advance).
	- Fix: the cell width now measures the real rendered pitch and is not rounded, so it matches the text and residual drift is sub-pixel. Per-cell fallback glyphs are fit to their cell box, scaled and centered so an over-wide fallback can't spill onto its neighbour. Verified: the cursor sits one cell after the prompt with no drift, and wide glyphs (CJK, emoji) occupy two cells without overlapping.
	- (Earlier partial fix, superseded by the above) 1) `set_monospace_width(cell_w)` in `TextCtx::new_buffer`; 2) pulling glyphs the primary mono face lacks out of the main buffer and drawing them per-cell. The extraction [2] is still in place; [1] is kept but is largely inert for system fonts.

- ✅ Opacity should only affect the text rendering area, the actual terminal. Instead, it is also affecting the entire window including window decorations.
	- Cause: the early build leaned on whole-window opacity, which by definition dims the decorations and text too. What's actually wanted is per-pixel surface alpha, and wgpu can't drive that on X11 directly (its Vulkan swapchain forces an opaque surface; its GL backend won't bind the ARGB visual).
	- Fix: Done - solved via the glutin + wgpu-hal GL-interop path (see "True transparency" below). Opacity now affects only the terminal background; text, decorations, and chrome stay opaque. The old whole-window opacity route was removed.

- ✅ Config file values don't work without a leading 0.
	- Cause: `.25` is invalid TOML, so the whole file failed to parse and every value reverted to default (hence "all values").
	- Fix: `config::lenient_floats` rewrites a bare-decimal value after `=` (`.25` -> `0.25`, `-.5` -> `-0.5`) before parsing.
	- Verified: `opacity = .25` now applies and other keys still load.

- ✅ The font size is still smaller than the system monospace size.
	- Causes:
		1. `config.toml` pinned `font_size = 15.0` (from an older template), overriding the new follow-the-system default.
			- Fix: Commented it out so detection applies.
		2. "Use system monospace" had only meant cosmic-text's generic `Family::Monospace`, not the OS-configured family, so even at matching point size the glyphs looked smaller/different.
			- Fix: Now `sysfont::monospace()` also returns the configured family (Pango/`defaults` parse, style+size stripped) and `resolve_mono_family` pins it when it's actually installed (validated against the font db), else falls back to generic monospace.
	- Verified: renders Monaspace Argon at 13pt (cell 11x21, window 1680x1016).

- ✅ Text sometimes renders in a different font (e.g. when running `source x9ps1-git; export X9PS1_STANDARD=1`). It seems that some color control codes causes the font change.
	- Cause: the prompt sets bold (`ESC[01;..m`), and cosmic-text's generic `Family::Monospace` resolves the best face per query, so a bold run landed in a different family than the regular run.
	- Fix: resolve the concrete monospace family name once at startup (`text::resolve_mono_family`) and pin `Family::Name` for every weight, so bold/italic stay in it.

- ✅ Text size is smaller than system default monospace.
	- Fix: Default font size now follows the OS's monospace/fixed-pitch size instead of a fixed 15px, via per-platform detection in `src/sysfont.rs`: Linux `gsettings` -> `fc-match`; macOS `defaults read -g NSFixedPitchFontSize`; Windows `SystemParametersInfo` message-font (windows-sys FFI). Points->px at 96 DPI. `font_size` in the config is commented out by default (follow system); set it to pin a size. Falls back to 17px when detection fails.
	- Verified: renders at the detected size on Linux, and the Windows cross-build compiles. The macOS path is not run-tested (no mac target).

- ✅ Native keybindings for `less` don't work.
	- Fix: `less` enables application-cursor-keys mode (DECCKM); arrow / Home / End are now encoded as `ESC O x` instead of `ESC [ x` when that mode is active. The mouse wheel also now drives full-screen apps: when the alternate screen / alternate-scroll mode is active it sends cursor-key presses instead of moving the (nonexistent) scrollback.

#### Done - new features and enhancements

- ✅ Settings dialog:
	- ✅ Remove "Settings" heading text, it's redundant with the window title.
		- Done: dropped the prominent in-dialog title (and its band); the tab bar now sits at the top. The OS window title still reads "Settings".
	- ✅ Change the buttons at the top for different pages, to tabs.
		- Done: the top selectors are a real tab bar (Appearance / Font / Colors / Window / Scrolling), the active tab highlighted.
		- ✅ Can cycle through with Ctrl+PgUp|PgDn.
			- Done: Ctrl+PageDown = next tab, Ctrl+PageUp = previous, alongside the existing Ctrl+Tab.

- ✅ For screenshots, and videos, use "Monaspace Argon NF Medium".
	- Done: `cicd/utility/screenshots.bash` font stack set to the Monaspace Argon NF family with fallbacks. Note: `font_family` selects a family, not a weight, so it renders at regular weight (true Medium would need a font-weight config). Videos will pick this up when that item is built.
	- Pending: regenerate the committed screenshot PNGs so they show the new font. Fold into the next visual regeneration and eyeball.

- ✅ Copy on... (20260713)
	- ✅ Update "[ ] Copy on output", to offer two options:
		- ✅ "Copy on   [ ] select   [ ] output"
			- Only one or the other
			- Done: menu bar now shows both checkboxes; turning one on turns the other off.
				- ✅ Vertically center text and checkboxes. Currently bottom-aligned. (20260713)
					- Done: the labels now center on their full ink, descenders included; the boxes were already centered.
		- ✅ Menu items too
			- Done: "Copy on select" / "Copy on output" toggles in the Edit menu and the right-click menu.
	- ✅ Implement "Copy on select"
		- Done: finishing a selection also puts it on the desktop clipboard (primary selection still set as always).
	- ✅ Improvements to copy on output:
		- ✅ Should only copy program stdout/stderr, and NOT the terminal prompt that resumes afterward.
			- Done: the input line was already excluded; multi-line prompts now handled too - the rows a prompt draws above its input line are recognized from the previous command and dropped from the copy. First command after enable can still include them (nothing learned yet); dynamic prompt rows that change every draw stay in the copy (fail-safe).
		- ✅ The checkbox button and menu item should only be visibly enabled for one pane at a time.
			- ✅ If you change tabs or panes, the feature gets turned off. (Visibly and actually.)
				- ✅ Changing to other non-SilkTerm windows is OK.
			- ✅ But if you later enable the feature on a different silkterm window, it gets disabled on other open windows. (Visibly and actually.)
				- Done: enabling notifies other running instances over the control socket; Linux/Unix only for now (same limit as the other socket commands).
		- ✅ Verify that it's not persisted across sessions. (I don't remember wiring this but who knows.)
			- Confirmed: no config key exists; the mode always starts off.

- ✅ New defaults: Background image opacity 10%. Background image blur, 10.

- ✅ CI/CD improvements:
	- Guiding constraints: rely on GitHub as little as possible (dumb git hosting plus optional release storage, nothing more), no cloud-hosted CI/CD, as few third-party tools as possible - but still cover the lightweight local-pipeline best practices for Rust.
	- ✅ Local merge gate instead of hosted CI
		- Add a fast `cicd.bash --gate` mode (fmt --check, clippy -D warnings, cargo test) and wire it as a git pre-push hook, so nothing reaches main unverified even outside a full cicd run.
		- This replaces what a bare-bones GitHub Actions workflow would do; the safety net runs on this box, not in the cloud.
		- The full pipeline (fuzz, packages, profiling, dogfood, publish) stays unchanged.
		- Done: `cicd.bash --gate` + `utility/git-hooks/pre-push` (gates pushes to main/dev only; `--no-verify` or `SKIP_GATE=1` bypasses).
	- ✅ Dev branch + release on main
		- Adopt a dev branch as the integration target. Feature branches merge to dev; main becomes release-only.
		- Merging dev to main cuts a release locally: tag the merge, run the packages stage, and optionally push the tag + attach artifacts to a GitHub Release as plain uploads (no Actions).
		- Version source is `Cargo.toml` alone: the tag is read from it and the build stamps from it, so they can never disagree.
		- Document the flow where branch conventions live, so day-to-day work knows the merge-back target changed.
		- Done: `dev` branch created and pushed; flow documented in design.md "Delivery"; `cicd/utility/release.bash` cuts the tag from `Cargo.toml` and can push + attach artifacts via `gh` (packages stage folds in when it lands).
	- ✅ Release packaging polish
		- Keep the hand-rolled packages stage (it already covers .deb/.rpm/NSIS across four targets, which cargo-dist does not) - no new packaging tool.
		- Add a sha256 checksums file next to the artifacts, and fold the release version into artifact names in one stable scheme, decided before the first tagged release so download links never have to change.
		- Done: scheme is `<exe>-<version>-<os-arch>[.exe]` + `<exe>-<version>-sha256sums.txt`, collected into `cicd/artifacts/release/` after the release builds. The future packages stage inherits the same scheme.
	- ✅ Pin toolchain and tool versions
		- Add `rust-toolchain.toml` pinning the rustc/clippy toolchain - this also kills the standing 1.94-vs-1.96 clippy split for good.
		- Pin the versions of cargo-installed helpers the pipeline probes for (cargo-deny, cargo-zigbuild, and any later additions) in one place cicd reads, so results stop drifting as the box updates.
		- No dependabot (GitHub-hosted): dependency freshness is a periodic local `cargo update` pass, with cargo-deny advisories already flagging anything urgent in every run.
		- Done: `rust-toolchain.toml` pins 1.96.0 + clippy/rustfmt + the three cross targets; helper pins live in `TOOL_PINS` in cicd/config.bash (non-gating drift warning).
	- ✅ README badges
		- Only the ones that carry signal without hosted CI: latest release tag, license, minimum Rust version. Static shields, one line at the top, matching the existing README style.
		- No CI badge - there is no hosted workflow to point it at, and a self-reported badge is noise.
		- Done: Release + minimum-Rust badges added to the existing badge block (license badge was already there). The release badge is static; release.bash refuses to tag until it matches Cargo.toml.

- ✅ Settings dialog:
	- ✅ Focus control:
		- ✅ When an item is focused, there shouldn't be a focus box the same size for every row, around the entire group of controls. The focus box should only go around the control being focused.
			- Done: the keyboard-focus ring now hugs just the focused control (checkbox / dropdown / text field / swatch+hex / whole radio group / slider) a couple px out, instead of spanning the row.
			- ✅ For slider controls, that should go first to the slider, then the related text box.
				- Done: a slider is now two Tab stops - the track first, then its numeric field - each ringed on its own.
			- "Reset" remains a focus-less control (the per-row revert icon stays mouse-only, unchanged).
	- ✅ Cursor scrim/outline:
		- ✅ Rather than two lines, just one, like so:
			Cursor    [ ] Scrim    [ ] Outline                [reset]
			- Done: the two "Cursor in scrim / outline" toggle rows collapsed into one `Cursor` row with two labelled checkboxes (each its own focus stop; Scrim greys with the scrim off, Outline with no outline).
		- ✅ The reset resets both of them (the row's revert icon reverts cursor_scrim + cursor_outline together).

- ✅ Use dropdown list boxes for Scrim function, and Scrim falloff.
	- Done: both are now dropdown list boxes (new `Dropdown` control in the Settings dialog) instead of radios - a collapsed box showing the current value + a down-arrow, opening a popup list on click / Space / Alt+Down. Keyboard: Up/Down move the highlight, Enter/Space pick, Esc closes, Left/Right nudge without opening. The popup draws in a second pass on top so covered rows can't bleed through it; it opens upward when it would spill past the panel bottom. The fuller labels the radios couldn't fit are back.
	- ✅ Order for Scrim function: SDF, DT, Dilate, Gaussian (default SDF).
	- ✅ Order for Scrim falloff: Exponential, Gaussian, Log, S-curve, Linear (default Gaussian).
		- Note: the default falloff changed from S-curve to Gaussian per this item (supersedes the earlier "default to S-curve").
	- ✅ Bug "Function selection not saving state": the apply path swaps the live settings (`RwLock<Arc<Settings>>`) and the diff writer persists `text_scrim_function`, so a picked function both takes effect live and is written to `config.toml` on Apply/OK - verified end to end.

- ✅ Improve the text scrim
	- Done: added a "Scrim function" choice (Dilate / SDF / DT / Gaussian [ugly]) and expanded "Scrim falloff" to five curves (S-curve / Gaussian / Linear / Logarithmic / Exponential), both in `config.toml` (`text_scrim_function`, `text_scrim_ramp`) and as Settings radios. The three non-Gaussian functions share one cheap separable Euclidean/Chebyshev distance transform (bounded to the halo radius, two passes, no jump-flood), so corners stay full instead of receding. Default is now SDF (round, full corners); Gaussian is kept as the labelled-ugly baseline. Falloff and function are orthogonal (shape vs fade). Verified all four render on the GL path and read as distinct backings.
	- Standard Gaussian Blur function is a poor fit for the text scrim, as a legibility aid. Here's why:
	- **What's wrong**: To illustrate conceptually: If you apply a background scrim to a solid square using gaussian blur, as the blur radius increases, the total blur shape looks more and more "round". This means that - effectively - the blur behind the square, doesn't look even at the corners. It looks "too strong" along the middle of the sides of the square, and "pulled-in" at the corners. The corners look naked. Basically it looks like a square sitting on top of a separate round fuzzy thing - rather than something evenly integrated with the square. (Which describes the cursor in block mode perfectly, and also why the scrim behind some clusters of letters looks "clumpy".)
	- **What would be better**: Ideally, the blur would also be square-ish - extending evenly from every angle, from every point along the edge of the square. (With corners rounding off with increasing blur radius, but never actually pulling in below the corners.) In other words, if you measured the density fall-off of the blur starting from the corner and moving outwart diagonally, it should fall-off at about the same rate, as if you measured it from the middle of an edge and moved out perpendicularly.
	- **Note**: "Gaussian" isn't just a blur function, it also describes blur falloff. (The Gaussian function makes the bell-shaped normal distribution, the falloff is half of one side.) So while the Gaussian *blur* function is probably the wrong blur to use, the *falloff* model is fine. Whether the two concepts can be separated in practice, is an open question for now, but seems doable (but also there's no reason for it to be a hard requirement - and isn't).
	- **Solutions ideas**:
		- **Distance field blur**. Aka signed distance field blur. This may be the closest match. Compute the signed distance from every pixel to the boundary of the shape, then apply a falloff function (Gaussian, linear, S, etc.) to that distance. Every point one pixel outside the shape has the same opacity regardless of whether it's beside an edge or outside a corner. The corners stay "full" instead of receding.
		- **Morphological dilation followed by feathering**. This might be the easiest and most practical to implement. Common in graphics applications. First expand the shape (using a square or other structuring element). In this case, each character individually on their center (and they'd grow into each other). Then feather the expanded edge - again with a falloff function. This also avoids the rounded-cloud appearance.
		- **Distance transform + transfer function**. Common in vector rendering and font rendering. Rather than convolving with a kernel, opacity is a function of distance from the boundary. I'm not really clear on how that works.
		- **All of them**: Rather than trying to decide which is best in a vaccuum, add an item to the config file (and a dropdown selection box in Settings) for "Scrim function", to choose among those three - plus the original "Gaussian [ugly]" (at the bottom). And as long as we're doing that, we might as well add a dropdown selection box for "Scrim falloff", including "S-curve, Gaussian, Linear, Logarithmic, Exponential".

- ✅ Rename "text outer glow" to "text scrim". And all syntactically same variants. In:
	- Source code
	- Config file
	- Settings dialog
	- README.md
	- design.md
	- Open bugs and issues in backlog.md, but not any below the "Done" section - need those for historical reference.
	- Done: config keys `text_glow*`/`cursor_glow` -> `text_scrim*`/`cursor_scrim` (value-preserving migration keeps existing configs); module/struct/idents `glow` -> `scrim`; Settings labels/enums; README, design.md; open backlog items; `03-glow.png` -> `03-scrim.png`. `text_outline` (a sibling, not the scrim) kept its name. (20260708)

- ✅ Options to include the cursor in the text scrim, and outline. Default scrim to off, outline to on.
	- Done: split the cursor coverage into its own texture, separate from the text, so `cursor_scrim` (halo) and `cursor_outline` (border) are independent. Two config keys + Settings toggles ("Cursor in scrim", "Cursor in outline"). Defaults: scrim off, outline on. (20260708)

- ✅ Donations model:
	- ✅ "Support SilkTerm!" button in Help|About, with flyover text of URL it's going to open in a web page.
		- Done: a filled button under the About text opens `DONATE_URL`. Hovering it shows the full destination URL, and the dialog is widened so it isn't clipped.
	- ✅ `## Support Silkterm` section in README.md
	- ✅ `DONATE.md`
	- ✅ `.github/FUNDING.yml`
	- ✅ Locked with `.github/CODEOWNERS`:
		- ✅ Help|About dialog
		- ✅ /.github/CODEOWNERS  @jim-collier
		- ✅ /DONATE.md  @jim-collier
		- ✅ /.github/FUNDING.yml  @jim-collier
	- ✅ Remove ssh signing keys model (for now).

- ✅ Cursor animation immediately resets and starts over on keypresses (typing, editing, or moving). That's not very smooth, it shouldn't do that.
	- Add options:
		- Keep animating.
		- Wait until the animation reaches full-size, then stop animating. Don't resume animating until some timeout after input stops, and then resume animating at the "top" of the cycle.
	- Done: `cursor_animation_input` config key, "continuous" (default) or "pause".
	- Fixed: the remaining snap in both modes. A keystroke slides the cursor to its new column, and during that slide it was drawn as a solid full block, overriding the animation - that was the instant jump to full, and the size popping back afterward was the double bounce. The animation now keeps running through the slide, so the size never jumps.
	- Fixed: "pause" resuming at the wrong size. At slow blink rates the run-out to full takes longer than the idle timeout, and the animation resumed from wherever it happened to be (small). Reworked: input lets the cycle run on at normal speed until it reaches full-size, holds there through the timeout, then resumes the cycle from full - continuous size at every step.
	- Note: "continuous" now never stops or resets for any reason; "pause" never jumps at entry, hold, or resume.
	- **Note**: Retrospectively, this was a HUGE pain in the arse. The bug where the cursor kept instantly snapping to the largest point in the animation cycle on any keypress, was really annoying and hard to fix. (I mean that's the opposite of "smooth", right? It was distracting.) Likewise, the bug where resuming the animation after pause, would catch it at a "random" point in the animation cycle, sometimes at the smallest point. Again, and instant warp from largest to smallest. And then worse was when both bugs conspired together on sporadic input, to cause a jarring "superbounce" effect.
		- But now it works as designed.

- ✅ Triple-click: Select the entire line - even if it's wrapped.
	- Done: a multi-click counter (single = run, double = word or pair, triple = line, a fourth wraps back), using the same timing window as double-click. Triple selects the whole logical line, including soft-wrapped continuation rows.
	- Verified: triple-clicking a line that wraps across two rows selects the full logical line; double still selects the word, single unchanged.

- ✅ Settings: "Backdrop blur" -> "Blur-behind"
	- Done: renamed the Settings toggle label; the internal key is unchanged.

- ✅ README screenshots, refreshed after significant visual changes: five anonymized shots (shell session, split panes, transparency + background image + glow, tabs / 24-bit / Unicode, Settings dialog) rendered at 1920x1080 and downsampled to 640x360 thumbnails.
	- Done: originals in `assets/screenshots/large/`, thumbnails in `assets/screenshots/`, shown as a grid in the README that links each thumbnail to its full-size image.
	- Note: the renderer (`cicd/utility/screenshots.bash`) runs in cicd before publish (skipped under `--quick`), so regenerated shots get committed with the visual change.

- ✅ Split pane auto-sizing logic: By default, when panes are split, if more than two are split in the same direction at a time, distribute their sizes equally. (E.g. All 50%, then all 33%, 25%, 20%, and so on.) But if the user breaks that trend by manually adjusting any of those, then from then on, every successive new pane splits 50% (until that sequence of same direction for pane splits stops - e.g. if the user starts splitting a different pane ancestry and/or in a different direction) Specifying pane % on the command-line also short-circuits the even-distribution logic, for that direction and ancestry.
	- Done: splitting in the same direction redistributes those panes to equal sizes (thirds, quarters, and so on).
	- Note: once you drag a divider in that run, further splits there stay 50/50 and your sizes are kept.
	- Note: a split in a different direction or ancestry is treated as its own run.
	- Note: command-line splits keep their explicit sizing.
	- Verified: unit tests cover equal thirds and quarters, the manual-drag case, and mixed directions.

- ✅ Option to copy all output (`stderr` and `stdout`) to desktop clipboard automatically. (For security reasons this may need to be an always-visible checkbox on the right-side of the main menu, as well as accessible from the right-click menu.)
	- Done: a per-pane toggle. When on, the focused pane's output copies to the clipboard as each command finishes.
	- Note: only the focused pane of the focused window ever copies, so a background window can't leak output.
	- Note: the text is plain printable Unicode, with colour and control codes removed. A command with no output leaves the clipboard alone.
	- Done: an always-visible "Copy output" checkbox on the menu bar, plus a toggle in the right-click and Edit menus.
	- Verified: instant, slow and multi-line commands all captured. The checkbox reflects and toggles the state.
	- Note: Unix only.

- ✅ Config:
	- ✅ "Glow border" -> "Text outline" (change description and config name). Change default value to 2.0.
		- Done: renamed the config key and the dialog label, and set the default to 2.0.
		- Note: existing configs migrate to the new key without losing their value.
	- ✅ Glow falloff: Change default to S-curve.
		- Done: the default falloff is now the S-curve.

- ✅ CICD dogfood section:
	- ✅ Copy as a different name every time, in format "slktrmdf_YYYYmmDD-HHMMSS"
		- So that multiple versions can run, and automated testing won't kill them.
		- Automatically delete existing older copies that are not in use.
		- Done: each build installs under its own timestamped name, so versions coexist.
		- Done: copies that aren't currently running are pruned automatically.
		- Done: two installs now - the old fixed name to the synced bin, and the rotating dated copy to ~/.local/bin. The preflight shows both.
		- Verified: a running copy is kept, an idle older one is removed, and the new copy appears.

- ✅ Create a new bash 5 script 'utility/n8runterm':
	- Can run any terminal along with script args it received (e.g. if user edits it), but by default it runs the function fSilkTermDogfood(), which:
		- Looks for the newest 'slktrmdf_YYYYmmDD-HHMMSS', and runs it with script args "$@".
		- Done: wrote the launcher. It finds the newest dogfood build and runs it, passing arguments through. Edit fMain() to launch a different terminal.
		- Verified: runs the newest build, passes args, and errors cleanly when none exists.
	- ✅ Also pass a random background image and a build-tagged title:
		- Done: prepends a random image from `~/.config/silkterm/backgrounds/` and a title tagged with the build's timestamp. Both go before the passed args, so a caller can still override.
		- Note: skipped quietly when the backgrounds folder has no images.
		- Verified.
	- ✅ Fall back to a known terminal when no dogfood build (or fMain's target) is found:
		- Done: tries terminator, xfce4-terminal, gnome-terminal, konsole, alacritty, kitty, then xterm, and runs the first one installed.
		- Note: prints a short note before falling back, and a real error only when nothing at all is installed.
		- Verified: selection order, the fallback note, and that a present dogfood build wins.

- ✅ Buttons:
	- ✅ Center text.
		- Done: the Cancel/Apply/OK captions are centered in the button. They were left-aligned before.
	- ✅ Provide click feedback.
		- Done: a button highlights while held and fires on release. Dragging off it first cancels.
		- Verified: unit tests, plus a check of the highlight and centering.

- ✅ CICD script: Don't prompt Y/N after prompting for commit message. User can just CTRL+C at that point if not wishing to contiue, and reduces friction for the most common path.
	- Done: removed the "Proceed? [y/N]" step. The commit-message prompt is now where you bail out, with Ctrl+C.
	- Note: `-y` still skips prompting entirely.

- ✅ Menu bar: (issue #t6thx, 20260626-132615)
	- ✅ Menu and Dialog background and text color user-adjustable, even per-theme. It's just that all themes by default should use the same menu colors.
		- Done: menu and dialog colours are part of each theme now, sharing the same neutral defaults across all themes.
		- Done: config keys let you override the menu and dialog colours.
		- Note: menu hover, border and separator shades follow the menu colour automatically.
		- Verified: a custom dialog colour recoloured the Settings panel. Unit-tested.

- ✅ Window title:
	- ✅ Updated requirement: Window title: Either use top-level `--title=`, or fallback to default, which is "SilkTerm - XYZ"; where 'XYZ' is the title of the current tab.
		- Done: a `--title` wins as-is. Otherwise the title is "SilkTerm - <current tab>".
		- Note: it tracks the focused tab's running program live.
		- Verified: the window name became "SilkTerm - dash".

- ✅ Automated testing: Test with HiDPI (simulated if necessary) to make sure menu text, tab title, Settings, and About still render OK.
	- Verified: at 2x the title, tabs, labels, sliders, fields, checkboxes and buttons all scale cleanly.
	- Reproduced: the Settings radio labels collided at 2x.
	- Cause: the radio spacing was a fixed pixel value while the text grew with the font.
	- Fix: radio spacing now scales with the font, and the panel widens so every option fits.
	- Verified: a unit test guards the scaling.

- ✅ Setting dialog (part 2):
	- ✅ A radio button for background image, to stretch or zoom. - New `Kind::Radio(&[..])` in the settings dialog (reusable N-option control: indicator box per option, fills the selected, click-to-pick); a "Bg image fit" row bound to `background_fit` (Stretch/Zoom). Verified: renders with Stretch selected by default; clicking Zoom switches it; `background_fit` persists + re-fits the image on Apply.
	- ✅ "Default shell": A command line to launch by default for new windows, tabs, and panes, if nothing else specified. Leave blank to use system default. - New "Shell" section in Settings with a "Default shell" text field bound to the existing `default_shell` config (empty shows "(system default)"; argv-split applies to new tabs/panes). Verified the field renders.
	- ✅ Size: A boolean setting to "Remember last size".
		- Done: remember_size config plus a dialog toggle. On launch it uses the remembered columns and rows. The pair updates on every manual window resize; startup and programmatic resizes are skipped so they don't clobber it. Columns and Rows grey out when on.
		- Verified: a manual resize persisted the remembered size, relaunch used it instead of the default, and the dialog shows the toggle checked with Columns and Rows greyed.
		- Overrides explicit numeric size.
		- Explicit numeric size fields disabled and grayed out.
		- "Remembered" values stored separately in config, so that user can uncheck the boolean and revert to previous numericly defined size. These "remembered" values are not exposed in the settings dialog, only exist in config file. Always update to last manual window resize, whether boolean is yes or no.
	- ✅ Should be able to use tab key to cycle among settings (and dialog buttons - in a loop). (20260702, branch kbdbtn) - the Tab ring now runs the active tab's focusable controls THEN the three footer buttons (Cancel/Apply/OK) and wraps, both directions (Shift+Tab / Up-Down too). A focused button shows the accent ring and is fired by Space or Enter. Built on the dlgkeys focus model (`Focus::Row | Focus::Button`). Unit-tested (walk controls -> Button 0/1/2 -> wrap; Space=Cancel / Enter=OK on a focused button).
	- ✅ A little more vertical space between the section headings, and the corresponding horizontal line. - Taller heading row (`HEADER_H` 34->42); the heading text is top-aligned and the rule sits near the bottom, leaving a clear ~7px gap (was overlapping). Verified in the dialog.

- ✅ Tab interface: single-window core done (`Tabs` in app.rs: each tab owns a `PaneManager`; tab bar shown with >1 tab, click to switch; pane area reduced by the bar). Detach/dock deferred (need multi-window). Verified: new tab, switch (content swaps), close (bar hides).
	- ✅ New tab (CTRL+Shift+T by default)
	- ✅ Change tab (CTRL+page up, down)
	- ✅ Move tab order (Shift+CTRL+Page up, down)

- ✅ Menu bar: (issue #t6thx, 20260626-132615)
	- ✅ Currently using "system sans serif", but if system proportional font is serif, the menu font is incorrect. - Fixed under bug #1n45bca: chrome pins a concrete sans family (`resolve_sans_family` / `sysfont::sans_serif`) instead of generic `Family::SansSerif`, which had been falling through to the serif document font.
	- ✅ Auto-adjust height based on menu font size.
		- Done (`app.rs`): the `MENU_BAR_H`/`TAB_BAR_H` consts are gone; bar heights now come from `menu_bar_h()`/`tab_bar_h()` = the menu font's line height (`text.cell_h`) + a small `MENU_BAR_VPAD`/`TAB_BAR_VPAD`, and the title text is centered in the scaled bar. So a larger font grows the bars instead of clipping. All ~13 const usages (layout, render, hit-testing, the resumed-time initial size) were switched. At the default font it's ~1px taller than before (27/29 vs 26/28) - imperceptible; verified it builds clean.
	- ✅ Make menu gray, with white text. (For both light and dark themes.)
		- The menu / tab-bar / context-menu chrome consts (`MENU_*`, `TAB_*`) are now neutral grays with near-white text, fixed across modes (per #166 default).

- ✅ Whenever a program update adds or changes config file settings, update the existing toml file in-place. E.g. reorganize, add/remove/rename items, but preserve existing active user settings and values that remain. (20260701; reorder 20260702, branch cfgorder)
	- ✅ `migrate_config` (runs before backfill on load): renames changed keys (value preserved), removes obsolete ones; `backfill_config` adds missing keys. Together: add/remove/rename + preserve, in-place, comments/layout kept.
		- Partially verified: a config with cursor_insert_shape/cursor_overwrite_shape/cursor_blink migrated correctly (and this auto-cleans the old invalid `cursor_blink = enable`).
	- ✅ Literal reordering to match template order (20260702, branch cfgorder). `reorder_config` (runs on load after migrate + backfill) rewrites an existing config into the template's canonical section order, preserving each setting's value + enabled/commented state while refreshing the section headers and explanatory comments from the current template. Keys the template no longer defines, and any user-added tables (`[themes.*]`), are carried through verbatim so nothing is lost. Pure + idempotent (`reorder_config_text`): a canonical file is never rewritten. Verified on a real drifted config (values incl. remembered_columns=187 preserved, re-parses as valid TOML) + 8 unit tests (order, value/state, unknown table + key, backfill-via-template, idempotency, full on-disk migrate->backfill->reorder pipeline).
		- ✅ Grouped the template into logical sections (Font, Window, Background and transparency, Text glow, Cursor, Selection, Shell, Scrolling, Theme and colours) with `##===`-ruled section headers and blank-line spacing.

- ✅ Settings dialog:
	- Done: all sub-items complete (last was full keyboard control).
	- ✅ Should be "modal" and connected to terminal window. (20260702, branch dlgmodal)
		- Done: the dialog is tied to the terminal window - X11 gets a transient-for hint, and Windows and macOS use the window-manager parent relationship. The window manager keeps it above the terminal and groups them. While a dialog is open the main window swallows keyboard, wheel, and IME input, and clicking it re-focuses the dialog. Applies to About too.
		- Verified: the transient-for hint points at the terminal window; typing at the terminal does nothing while open; clicking the terminal keeps the dialog active; after Esc, typing renders again.
	- ✅ As the number of settings may grow, we need a way to manage increasing length. Can't go beying about 1048 pixels high, including window decorations. (So roughly 1010 pixels total to be safe.) Implement both of these options: (20260626-102933)
		- ✅ Make the Settings window shrinkable and then add scrollbars only when necessary, so that it won't render beyond allowable space. By default, always try to open it normal size, unless constrained by display resolution.
			- Done: the window opens at its natural content size, capped to fit the monitor. When a tab still overflows (a huge UI font or short screen) the rows scroll, via wheel or a draggable thumb, and are clipped so they never paint over the title, tabs, or buttons.
			- Verified: unit-tested; no scrollbar appears when everything fits.
		- ✅ Group sections into logical "super-sections", and put them into tabs. A tabbled interface for settings.
			- Done: five tabs (Appearance, Font, Colors, Window incl. Shell, Scrolling), with measured tab widths and the active tab highlighted. The dialog now fits on screen; it was taller than 1080p.
			- Verified: every tab renders and switches, and a slider change plus Apply on a non-default tab persists.
	- ✅ Some more space between sections, so otherwise it seems run together.
		- Done: a second section on the same tab gets an extra gap above its heading.
	- ✅ Every setting in Settings dialog should have a clickable icon to "Revert to default". This icon (an emoji) should also indicate if the setting is default, and only be clickable if it's not. (20260626-102000; done 20260702, branch dlgrevert)
		- In the config file, if user clicks "Revert to default" in settings, set the value to default and comment it out.
		- Done: every control row has a right-edge revert glyph. It's accent-coloured and clickable when the value is off-default, dim and inert at default. Clicking it restores the default in the dialog, and colours revert to the active theme's value. On Apply, reverted keys are dropped from config and backfill restores the template's default line - commented for normal keys, active-at-default for the few template-active ones, so it looks like a fresh config.
		- Note: reverting Font size does not clear "Use system font" (unit-tested).
		- Verified: end-to-end.
	- ✅ "Use system font" boolean should be visible checked, if using it.
		- Done: already in place. Re-verified in the new Font tab - box checked, fields greyed.
		- ✅ If checked (setting a config boolean), the other font settings should be disabled. Whatever values they held, should remain.
			- Done: existing behavior - Font family and Font size grey out and keep their values. Re-verified.
		- ✅ Font family should default to a list with several fallbacks for Linux, Windows, and macOS.
			- Done: an existing default font stack (JetBrains Mono through Menlo, Consolas, monospace) shows in the greyed field.
	- ✅ Editable fields should have a visible cursor when focused, and respond to standard text-editing key controls. (20260702, branch dlgedit)
		- Done: the edit carries a caret. Typing inserts at it, Backspace and Delete remove around it, Home/End and arrows move it, and a thin caret line renders at the right spot in both hex and text fields.
		- Verified: typed and edited a value with the caret visibly tracking position.
		- Note: click still places the caret at the end; click-to-position is queued with the full-keyboard-control item.
	- ✅ Full keyboard control, e.g. tab order, full text field editing, alt+down for dropdowns, space to toggle booleans, etc. (20260702, branch dlgkeys)
		- Done: a keyboard-focus model over the whole dialog. Tab and Shift+Tab (and Up/Down) walk the controls on the active tab, wrapping and auto-scrolling into view, skipping headers and greyed-out rows. Ctrl+Tab cycles the tabs. Space flips a toggle or opens a field; arrows adjust a focused slider or radio and double as caret motion while editing. Clicking a field drops the caret at the nearest character to the click.
		- Verified: unit tests plus a focus-ring walk that correctly skips disabled rows.
		- Note: alt+down for dropdowns is N/A today - the dialog has no dropdowns yet; wire it up with the theme dropdown in Themes part 3.
	- Note: It might be best to defer some of these, until after (and if) native window controls are implimented.

- ✅ Window title: Just "SilkTerm", plus the icon in assets/logo.png (for display in alt+tab).
	- Done: `update_title` now sets the window title to just `APP_NAME` (per-program info stays in the tab titles). The window icon is loaded from `assets/logo.png` (`include_bytes!`, decoded + downscaled to 64x64 via the `image` crate) in `load_icon` and set with `with_window_icon`. Verified: window name = "SilkTerm", `_NET_WM_ICON` is set.

- ✅ The cursor [used to] render *behind* outer glow, which sometimes obscures the cursor. As noted in another issue below, the cursor itself should also have an outer glow, if not too computationally expensive with an animated cursor. In that case, the cursor shadow should merge with the text outer glow. And either way, the cursor should appear *above* any outer glow.
	- ✅ Cursor now renders ABOVE the glow. (20260701)
		- Done: cursor quads draw after the glow composite, under the crisp text.
		- Verified: a block cursor with a strong glow stays a crisp solid block.
	- ✅ Cursor's own glow (merged with the text glow). (20260701, branch glow2)
		- Done: the cursor draws into the glow source before the blur, so its halo is the text glow at no extra per-frame cost. The crisp cursor still draws on top. A cursor_glow config toggle, default on.
		- Verified: with cursor_glow off, only the cursor's own area changes.

- ✅ Outer glow enhancements:
	- Verified: all four, each showing its expected effect over a bright background.
	- ✅ When outer glow is applied, also add an antialiased (user-definable) 1px outer border around the letters, using the same color rules as outer glow.
		- Done: the composite also dilates the crisp coverage by text_glow_border px (antialiased), unioned with the halo and coloured by the same per-cell bg map. Config text_glow_border (default 1.0, 0 = off) plus a Glow border slider.
	- ✅ For bold text, calculate the blur for the outer glow, based on all non-bold text. (But still render the visible text on top in whatever weight it was meant to.
		- Done: the glow source has its own renderer. A pane containing bold shapes a parallel bold-stripped buffer and feeds that to the glow, while crisp text keeps its weight. Costs a second shape only on frames with bold. Config text_glow_regular_weight, default on.
		- Verified: turning it off changes only the area around bold runs.
	- ✅ Cursor should have blur if possible (investigate - this may not be possible, especially with the phasing).
		- Done: possible and done (see the cursor-glow item above). Phasing works because the animation alpha rides the quad colour, which blurs like glyph coverage.
	- ✅ Provide options for different blur fadeoff ramps. E.g. default gaussian, linear, or "S"-shaped.
		- Done: the blur falloff is selectable - text_glow_ramp of gaussian (default), linear, or s. A Glow falloff radio in Settings.

- ✅ Terminal should support standard terminal editing and/or navigation keys. (20260701)
	- ✅ Research: The only one I can think of that isn't currently supported, is Ctrl + arrow key (to skip whole words - other terminals do this).
		- Done: sends the xterm modified forms for Ctrl/Shift/Alt with arrows, Home, and End, so readline and TUIs word-skip as expected. F5-F12 were also missing entirely and were added, with modified variants. Unit tests pin the sequences.
	- ✅ Are Ctrl+Backspace, Ctrl+Del possible to delete whole words? Is that something some terminals do? XFCE terminal and Terminator don't.
		- Done: Both send now (xterm convention: Ctrl+Backspace = 0x08, Ctrl+Del = `ESC[3;5~`). Whether they delete a word is up to the app. Bash needs `bind '"\C-h": backward-kill-word'` / `'"\e[3;5~": kill-word'`, most modern TUIs handle them out of the box.

- Added `cicd/utility/gui-headless.bash`, a helper for running the terminal in an isolated GUI environment.
	- ✅ Update all tests, scripts, and profiling to run in that environment. (20260701)
		- Done: the profiler stage runs the app on the private display, so no window pops on the live session. It skips if the display, python3, or the workload are missing. Unit tests need no display anyway.
		- Verified: the app renders on the private display and the profiler produced a valid flamegraph.

- ✅ Cursor: (20260701)
	- ✅ After the related cursor bug fix above, set default cursor_size_horizontal to 25.
		- Done: with cursor_size_vertical at 100, this gives a 25%-width bar.
	- ✅ Default cursor_animation = "pulse_vertical"

- ✅ Settings dialog:
	- ✅ Alt+hotkeys for "Apply" and "OK", that underline when holding alt. (20260701)
		- Done: while Alt is held, Cancel/Apply/OK underline their first letter and Alt+C/A/O trigger them.
		- Verified: underlines render and Alt+C closes.
	- Font settings:
		- ✅ Add a sane set of fonts and fallbacks to the default "font family" setting, and make it an active setting in config. (20260701, decision #4)
			- Done: a use_system_font bool (default true) follows the OS monospace, overriding an always-active comma-separated font_family fallback stack (first installed wins) plus size. A pre-existing explicit font migrates to use_system_font=false.
			- Verified: the system font resolved, and the stack correctly skipped a missing first choice.
		- ✅ If using the system-defined font, enable the checbox and disable the related font adjustements (but don't clear their values). (20260701)
			- Done: the box opens checked when on the system font; Font family and Font size grey out but keep their values.
			- Verified: in the dialog.
			- User can un-check this later (or change the related config setting), to user the defined font settings instead.

- ✅ Cursor settings: (20260701, decisions #1-3)
	- ✅ size_vertical =  ## 1 to 100%, from left-to-right
		- Done: cursor_size_vertical is the cursor width % from the left, replacing cursor_shape. Bar 15, block and underline 100.
	- ✅ size_horizontal =  ## 1 to 100%, from bottom-up
		- Done: cursor_size_horizontal is the cursor height % from the bottom. Together with width they make any shape.
		- Verified: bar, block, and underline all render.
	- ✅ animation_style
		- Done: cursor_animation of none, phase, pulse_vertical, pulse_horizontal, or pulse_both, one cycle per blink_rate. Pulse grows from the cell centre, holds, shrinks, then disappears.
		- Verified: pulse_both grows, peaks, shrinks, and vanishes over about a second.
		- ✅ none
		- ✅ phase (the current default)
		- ✅ pulse_vertical
			- Starts with a single-pixel line in the middle, then animate up and down for full-height, pause there for a moment, then back and disappear momentarily, then start animation again.
			- Should happen in the same time as a cursor blink cycle. All animations happen in blink_rate.
		- ✅ pulse_horizontal (same idea as pulse vertical, but the animation goes left and right rather than up and down).
		- ✅ pulse_both (grow and shrink both vertically and horizontally)
	- ✅ blink_rate  ## ms
		- Done: cursor_blink_rate_ms, default 500. One animation cycle equals the rate.
	- ✅ Change default cursor colors: (20260701)
		- Done: SilkTerm dark foreground #88ffee, cursor #ff88aa.
		- Verified: cyan prompt, pink cursor.
		- Default SilkTerm theme (dark):
			- Foreground text color: 88ffee
			- Cursor: ff88aa

- ✅ Add an option to cicd: '--quick'. This excludes the slow processes like profiling and cross-platform building. (20260701)
	- Done: --quick disables cross-building and profiling (same as --no-cross --no-profile).

- ✅ Change the default hotkey for opening a new tab to Ctrl+Shift+T. (20260629)
	- Done: new-tab is Ctrl+Shift+T; plain Ctrl+T now passes through to the shell instead of opening a tab.

- ✅ Config file: resilient loading - one broken line must not drop every setting. (20260630)
	- Cause: a single TOML syntax error failed the whole document, so the entire config was ignored and everything reverted to default.
	- Fixed: blank the offending line and retry, dropping only the bad setting while the rest load.
	- Verified: unit-tested, and a bad line alongside columns/rows still sized the window.

- ✅ Config file: Preceed actual comments with double '## '. Commented-out *settings* get a single '# '. (20260629)
	- Done: DEFAULT_CONFIG template rewritten to the convention: explanatory + inline comments use `## `; disabled `# key = value` settings keep a single `# `. The parser already distinguished them (`line_setting_key` strips one `#`, so `## prose` yields no key), and toml_edit round-trips `##` fine. Two unit tests added (valid-TOML/deserialize + style check); 31 tests pass.
	- Note: only newly-generated configs and newly-backfilled keys get the new style; an existing config's already-present lines aren't reformatted (delete config.toml to regenerate the clean layout).

- ✅ New setting: Transparent background blur. (20260629)
	- This is independent of background *image* blur, which maintains its independence.
	- It blurs what's behind the terminal, as if it were made of frosted glass.
	- Done: compositor-provided. SilkTerm sets a stable WM_CLASS + a "Backdrop blur" toggle (KWin/picom hint); on Compiz, match `class=SilkTerm` in its own Blur plugin. Detail + Compiz recipe in the private dev notes.

- ✅ Change defaults: (20260629)
	- Done: Settings::default is the single source of truth, and the config template's example values now match. A guard test was added.
	- Note: glow is on by default now, so the glow pass runs every frame - confirm the look and feel by eye.
	- ✅ Background image blur: 8 px
	- ✅ text_glow = true
	- ✅ text_glow_radius = 5
	- ✅ text_glow_softness = 0.5

- ✅ Bell/warning: (20260629)
	- Gently and smoothly brighten all text, like the modern Windows Terminal does. - on BEL the text brightens toward white then fades back (~0.8s); text only (bg/cursor unchanged). Tunable `BELL_BRIGHTEN` if you want it stronger. Detail in the private dev notes.

- ✅ "Reload config" should re-read the background image too. In case user changed the image and kept it the same name. (20260626-102603)
	- Cause: `apply_new_settings` reloaded the image only when `bg_image_changed` (path/opacity/fit/blur differ). A same-name file swap leaves the path string identical, so it skipped the reload.
	- Fix: `apply_new_settings` takes a `force_bg` flag; `reload_config` passes `true` so Reload Config always re-reads the image file (the dialog Apply path still reloads only on a real change). `app.rs`.

- ✅ About dialog:
	- Include the version, build, copyright, and license.
	- Done (`dialog.rs` `layout_about`): added a copyright line and a `License: <SPDX>` line (`env!("CARGO_PKG_LICENSE")` -> GPL-2.0-or-later) under the version, and a `Build: <arch> / <os> (debug|release)` line in the Info section (`std::env::consts` + `cfg!(debug_assertions)`, so it names which cross-built target the binary is). The About window is content-sized, so it grows to fit. Builds clean.

- ✅ Menu (part 2):
	- ✅ When a menu is open, keyboard arrow should work on them, not on the active terminal pane.
		- Fix: An open menu (context menu or menu-bar dropdown) now captures navigation keys: Up/Down move a highlighted item (`ContextMenu::step`, wraps, skips separators, reuses the `hover` field/render), Enter activates it, Esc closes, Left/Right cycle between menu-bar dropdowns.
		- Verified: arrows highlight (separators skipped), Enter->New Tab opened a 2nd tab, Esc closed.
	- ✅ When 'Alt' Pressed, keyboard accelerators should become visible on the menu (traditionally with underscores). - Open dropdowns underline each item's first letter and a letter-press activates the first item starting with it (verified: 'n' -> New Tab). Alt+F/E/V/T/P/H open the bar menus. And now the bar titles themselves underline their accelerator letter while Alt is held.
		- ✅ Show the underline on the bar titles on Alt-hold (a redraw-on-Alt + char-measure pass). - Done (`app.rs` render): while `self.mods.alt_key()` and no dropdown is open, an underline rect is drawn under each top-level title's first letter (measured via `measure_text`, like the dropdown items); `ModifiersChanged` now sets `dirty` so it appears/disappears live on Alt press/release. Builds clean (cosmetic, to eyeball).
	- Note: the cross-platform-windowing-widget question (the `🚫` note under "Setting dialog (part 2)") is now decided - chrome stays hand-rolled (egui declined after a real spike). So the bar-title Alt underline is just a normal hand-rolled task.

- ✅ Change license from MIT to "GNU General Public License v2.0 or later", SPDX "GPL-2.0-or-later", reference https://spdx.org/licenses/GPL-2.0-or-later.html.
	- Status: Done. `license.md` now holds the canonical, verbatim GPL-2.0 text from gnu.org, in a markdown fenced block. `Cargo.toml`, `license = "GPL-2.0-or-later"`. README badge -> GPL v2+ and the license blurb updated; every `.rs` file (src + examples, 18) carries an `// SPDX-License-Identifier: GPL-2.0-or-later` + copyright header. Builds + 19 tests pass. The only remaining "MIT" string is in the README's commented-out badge palette, left intact.
	- The reason it was MIT before, was due to the misunderstanding that derived works have to also be MIT. But that's not the case, MIT allows relicensing derived works.
	- GNU General Public License v2.0 or later offers more protections, while being compatible with the Linux kernel and Darwin.
		- Also, some included libraries are Apache, which is compatible with GPLv3 (and therefore GPLv2+), but not bare GPLv2.

- ✅ Smooth-scroll enhancement: (20260626-100721)
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
	- ✅ Set default "Initial scroll speed" to 25.
		- Done: the default is now speed 25 on the 1..100 scale, in both the code default and the config template.
		- Verified: a fresh config and the dialog both show 25.

- ✅ Config file: Separate different grouped setting comments and settings (which are good to keep together), by an empty newline. Keep individual settings and comments together though. (20260625)
	- The `DEFAULT_CONFIG` template is now grouped consistently (each setting with its own comment; `line_height_scale` no longer rides the font-size group. The three background-image keys split into their own comment groups. `backfill_config` is group-aware: `setting_groups` tags whether each template setting starts a new group (preceded by a blank/table), so a re-inserted key carries its comment block and different groups are blank-separated, while same-group keys (e.g. columns+rows, the scroll-feel keys) stay together. A boundary double-blank is de-duped. Note: only affects freshly-written or newly-backfilled keys - an existing file's already-present bare keys aren't reformatted (regenerate for the clean layout).

- ✅ When double-clicking to select text, if the rule about quotes and brackets is in effect, and there are nothing but spaces in between selectable text and the matching quotes or brackets - then don't include the spaces in the selection. For example: " Now is the time. " - exclude the spaces between the symbols and the open and close quotes, in the selection. (20260625)
	- Status: Done. `pair_inside` now trims runs of spaces directly against the delimiters (interior spaces kept): `" Now is the time. "` selects `Now is the time.`, `[  hi  ]` selects `hi`. All-spaces inside falls back to the full inside span. Unit-tested (`pair_trims_adjacent_spaces`).

- ✅ Optimize compiled binaries to balance executable size and speed (slight nod to size), without the risk of triggering antivirus.
	- Status: Done. `[profile.release]`: `lto = "fat"` (whole-program inlining - smaller and usually faster than thin), `panic = "abort"` (drops unwinding tables - sizable shrink, fine for a GUI app), kept `codegen-units = 1` + `strip = true`, and opt-level stays 3 so renderer/PTY hot paths aren't slowed (the size improvement comes from the free wins, not from `opt-level=s/z`). Deliberately no UPX/packer - packers routinely trip AV heuristics. - Result: Linux binary is ~13% smaller, no runtime-speed tradeoff; verified still runs.

- ✅ Local CI/CD pipeline, one command, fail-fast, reusable across projects (`cicd/`). (20260628)
	- Expand the scope of existing `cicd.bash` copied from a sister project.
	- Solution:
		- One command (`cicd/cicd.bash`) runs the whole release end to end: format the code, debug build, run the tests, take a profiler snapshot, build all the release targets (native + cross), install the native build into a local bin dir ("dogfood"), then back up and publish to git. It prints the plan and the paths it will use first, and stops at the first problem.
		- Reusable in other projects: copy the `cicd/` directory and edit just `cicd/config.bash`. The engine itself stays generic.
		- Can run fully unattended with `-y` (give the publish commit message up front with `-m "..."`), so it formats, builds, tests, releases, and publishes without stopping to ask. Any stage can be skipped (`--no-fmt`, `--no-cross`, `--no-profile`, `--no-dogfood`, `--no-publish`).
		- The profiler stage is informational, not a pass/fail gate: it runs the real app under heavy load for a few seconds and saves a flamegraph - a single SVG you open in a browser to see where the time goes. It only aborts the run if the app itself misbehaves, not for environmental reasons like no display.
		- Old profiler snapshots and git backups are both trimmed to about 30 files by one shared routine, keeping a time-spread history: the most recent handful, plus the newest of each recent hour/day/week/month/year, plus the very first.
		- The fuller details (profiler tooling, the dedicated build profile, the rotation rules and tuning knobs) are documented in the `cicd/` scripts themselves.

- ✅ Background image:
	- ✅ By default unless overridden, look in ~/.config/silkterm/backgrounds/background.* - Status: Done. `resolve_bg_image` now auto-detects `backgrounds/background.{png|jpg|jpeg}` under the config dir (explicit `background_image` paths unchanged). Verified: image in that dir auto-loads.
	- ✅ Change default from "zoom" to "stretch".
		- Done: the default and template are now stretch.
		- Verified: an auto-detected image fills the window, ignoring aspect.
	- ✅ Add to background settings: Gaussian blur radius.
		- Done: a background_blur config (sigma in px, default 0) applied at image load, plus a Bg image blur slider in Settings.
		- Verified: the blur applies.
		- Note: the blur is in source-image space, before the fit - fine for a decorative low-opacity background. A true post-fit blur would need a 2-pass GPU blur (follow-up if wanted).
		- ✅ Results in pronounced color banding. Look into higher-quality blur filter, higher bit-depth for intermediate calculation, and/or dithering.
			- Cause. Mostly bit depth: the GL offscreen was 8-bit linear (`Rgba8Unorm`).
			- Fixes:
				1. Offscreen is now `Rgba16Float`, high-precision linear intermediate; the blit still does the single linear->sRGB encode into the 8-bit fbo 0.
				2. The blit adds TPDF dither (~1 LSB, per-pixel hash) before the 8-bit write, breaking residual banding scene-wide.
				3. The blur now runs in linear light (decode sRGB -> blur in f32 -> re-encode) so edges are gamma-correct.
			- Verified: the gradient is visibly smooth.

- ✅ Text readability glow:
	- ✅ When enabled, this setting adds some blurry background color, behind each glyph. In Photoshop, it's called "Outer Glow". - Done via `src/glow.rs` (`Glow`): the scene's text is rendered to a texture, blurred with a 2-pass separable Gaussian (`text_glow_radius` sigma), then composited (tinted the bg colour, `srgb_f32(bg)`) under the crisp text. Ping-pong f16 textures; intensity boost (`GLOW_INTENSITY=6`) so the blurred coverage is solid near glyphs. Gated `config.text_glow` (default off -> render path unchanged). Verified: light text on a light background is unreadable without it, clearly readable with it (dark halo). Implements exactly the suggested approach (render-bg-colour -> blur -> crisp on top), using the glyph alpha as the glow mask so no separate glow-coloured buffers are needed.
	- One possible way to do this - and there may be other, better ways:
		- Render the text exactly as normal, except in the background color. (As if background were 100% opaque.) On a fully transparent temporary canvas (at least conceptually - not necessarily literally).
		- Blur that rendered text with a gaussian blur, according to the specified blur radius in settings.
			- We may need to scale the radius value the user sees and adjusts, x*10, for cleaner integer values, then n/10 to use in code.
		- On top on that blurry background-color text, render the actual text in normal crisp text color.
	- The end result will be:
		- Even if the background is 0% opaque and effectively invisible, and the screen background is very light (like the terminal text color), the text will still be readable because it will have a dark (or background-colored) "glow" around it.
		- Even if the background is 100% opaque but the background image is very light (like the terminal text color), the text will still be readable - for the same reason.
	- ✅ Expose config value in settings dialog:
		- ✅ Blur radius: Boolean to enable, slider + number field to adjust.
			- "Text glow" toggle + "Glow radius" slider in Settings -> Appearance; the radius is greyed out/inert when the toggle is off (same `disabled()` mechanism as the Opacity slider). Verified in the dialog. (Editable numeric field is part of the deferred dialog-part-2 work.)
		- ✅ Softness/intensity control. Maybe "Softness" as the name. - New `text_glow_softness` (0..1, default 0.4) + a "Softness" slider in Settings (greyed when Text glow is off). Maps to the glow's coverage boost: 0 = hard/solid/strong halo (x10), 1 = soft/faint (x1). Verified: softness 0.1 = bold dark halo, 0.9 = gentle faint glow. (If the high=softer direction reads backwards, it's a one-line flip.)
	- ✅ Visual bug: When background glow is applied to characters that have a per-character(s)-box different background, and the foreground color is similar to the global background for that character(s), then the character is a blurry mess. (E.g. the global background is dark, but some characters are rendered one-off with dark text and light background, then it's not readable.)
		- ✅ The solution is, if a character has a different background color than global, use that one-off background color as the glow color for that character. - Done: the glow is now coloured by a per-pixel "bgcolor" texture (cleared to the global bg, with the per-cell bg rects drawn over it) instead of a single global tint; the composite multiplies the blurred glyph coverage by that local colour. So a glyph on a colored cell gets a halo matching its own cell bg (harmless), while global-bg cells keep their readability halo. Verified: dark text on a light cell over a dark global bg renders clean (no dark blur), global-bg text keeps its glow.

- ✅ Config file: When reading a value from the config file, if the entry doesn't exist, insert the setting into the file using hard-coded defaults, in an approprite section. (While not overwriting other existing values, comments, space formatting, etc.) Make this a reusable feature.
	- Status: Done. `config::backfill_config` (run in `load` after the file exists) inserts any setting the `DEFAULT_CONFIG` template defines that the user's file lacks, using the template's own line - so follow-system keys (font_size, font_family, background_*) stay commented (behavior unchanged) and active keys get their default value. Top-level keys go before the first table; `[colors]` keys under that header. Existing values/comments/formatting are preserved (insert-only). Reusable helpers: `setting_lines`/`line_table`/`line_setting_key`.
	- Verified: a partial config gets the missing keys (commented vs active per template), custom `opacity`/`foreground` preserved, re-run idempotent.

- ✅ When double-clicking to select stuff backwards and forwards to defined delimiters: Ignore delimiters if inside a consistent pair of single or double quotes, or paired (), [], <>, or {}. In those cases, select everything inside those (but not including).
	- Implied: `Pane::pair_span` + pure `pair_inside`/`distinct_pair`/`same_char_pair` (pane.rs, unit-tested). On a double-click the app first checks `pair_inside`; if the click is inside a matched pair it selects the contents (a `Simple` range), else falls back to the normal `Semantic` word select. Single-line only (multi-line pairs not handled).
	- ✅ But if the double-click happened outside such consisten parings, then ignore that logic (and the selection might include such characters depending on defined delimiters).
		- Falls back to `Semantic`.
	- ✅ The order of pair inclusion precedence: ``, "", '', {}, (), [], <>.
		- Done: the first enclosing pair in that order wins, so inside () selects the () contents even when [] is nested within.
		- Verified: precedence and quote-beats-paren tests.
	- ✅ List of delimiters should also be read from config file.
		- Status: Done. `word_separators` (config) feeds alacritty's `semantic_escape_chars`; backfilled if missing.
	- ✅ The list of selection inclusion pairs should be read from the config file.
		- Status: Done. new `selection_pairs` config key (default `` `` "" '' {} () [] <> ``), parsed by `config::selection_pairs()`; backfilled (commented) if missing. Not in the Settings dialog.

- ✅ Build targets, listed in order of importance: (20260626-091500)
	- ✅ Linux x86_64 (aka AMD64, but name everything referred to as "x86_64" for consumers/readers sake because "AMD64" is visually confusable with "ARM64").
		- Done. Native: `cargo build --release`. (Naming already consistent: no "AMD64" anywhere in code/docs/build config.)
	- ✅ Linux ARM64: `cargo zigbuild --release --target aarch64-unknown-linux-gnu` (cargo-zigbuild + zig 0.13). Built clean; binary is ELF aarch64.
	- ✅ Windows x86_64: `cargo build --release --target x86_64-pc-windows-gnu` (mingw). PE32+ x86-64.
	- ✅ Windows ARM64: `cargo zigbuild --release --target aarch64-pc-windows-gnullvm`. Built clean; PE32+ ARM64.
	- 🚫 macOS ARM64: Deferred. cross-compiling Linux->macOS needs Apple's SDK (osxcross), which is license-gated; do it on a Mac / in CI.
	- 🚫 macOS x86_64: Deferred. (Same; Mac/CI.)
	- Toolchain setup + commands are in `build.md`; one-time: install zig + `cargo install cargo-zigbuild` + `rustup target add aarch64-unknown-linux-gnu aarch64-pc-windows-gnullvm`. No ARM64 system libs needed (X11/EGL dlopen'd at runtime).

- ✅ True transparency:
	- Bug (fixed): Adjusting the transparency affects only the overall terminal background (including image which already has it's own correctly functioning opacity).
	- Transparency should not affect the Window decorations, menu, focus, or - critically - terminal text.
	- Status: Done. Opt-in `transparent_background = true`; `opacity` is the background alpha; text, decorations, and the menu/tab bars stay opaque. Verified on X11/Compiz/NVIDIA, decorated and borderless. Default (`false`) path unchanged (native wgpu).
	- How: wgpu can't get per-pixel alpha on X11 by itself (its Vulkan swapchain forces an opaque surface; its GL backend won't bind the 32-bit ARGB visual). So on X11 we create the window + a transparent GL context with glutin and run wgpu on top of it via hal interop (`Gfx::new_gl_transparent`), render the scene to an offscreen texture, then blit that into the GL framebuffer. Off X11 (e.g. Wayland) the plain wgpu surface already does premultiplied alpha. `Gfx` is a two-backend enum (native wgpu / GL). No wgpu downgrade, no renderer rewrite.
	- Note: the hard part was that on NVIDIA/Linux glyphon renders no text on a GL context below 4.2, because drawing into a texture view silently no-ops there (that is how glyphon builds its atlas). Fix: request a GL 4.6 context, falling back as low as 3.3.

- ✅ Make both the main menu, and the right-click menu appearances more traditional:
	- ✅ Use the system proportional font, rather than monospace font. - New `text::sans_attrs()` (cosmic-text `Family::SansSerif` -> the system default proportional font); the menu bar titles, dropdowns, and the right-click menu all use it.
	- 🚫 Use the system menu background and text color if reasonably feasible in a cross-platform way.
		- Canceled. There's no clean cross-platform API (Windows has `GetSysColor(COLOR_MENU/COLOR_MENUTEXT)`, but Linux/GTK needs CSS-theme parsing and macOS needs `NSColor`/objc). Kept the existing tasteful dark menu palette.
	- ✅ No indented items.
		- Done: All labels start at a common x after a fixed checkmark gutter (`MENU_GUTTER`); a `✓` is drawn in the gutter for active toggles, so checkable and plain items align.
	- ✅ Group items logically, and use faint horizontal lines and extra space to separate the logical groupings, as has been standard for menus since early Macintosh and Windows.
		- Done: Menu entries are now `Entry::Item`/`Entry::Sep`; separators render as a faint 1px line (`MENU_SEP`) with row spacing (`MENU_SEP_H`). Right-click groups: clipboard | read-only | tab/split/close | window toggles | config/settings. Verified.

- ✅ Format the "Help|About" widget better.
	- ✅ Use system proportional font.
		- Done. `text::sans_attrs()`, one buffer per line.
	- ✅ Add space between sections.
		- Done. `open_about` lays lines out with a section gap (`MENU_SEP_H`) before Info, the link, and the hint.
	- ✅ Put system info under an "Info" heading.
		- Done. "Info" heading with Renderer / Backend / Acceleration indented under it.
	- ✅ In addition to GPU info, note if using GPU acelleration or not.
		- Done. "Acceleration:" line from `adapter_info.device_type`: Hardware (discrete/integrated/virtual GPU) vs Software (CPU).
	- ✅ Add clickable github URL.
		- Done. Repo URL (from `CARGO_PKG_REPOSITORY`) drawn in the link color + underline; click within its rect runs `open_url` (xdg-open / open / start). Hit-rect verified; browser-launch not runtime-tested (would pop a browser).
	- ✅ Separate modal window rather than an embedded widget.
		- Done. About is now a real pop-out OS window sized to its content (`src/dialog.rs` `DialogWin::new_about`), via the new multi-window foundation (`App.dialog`, event-dispatch by `WindowId`, rendered in `about_to_wait`. Window creation signaled from `State` since it needs the event loop). Esc / window-close dismisses it. The repo link is clickable. The old in-surface overlay path is superseded; its dead code has now been removed (branch `rmoverlay`).
	- 🚫 Use the system window background and text color if reasonably feasible in a cross-platform way.
		- Canceled. Same as the menus: no clean cross-platform API. Kept the dark palette.

- ✅ Settings dialog:
	- ✅ Use the system proportional font.
		- Done. Dialog text now uses `text::sans_attrs()`, centered against the real line height (also fixed the misalignment bug above).
	- ✅ Allow selection of terminal background image (or none).
		- Done. A "Background image" text field (Kind::Text): type/paste a path; empty shows "(none)" and clears it. Live-edited (`reparse_edit` -> `background_image`), persisted by `config::persist` (sets the key, or removes it for none). Apply reloads the image. No native file picker in this stack, so a path field.
	- ✅ Allow setting font and size to "System default".
		- Done. A single "Use system font" checkbox (Kind::Toggle): when on it clears `font_family` and adopts the detected size live, and Apply removes `font_family`/`font_size` from config (`config::remove_keys`) so launches follow the OS; dragging the Font size slider turns it back off (explicit).
	- ✅ Make settings dialog a separate modal window rather than an embedded widget.
		- Done. Settings is now a pop-out OS window (`DialogWin::new_settings`, `Content::Settings(SettingsDialog)`), content-sized (~540x800) and non-resizable, so the whole dialog is visible regardless of the main window size (the requirement). Full interaction in-window: sliders (drag/click), text/hex fields (type), color swatches, Cancel/Apply/OK + Esc. Apply/OK live-apply to the main window via `App::apply_dialog_settings` -> `State::apply_settings_values` (config persist + rebuild). Verified: slider->Apply persisted `opacity` to config; OK closes; main survives. (The old in-surface overlay paths have now been removed in a dedicated cleanup, branch `rmoverlay`: `open_about_overlay`/`open_settings_overlay`/`apply_settings`/`handle_dlg_action`, the `AboutBox`/`AboutLine` structs, the `about`/`settings_dlg` fields, and all their render/event branches; ~278 lines. The live pop-out path and menu overlay are untouched.)
	- 🚫 Use the system window background and text color, if feasible in a cross-platform way.
		- Canceled. No portable API; same as the menus/About.

- ✅ Allow common menu accelerators (e.g. Alt+F for File menu).
	- Done: Alt+F/E/V/T/P/H open the matching top-level menu (first-letter match against `MENU_BAR`), when the menu bar is shown. note: this deliberately shadows the shell's Meta+<those letters> (e.g. Meta-f word-forward) - the standard menu-bar tradeoff (GNOME Terminal does the same).
	- Verified.

- ✅ Tab titles:
	- If a non-shell program is currently running, display: "shell [program]", where 'program' is the name of the running program.
	- If only the shell is running, display: shell [last: program]
		- 🔘 bug: If I run for example `ls`, The title isn't updated to "shell [last: ls]".
			- It seems to hinge on how long the command takes to execute. If the code is doing some kind of frequent sampling to get the program name, and if that could impact performance, then let's get rid of the " [last: <program>]" requirement and just show "shell". Otherwise if there is a more reliable alternate method to always know the last program that was run, that doesn't hurt performance (e.g. by requiring a watcher loop), let's try that.
	- Just the executable name for both, not the full command-line
	- Implemented:
		- Done: `TermInstance` captures the PTY master fd + shell pid at spawn (before the event loop takes the pty). `tab_title()` reads the foreground process group via `libc::tcgetpgrp(master_fd)` and its `/proc/<pid>/comm` (executable basename), comparing to the cached shell name: a different program -> "`<shell> [<program>]`" (and remembers it); only the shell -> "`<shell> [last: <program>]`" or just "`<shell>`". Polled when the tab bar is built (renders happen on output). Unix only (`#[cfg(unix)]`); other platforms fall back to the app name. New direct dep `libc` (unix).
		- Verified: `dash` -> `dash [sleep]` -> `dash [last: sleep]`. Tab titles also use the proportional font now.

- ✅ No hotkeys for pane management except. Minimal hotkeys overall, except for window, tab, menu, and clipboard managent.
	- Done. Removed the pane hotkeys: Ctrl+Shift+R/D (split V/H), Ctrl+Shift+W (close pane), Ctrl+Shift+Tab (cycle focus). Pane management is menu-only now (Panes menu / right-click; focus by clicking). `cycle_focus` deleted. Remaining hotkeys are window (F11), tab (Ctrl+Shift+T new, Ctrl+PageUp/Down change, +Shift move), menu (Alt accelerators, Menu key, Ctrl+,), clipboard (Ctrl+Shift+C/V).

- ✅ Changed mind about "close tab" hotkey: none. Use right-click or main menu, or just exit command.
	- Done. Removed the Ctrl+F4 close-tab hotkey. Close a tab via the Tabs menu ("Close Tab") or by exiting the shell.

- ✅ Menu keyboard key should activate right-click menu on active pane.
	- Done. The Menu/Apps key (`NamedKey::ContextMenu`) opens the context menu anchored near the focused pane's top-left. Verified.

- ✅ Group Settings items into logical sections.
	- Done. Added section headers (`Kind::Header`, bold + a faint rule): Appearance / Font / Window / Scrolling / Colors. `row_y`/height now sum per-row heights (headers are taller). Verified at runtime.

- ✅ Need a way to specify the font in the Settings dialog.
	- Done. "Font family" text field (empty = "(system default)"). Applies live: `MONO_FAMILY` changed from a write-once `OnceLock` to a re-settable `RwLock<Option<&'static str>>` (`pin_mono_family` re-resolves on each `TextCtx` rebuild and leaks the name for the `'static` `Attrs`), so the dialog's font family / "Use system font" take effect on Apply, not just restart. Persisted by `config::persist`. Also fixed: the spacebar is `Named(Space)` (not a Character), so font names / paths with spaces now type correctly into dialog fields. Verified: set "DejaVu Sans Mono" -> persisted with spaces, applied live, text renders.

- ✅ Add window dimensions to Settings dialog.
	- Done: Columns (20-400) and Rows (6-120) sliders in the new "Window" section. On Apply, if they changed, the window is resized to the new cell grid (`request_inner_size` from `cols*cell_w` / `rows*cell_h` + margins + menu bar). Persisted.
	- Verified: Columns slider -> window 1703->818px, `columns = 76` written.

- ✅ Make "Settings" title on dialog more prominent. (Bigger bolder font. Same with "About" dialog - but give it a title first.)
	- Done. `TextItem`/`AboutLine` gained `bold` + `scale`; the app applies `Weight::BOLD` and `TextArea.scale`. The "Settings" title is bold + 1.4x; the About box now leads with a bold + 1.5x "About SilkTerm" title (it had no real title before).
	- Verified.

- ✅ Menu content change: No tab or pane setting under the "File" menu. "Panes" can be it's own top-level menu item, between "Tabs" and "Help".
	- Done. Menu bar is now File / Edit / View / Tabs / Panes / Help. File = Reload Config, Settings..., Quit (no tab/pane items). Tabs = New/Next/Previous/Close Tab. Panes (new, between Tabs and Help) = Split Vertical, Split Horizontal, Close Pane (moved out of View). View = Fullscreen, Hide window frame, Menu bar. Verified at runtime: bar shows the six menus, File has only app-level items, Panes holds the split/close actions.

- ✅ Right-click menu options (with logical grouping):
	- ✅ Copy; selection -> CLIPBOARD
	- ✅ Paste; CLIPBOARD -> pane (bracketed-aware)
	- ✅ Paste selection; PRIMARY -> pane
	- ✅ Read-only (accept no input or interruption, but mouse selection and copy still work; toggle with checkmark)
	- ✅ New tab
		- Done. Right-click "New Tab" (`MenuAction::NewTab` -> `App::new_tab`); same as Ctrl+T.
	- ✅ Split vertical (already exists)
	- ✅ Split horizontal (already exists)
	- ✅ Hide menu (toggle with checkmark)
		- Done. View -> "Menu bar" (✓) and the right-click menu both toggle `menu_bar` (`MenuAction::ToggleMenuBar`); hidden = content to the top edge, re-show from the right-click menu.
	- ✅ Hide window frame (toggle with checkmark)
		- Done. `window.set_decorations`; verified frame extents 39px->0. Also the route to content-only transparency (bug 1).
	- 🚫 Hide scrollbar (toggle with checkmark)
		- Canceled. No scrollbar exists for smooth-scroll.
	- ✅ Fullscreen (toggle with checkmark)
		- Done. `window.set_fullscreen(Borderless)` + F11. Code path verified called; Compiz on this box doesn't honor the request (env, like the F11 grab), works on a compliant WM.
	- ✅ Settings
		- Done. Right-click "Settings..." opens the Settings dialog (`MenuAction::Settings`). Also Ctrl+,. Plus "Reload Config" to apply hand-edits.

- ✅ Some way to auto-apply settings after editing config file, without watching it. Maybe an internal command.
	- Done. Right-click menu -> "Reload Config" re-reads `config.toml` from disk and live-applies it (`config::reload_from_disk` + the shared `App::apply_new_settings`, the same rebuild path the Settings dialog uses: re-creates `TextCtx` + relayout on metric changes, reloads the bg image, re-sets window opacity). No file watcher; the file is the source so nothing is persisted back.
	- Verified.

- ✅ Change default columns = 160. Default margin = 8.
	- Done: `Settings::default()` and the `DEFAULT_CONFIG` template now use `columns = 160`, `margin = 8.0`. Existing config files keep their own values (defaults only seed fresh configs). Verified: fresh config -> window 1703x1024 (160 cols), content inset 8px.

- ✅ A window menu with typical menus items and actions (File, Edit, View, Tabs, Help)
	- Done. Menu bar across the top (`MENU_BAR_H`, shown by default; `area()` insets the pane area below it, stacked above the tab bar). Click a top-level title to open its dropdown; hovering another title with one open switches to it; click the title again or click away / Esc to dismiss. Dropdowns reuse the existing `ContextMenu` widget (`bar_menu_items(idx)` builds each; `open_bar_menu`). Contents: File (New/Close Tab, Close Pane, Reload Config, Settings..., Quit), Edit (Copy/Paste/Paste Selection, Read-only ✓), View (Split V/H, Fullscreen ✓, Hide window frame ✓, Menu bar ✓), Tabs (New/Next/Previous/Close Tab), Help (About...). Help->About opens the About dialog (originally a centered overlay, since reworked into a pop-out window - see the Help/About item). New `MenuAction`s: CloseTab, NextTab, PrevTab, ToggleMenuBar, About, Quit. Initial window height adds `MENU_BAR_H` so the default row count still fits.
	- Verified: bar renders, dropdowns open/switch, About shows "NVIDIA ... - Vulkan", Menu bar toggle hides the strip (content to the top edge).

- ✅ Render area shouldn't have a blue line (or any line) around it. When Window decorations are turned off, it should be background all the way to the last pixel of the edge.
	- The "blue line" was the focus ring drawn around the focused pane, which with a single pane traces the whole content edge. `App::render` now draws the ring only when the current tab has >1 pane (it exists to tell panes apart), so a single pane reaches the window edge with just background. Verified: single pane has no ring (only the cursor is bluish); after a split the ring returns around the focused pane.

- ✅ Add adjustable background image opacity to config file, and make default about 33%. This is independent of "see-through" opacity. The "opacity" should be relative to the background color. 0% = all background color, 100% = all image.
	- Done. `background_opacity` already provided this (0 = all bg color, 1 = all image); changed the default to 0.33. Independent of `opacity` (see-through).

- ✅ CTRL+shift+C and CTRL+shift+V should work as clipboard commands.
	- Done. Ctrl+Shift+C copies the focused pane selection to the CLIPBOARD; Ctrl+Shift+V pastes it (`handle_hotkey`). Verified at runtime.

- ✅ Double-clicking selects a word up to user-tweakable delimiters (sane defaults; full paths stay whole).
	- Done. Double-click (<400ms, same cell) starts an alacritty `SelectionType::Semantic`. New `word_separators` config sets the delimiters; default = alacritty's (keeps /.-_~ as word chars). Verified: double-click selected a whole path.

- ✅ Settings GUI dialog with organized main tunables, with primary buttons: Cancel, Apply, OK. Default=OK.
	- Done: `src/settings_ui.rs`: modal overlay (second pass, like the context menu) opened via Ctrl+, or right-click "Settings...". Sliders for opacity / bg-image opacity / font size / line height / margin / scroll-tau / wheel-lines, and swatch + hex field for the 4 colors. Cancel / Apply / OK (Enter=OK, Esc=Cancel). Live-apply: opacity re-sets window opacity, colors re-render, font/metrics rebuild the TextCtx + relayout; persisted in place via toml_edit (only changed keys, comments preserved, floats rounded). Foundation: `config::settings()` is now a swappable `Arc<Settings>` (`config::update`/`config::persist`). Verified: slider drag + Apply changed live opacity and persisted; hex typing recolored the swatch live; font-size change rebuilt text live without crashing. Not yet exposed (field table is trivially extensible): font_family, scrollback, alt/output scroll lines, background_fit, columns/rows, word_separators.

- ✅ If hardware acceleration is not available, use software rendering. Also need a way to tell which the application is using. Maybe in "help/about".
	- Done: `Gfx::new` requests a GPU adapter, then retries with `force_fallback_adapter` (a CPU/software adapter) if that fails. The renderer (name / backend / device-type) is logged at startup, and the Help/About dialog shows it (Renderer / Backend / Acceleration) from `adapter_info`. Verified: logs "NVIDIA GeForce RTX 3060 Ti [Vulkan / DiscreteGpu]".

- ✅ Make it easy to change the program name, in project and code files
	- Display name centralized in `APP_NAME` (`src/config.rs`); `utility/rename.bash NewName` rewrites the name + lowercase id across Cargo.toml, sources, and docs in one shot. Not a runtime/user setting.

- ✅ Local config file with tunables, somewhere under ~/.config
	- Done: `$XDG_CONFIG_HOME/silkterm/config.toml` (-> `~/.config/...`), auto-created with commented defaults on first run. Tunables: font, size, line height, margin, scrollback, scroll feel, colors (`#rrggbb`). Malformed/unknown entries fall back to defaults.

- ✅ Use system monospace font by default
	- Default font is the OS-configured monospace family (e.g. Monaspace Argon from GNOME settings) when it's installed, else cosmic-text's generic `Family::Monospace`. `font_family` in the config overrides it by name.

- ✅ Slightly More (and user-adjustable) margin between output and window border.
	- Done: `margin` config option (logical px, default 4), DPI-scaled, inset on all sides of each pane's content.

- ✅ Default to all black background, and 152 columns by 48 rows
	- Solution: Default `background` is now `#000000`. New `columns`/`rows` config options (default 152x48) size the initial window: after cell metrics are known the window requests `cols*cell_w + 2*margin` x `rows*cell_h + 2*margin` px, so `content_dims` floors to exactly the requested grid. Existing config files keep their own colors (defaults only apply to freshly generated configs).

- ✅ Some unicode glyphs don't render, most likely due to inadequate font coverage rather than a bug. Need fallback fonts just for glyphs that don't render, similar to how other terminals and text editors work. Don't need to expose fallback fonts as tunables (other terminals and text editors don't).
	- Solution: Switched pane text shaping from `Shaping::Basic` to `Shaping::Advanced`, which does per-glyph font fallback (CJK, emoji, math symbols, RTL) instead of rendering tofu, while keeping monospace alignment via cosmic-text's monospace-fallback path. Uses installed system fonts; no tunable. This was basic because an earlier cosmic-text 0.18 hung on real output here; 0.18.2's fallback loop is bounded and was stress-tested. Glyphs with no font on the system (e.g. powerline/nerd PUA with no nerd font installed) still fall back to whatever claims them - the user installs the relevant font, as with any terminal.

- ✅ Ability to select text by partial lines, with left mouse button.
	- Solution: Left-press maps the pixel to a grid `Point`+`Side` (`Pane::point_at`) and starts an alacritty `Selection::Simple`; drag extends it; release copies `selection_to_string()` to the X11 PRIMARY selection. Selected cells are highlighted (`config::SELECTION_BG`) via `SelectionRange::contains`. A click with no drag clears the selection.
	- Verified.

- ✅ Ability to select text with in a grid-aligned rectangle, with CTRL+left mouse button.
	- Solution:  Same path with `SelectionType::Block` when Ctrl is held at press. Verified: Ctrl+drag yields a rectangular block (cols 2-4 across 3 rows), not a span.

- ✅ Copy & paste selected text to current cursor location, via middle mouse button.
	- Solution: copy-on-select writes to the primary selection, held for the app's lifetime. Middle-click reads the primary selection and writes it to the pane under the cursor, wrapped in bracketed-paste when the app enabled it. Verified: a middle-click paste reached the shell.

- ✅ Use mouse to resize panes by grabbing on to separater line.
	- Solution: Each `Split` already carried a `ratio`; `divider_at` hit-tests the gap strip (+/-`DIVIDER_GRAB_PX`) and returns the split-tree path, `drag_divider` walks that path and sets the ratio from the cursor (clamped 0.05-0.95) then relayouts. Left-press on a divider starts a resize instead of a selection; hovering one shows a `ColResize`/`RowResize` cursor.
	- Verified.

- ✅ Ability to re-order panes with drag-n-drop mouse (possibly "grabbing" via shift-primary mouse button - and drop targets highlight themselves under mouse).
	- Solution: Shift+left-press grabs the pane under the cursor (Grabbing cursor); the pane currently under the cursor is tinted as the drop target (`config::DROP_TARGET`, alpha 0.3); releasing swaps the two leaves in the split-tree (`swap_panes` -> `swap_leaves` + relayout).
	- Verified: Shift+dragged left pane onto right, the two panes (distinct shell PIDs) swapped positions.

- ✅ Ability to make terminal area transparent (from 0-100% opaque). Ignore if compositing is not supported.
	- Solution: Tunable `opacity` (0..1, default 0.95) sets the terminal-background alpha (opt-in `transparent_background`). On X11 the per-pixel route (glutin + wgpu-hal GL interop) makes only the background translucent - text and chrome stay crisp and opaque. On Wayland the native wgpu surface already exposes premultiplied alpha. Without a compositor it's a no-op. Full detail in the "True transparency" item above.

- ✅ Ability to set an image as background, with adjustable visibility from 0-100%. That also works with transparency.
	- Solution: `src/bgimage.rs` ImageRenderer: a full-window textured quad drawn over the pane fill, under cells/text (premultiplied, composes with window opacity). `background_image` (absolute or filename relative to the config dir), or auto-detect background.{png,jpg,jpeg} in the config dir. `background_opacity` (visibility) and `background_fit`. `image` crate decodes png/jpeg.
	- Verified: auto-detected background.jpg renders; opacity 0.3 dims it.
	- ✅ Render options: Stretch-to-fit, Zoom-to-fit.
		- Done. `background_fit` = "stretch" | "zoom"; default zoom/cover.

### Future and/or deferred

- ✋ Feature: Minority Report mode: Borderless, transparent, changes perspective depending on screen location.

- ✋ Feature: (Git) Implement branch protection rules on main:
	- ✋ Require a pull request before merging (blocks direct pushes), and
	- ✋ Require review from Code Owners.
	- ✋ In more distant future: Do not allow bypassing / include administrators
		- Without this, I (as OG admin) can still merge around it, which is good early on.

- ✋ Bug: Modal Bug - About only (almost certainly a Compiz issue): with the About/Settings dialog open, selecting another window then re-selecting the dialog leaves the terminal buried behind whatever got in front, instead of both coming to the top together. Settings now works; About still does this on some Compiz desktops.
	- Almost certainly a Compiz WM issue, not a SilkTerm bug: About and Settings use the exact same dialog code path (window creation, transient-for + EWMH dialog/MODAL/SKIP_TASKBAR hints, and the raise-with-parent restack), so a difference between them is the WM's handling, not our code.
	- Note: the general case is fixed - the hints are set before the window maps, and since Compiz won't raise a transient's parent, the terminal is restacked under the dialog on focus and re-asserted briefly to outlast Compiz's animated settle. Verified on Compiz for both dialogs; the About-only failure couldn't be reproduced there.

- ✋ Bug: Alt-screen enter/exit animated like a scroll (`smooth_scroll_apps`). Two symptoms: (a) opening nano "jiggles"/jelly-bounces or scrolls in from a few lines down; (b) exiting nano scrolls the previous screen contents back in from the bottom, where a normal terminal just cuts.
	- Cause: an alt-screen enter/exit is an instant full-screen swap, but the scroll probes diffed frame-to-frame across it. On enter the app-scroll probe matched blank rows between the old and new screens -> bogus slide (jiggle). On exit `history_size` jumps (the alt grid carries no scrollback) -> the output-ease read it as new output and scrolled the restored screen in.
	- Fixed: track the previous frame's alt-screen state; on a transition hard-cut it - cancel any in-flight slide, skip both probes, suppress the output nudge, and rebaseline the row fingerprints to the new screen.
	- Verified: confirmed fixed (both symptoms). Residual: a very slight one-line smooth scroll-up still happens on enter and exit - livable, deferred (see the deferred item below).
	- Verified: mostly fixed. Entering and exiting still result in a one-line smooth scroll. Tolerable, but fix someday.

- ✋ Bug: Residual 1-line smooth scroll-up on alt-screen enter AND exit (`smooth_scroll_apps`). The enter/exit hard-cut fixed the big jiggle and scroll-in, but a slight single-line ease still rides the transition. Livable, deferred. Likely the output-ease firing one frame after the transition. A candidate fix is to rebaseline the history baseline and suppress the nudge one frame past the transition.

### Canceled

- 🚫 README screenshot refresh in cicd is off (`SHOTS_ENABLE=0` in `cicd/config.bash`; `--shots` re-enables per run). So the README grid images won't auto-update after visual changes
	- Moot point.

- 🚫 CTRL+right arrow should move to the beginning of the next word, not the end of the current. (CTRL+left arrow works as expected.)
	- And delimit on spaces (only?).
	- Closed: After research, not a terminal-side fix. Ctrl+Right already sends the standard `\x1b[1;5C`; whether the cursor lands on the end of the word or the start of the next is decided by the running line editor (bash/readline `forward-word` = word end; zsh = next word start), so the asymmetry with Ctrl+Left is inherent to readline, identical across terminals. Changing the emitted sequence would break the standard every app expects. Achievable per-user via a readline binding, or later via the deferred key-remap system.

- 🚫 CI/CD scripts:
	- 🚫 Build alternate targets in parallel, to speed process up.
		- Too fiddly. Possibly revisit in future. This lives in `cicd.bash`, which is pseudo-generic and could be made more so. Maybe it can shell out to a hyper-specific build script, or be updated to handle rust, go, and c++. Or more likely, it's just project-specifig, in spite of being originally [re]architected to call a settings script.

- Setting dialog (part 2):
	- 🚫 Adopt a cross-platform GUI / windowing widget toolkit (e.g. egui) for Settings, About, the main menu, and the context menu instead of hand-rolling them.
		- **No**. Results of spike (branch `spike/egui-dialog`): The upside is that egui 0.35 rides our exact wgpu 29 + winit 0.30 (no downgrade, shares our graphics stack) and integrated easily.
		- Drawbacks to egui: it adds ~32% to the release binary for what is secondary chrome, against the minimal-binary-size priority. Hand-rolling also keeps one unified colour/theme + native-OS-font system across the terminal and the chrome. egui would need a separate egui-`Visuals` theme kept in sync, plus its own bundled fonts).
		- Decision: Chrome stays hand-rolled.

- 🚫 Allow toggling from default "Insert" mode, to "Overwrite". (20260629)
	- 🚫 Change cursor in default "Insert" mode, to a thinner bar than the block cursor (but thicker than, say, "|").
	- 🚫 Overwrite mode will be the regular block cursor.
		- Overwrite mode canceled.
	- Backed out (20260630): overwrite mode + the Insert-key toggle removed (a terminal can't force the shell's line editor to overwrite). Kept the cursor work - configurable shape, blink, smooth slide. Insert key now just passes through to the shell.
	- Resolution: This can't be done without wonky hacks.

- 🚫 Terse `--layout` DSL as optional sugar over the window/tab/pane CLI model (not a replacement). One compact string for quick splits; lowers to the exact same internal layout the hierarchical flags produce, so it inherits per-pane targeting "for free."
	- Operators (mnemonic = the divider they draw): `|` side-by-side (vertical divider), `-` stacked (horizontal divider); `(...)` to nest (a group is uniform - mix directions by nesting); `;` separates tabs; `.` = one default pane.
	- Leaf = `.` (default shell) | command-alias name (from a `[commands]` config table, keeps the string quote-free) | `{raw command}` (opaque span so an inner `|` pipe isn't parsed as a split; `\}` escapes a brace). Optional fixed-order suffixes: `@dir` (cwd), `:weight` (size), `!` (keep-open).
	- Example: `silkterm --layout '(.|.)-. ; nvim|{git log} ; btop'` -> tab1: two-on-top/one-below; tab2: nvim beside a git-log pane; tab3: btop. Same string is accepted in `layout = "..."` in the config.
	- Trade-off vs the flags: far terser for hand-typed/quick layouts, but less self-documenting; the flags stay the canonical form (and what "Save layout" emits). DSL is purely a convenience front-end.

- 🚫 In `nano`, scrolling isn't smooth, it jumps line-by-line like traditional terminals. Is that just an artifact of the way `nano` specifically works?
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
