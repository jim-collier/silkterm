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

### Bugs

- 🔘 Bug in double-click to select (then Ctrl+shift+C).
	- Steps to reproduce: The specific command was `zpool status`. Trying to double-click on a member by label (e.g. "zfs-..."), or "ONLINE", results in something else being selected. It appears to actually select something to the right. But if you can guess correctly on your aim, then hit the copy hotkey, it does correctly copy the text. (Just not the text that's highlighted.)

- 🛠️ Smooth app-scroll (`smooth_scroll_apps`) left a blank band above/below the text that grew with scroll speed, and stepped one line at a time before easing. (20260703)
	- Cause: the slide shifted the scroll region by up to several lines but only the one fractional-overscan row was ever drawn, so the revealed strip was bare background - and the scrolled-off alt-screen lines are gone from the grid, so there was nothing real to fill it with.
	- Fixed: retained-frame slide. The pane keeps the previous frame's shaped text (`prev_buffer`, swapped in on each detected step) and draws it, clipped to just the revealed strip, so the strip fills with the real outgoing content while the current frame slides in over it. The per-step offset is now set (not stacked) since the retained frame is exactly one step back.
	- Verified: across continuous multi-line slides the content fills top-to-bottom with no blank band (mid-slide frames show partial top/bottom rows and unbroken numbering).
	- Verified:
		- Works perfectly in `less`.
		- `nano` exhibits none of the bugs listed above, but it also doesn't scroll smoothly, either with the mouse wheel or via cursor. (In fact, the mouse wheel just moves the cursor up and down. That's standard `nano` behavior, but the note is that scrolling isn't smooth. The cursor vertical movement also isn't smooth (horizontal is). Nano doesn't neeed to have a per-app fix, if it can even be "fixed".
	- 🛠️ muffer now scrolls smoothly on output - but still not mouse wheel.
		- Cause: a wheel notch makes the app repaint a bigger jump than line-by-line output, over the 8-line detection window, so it was seen as "not a clean scroll" and hard-cut. Raised the window/slide cap to 24 (experimental, gated by `smooth_scroll_apps`, revert = the one commit or turn the flag off).
		- Limit: the slide retains only the single previous frame, so fast wheeling can still lag ~one step (looks like snapping). Smoothing that fully needs retaining more frames - a bigger change. Feel-test the cap first.
	- 🛠️ Static-top-band fix (nano/muffer wheel = no change; less fine). Dogfood: the cap-24 bump didn't help nano or muffer on the wheel (muffer wheels 1 line/notch, well inside the window - so it was never a cap problem).
		- Cause: the detector matched a line-shift only as a run anchored at the TOP row, and the renderer slid the whole pane from its top. `less` fills from the top with only a bottom status line, so both worked. `nano`/`muffer` keep a static title bar at the TOP: its unchanging row 0 broke the top-anchored match (detector returned 0 -> hard cut), and even if detected the title would slide/bounce.
		- Fixed: `scroll_shift_signed` now finds the k where the most rows translate ANYWHERE (tolerates static bands at both ends), guarded by a "rows actually moved" count so a static/blank field can't false-trigger; `build` counts a static TOP band (mirror of the bottom one); the slide renders the scroll region between `top_split_y` and `split_y` with the top title bar redrawn unshifted. No-band apps (less/vim) are byte-identical (top_split_y = f32::MIN). App-scroll is alt-screen-only, so apt is unaffected.
		- Feel-test on hardware: nano + muffer wheel one notch should ease, not snap; the title bar must stay put (no bounce); less must be unchanged. Still gated by `smooth_scroll_apps`.
	- ✋ Residual band jitter during a slide (nano; "almost perfect" otherwise). Two symptoms, different causes:
		- Text moving UP (content scrolls up): the drop-shadow *under* the inverse-video header title jumps DOWN. STILL OPEN (real root cause now known + reproduced).
			- Partial fix landed (own-bg mask): the glow no longer applies over any cell with its own solid bg (reverse video, coloured bg, selection) - they have full contrast, so the halo there was pure artifact. bgcolor-map alpha is now an own-bg mask; the blur gates each tap by it. Removed the header's STATIC halo (A/B: below a reverse bar over a bright image the halo patch (216,216,225) -> (235,235,245) = the image, all other text glow byte-identical). Shared with the inverse-thin bug below. But this did NOT fix the owner's symptom - it's a MOTION artifact:
			- Real root cause (reproduced headless, nano-shaped scene over a bright bg image, frozen slide via huge scroll_tau_ms): the retained-frame slide fills the revealed strip from the PREVIOUS frame for the TEXT but does NOT glow that strip (app.rs glow pass skipped prev). So during a down-slide the rows in the strip just below the header lose their dark readability backing, and as the slide settles the backed/unbacked boundary marches down = "the shadow jumps down". Clearly visible: strip rows render faint over the bright image while settled rows below have the dark backing.
			- Fixed (glow the reveal strip): the glow pass now also glows the prev-frame strip (mirrors the text pass), so the revealed rows keep their readability backing and the boundary no longer sweeps. Guarded: only when the band on the strip's furniture side is detected (`has_top_band` for a top strip / `has_band` for a bottom strip) - that band clips the prev frame's header/status out of the glow. The first naive attempt (unconditional) blobbed the header when the top band wasn't detected (`st=0`): the open clip dragged prev's own-bg header in, mis-aligned with the current own-bg mask, glowing it dark. Instrumented the detector to prove it - `st=0` only in a DECSTBM-scroll scene; a full-redraw scene (how nano/curses actually paint) gives `st=1` and the header stays clean.
			- Verified headless A/B (redraw scene, `st=1`): header dark-px flat 1459-1580 (0 blobby frames), strip glowed; and the `st=0` DECSTBM edge case no longer blobs (max 3742 -> 574) thanks to the guard. Needs an owner feel-test on REAL nano to confirm the wheel/cursor feel.
		- Text moving down fast: the bottom two lines jump UP. Likely the SAME un-glowed-strip issue at the bottom edge (up-slide reveal strip above the status line), now also glowed by the same fix when `has_band`. If any residual jump remains after the feel-test, the leftover is band re-detection mid-ease (a new step re-captures band sizes + resets `app_off`); fix would be to hold band sizes stable across an in-progress ease.
		- Band freeze did NOT help (owner re-tested: "looks the same as before"). Instrumented `static_bands`/`app_off` against REAL nano for ground truth: the bands were already stable (frozen `(1,3)`, no fluctuation) - so band jitter was never the cause of this symptom. The real signal was `app_off` itself oscillating frame-to-frame (`0.57 -> 1.15 -> 0.37 -> 1.72 ...`) - THAT is the bounce.
		- Accumulation (first attempt) made it worse, not better (owner: "jump much farther, with less aggressive scrolling"). Accumulating `app_off` for the current content was right (content is measurably smooth), but ALSO accumulating `slide_sh` to fill a taller strip from ONE stale prev snapshot was wrong: when the cumulative shift outgrew the scroll region, prev was re-captured and slide_sh reset, jumping the reveal strip (and its glow) by the WHOLE scroll-region height (~28 cells) - a periodic ~screenful jump that scales with accumulation depth = the "farther" bounce.
		- Real fix (recapture every step + lag ramp): keep `app_off` accumulating (smooth content), but RE-SNAPSHOT prev every step so the strip is always one fresh step back (no more 28-cell re-capture jump). One retained frame only fills a one-step strip, so a fast burst could still lag far enough to open a blank band; a lag ramp on the `app_off` ease (scroll.rs) fixes that - it eases at the smooth configured speed while the lag is under ~1 line (gentle scroll unchanged) and ramps the ease faster as the lag grows, bounding `app_off` to ~1 (gentle) / ~3 (fast) instead of running to ~7. Built a headless bounce harness (deterministic full-redraw nano-shaped scene + `SILK_SCROLLDBG` per-frame trace + a monotonicity analyzer) - measured content bounce 0, band-boundary jumps 0, prev-strip jump <1.6 cells, across gentle/fast/wheel with real top+bottom bands; mid-slide dumps show the blank band shrank from ~7 lines to ~1. But the residual on real nano over the owner's bg image was still visible ("only as bad as the previous-to-last round").
		- Deferred: Title-bar apps hard-cut for now (`SLIDE_TOP_BAND_APPS = false` in pane.rs): the smooth slide only engages when there's NO static top band (`st == 0`), so `less` still slides (verified) and nano/muffer just page-redraw, as before the top-band work - no slide means no bounce. The enter/exit hard-cut fixes (no jiggle on launch, no scroll-in on exit) are untouched (they live in the `alt_transition` path). Flip the const to re-enable and resume the bounce work; the proper next step there is N-frame retention so the reveal strip always fills regardless of lag. Verified headless: nano scene (st=1) detects the shift but `app_off` stays 0 (hard cut); less scene (st=0) still eases, bounce 0.

- 🔘 Choosing "Tabs|New Tab" the first time, opens a second tab. Doing it again, changes to the first tab, rather than opening a third tab.

- 🔘 When switching fonts then hitting "OK", the font changes but not the blur. An exit and reload is required to sync them up.
	- Investigated: no obvious code-path desync found. `bg_image_changed` already includes `background_blur`, `needs_text_rebuild` covers font, `load_bg_image` re-reads the fresh blur, and the glow re-shapes each build. Needs a live repro (change font in Settings, OK, watch the blur) to pin which "blur" and the exact trigger - deferred to an interactive pass since dialog driving is flaky here.

- 🔘 At high blur radius and low softness, the blur has boxy artifacts.
	- Diagnosed: the glow is a separable blur with a fixed 25-tap kernel truncated at +/-3 sigma (`glow.rs` fs_blur). Two causes: (a) the hard +/-3 sigma cutoff leaves a ~1% edge that low softness (x10 intensity) amplifies into a visible square; (b) the linear/s-curve falloffs aren't true Gaussians, so blurred separably their support is a diamond/box, not a circle - and the default falloff is now s-curve. Fix is a look-vs-perf tradeoff (wider extent + more taps, and/or a windowed kernel) that wants eyeballing - deferred to a visual pass.

- 🛠️ Terminal is sometimes completely black after coming back from a long session. It responds to input, it just can't be seen - all the input and output is black. In some cases, the cursor, and cells with individually-colored backgrounds, are visible. (20260630)
	- Cause: when the glyph atlas fills up during a long, varied session, text prepare() fails and render bailed out before the per-frame atlas trim. The atlas never recovered, so text stayed black. Cursor and per-cell backgrounds use a separate renderer, so they kept showing.
	- Fixed: trim the atlas on the prepare-failure path, so the next frame re-prepares with room and recovers.
	- Note: couldn't force an atlas-full for a live repro. A 20s flood didn't fill it; the trigger needs a genuinely long session.
	- Verified: a 50s max-rate unicode flood stayed visible throughout, app alive, no black-out. This box lacks the CJK/emoji coverage to actually fill the atlas, so the exact trigger is still unreproduced here.
	- Resolution: leave open until confirmed on long-running terminals.

### New features and enhancements

- 🛠️ Donations model:
	- ✅ "Support SilkTerm!" button in Help|About, with flyover text of URL it's going to open in a web page.
		- Filled button under the About text, opens `DONATE_URL` (DONATE.md); hovering it flies over the full destination URL. Widens the dialog so the URL isn't clipped.
	- ✅ `## Support Silkterm` section in README.md
	- ✅ `DONATE.md`
	- ✅ `.github/FUNDING.yml`
	- ✅ Locked with `.github/CODEOWNERS`:
		- ✅ Help|About dialog
		- ✅ /.github/CODEOWNERS  @jim-collier
		- ✅ /DONATE.md  @jim-collier
		- ✅ /.github/FUNDING.yml  @jim-collier
	- ✅ Remove ssh signing keys model (for now).
	- FYI to-do:
		- Enable a GitHub Sponsors profile for the Sponsor badge/link to go live (else it 404s)
		- fill in `.github/FUNDING.yml` handles.

- 🔘 Settings: "Backdrop blur" -> "Blur-behind"

- 🔘 For screenshots, and videos, use "Monaspace Argon NF Medium".

- 🔘 CICD process (without --quick): Only after quite major changes, record a demo video:
	- 🔘 Showing a wide variety of synthetic, anonymized content, with varying burst of text output length.
	- 🔘 Colorized (anonymous) prompt, colorized ls output, etc.
	- 🔘 Include showing smooth-scrolling in `nano` and `less`.
	- 🔘 Typing commands etc. should look as if a real human were doing it - a variable ~40 to 100 wpm (avg about 80), with common random mistakes and fixes.
	- 🔘 Include perfectly-matched keyboard click sounds, that vary realistically "random", except for the same sounds for space, enter, backpsace, etc.
		- 🔘 Very nice, ASMR-like, luxurious "thocky" keyboard sounds.
	- 🔘 Mouse click sound for demoing mouse features. (Quiet, nice mouseclick sound, deeper/softer than typical demo mouseclicks.)
	- 🔘 ~1024x768 terminal area. (Which we might change later.)
	- 🔘 Adjust some headline features through the Settings dialog.
	- 🔘 Show a readable banner on the video, briefly naming or describing what is being demo'd (and leave it up for a minumum human-readable time, or longer). Make sure the banner isn't blocking what's being demo'd.
	- 🔘 Use a codec that compresses this kind of content well.
	- By default use "background24.jpg" at 10% opacity. No terminal transparency except when demoing it. Background color #222222. Font current system font ("Monaspace Argon NF Medium").
	- 🔘 Store the videos under `silkterm/private/demo-video/`, using the same naming/rotation strategy as backups.
	- 🔘 Store a copy of only the most recent video probably under `siklterm/github/source/video/demo.{ext}`. Make sure README.md embeds or references it visibly near the top.

- 🔘 Triple-click: Select the entire line - even if it's wrapped.

- 🔘 Ctrl+Shift+N: New window on same directory.

- 🔘 Tabs: Include a subtle 'X' icon in right edge of tab, to close with mouse.

- 🔘 Blur: Naturally doesn't extend diagonally very far. When blurring a rectangle, for example, this is a known effect of, say, Gaussian blur. So either use a different blur that covers the diagonal directions better, or tweak the blur kernel so that it does that.

- 🔘 Cursor animation immediately resets and starts over on keypresses (typing, editing, or moving). That's not very smooth, it shouldn't do that. Add options:
		- Keep animating.
		- Wait until the animation reaches full-size, then stop animating. Don't resume animating until some timeout after input stops - maybe 1 second. (Default.)

- 🔘 Option to include the cursor in outer-glow. Default to off. Still outline it though.

- 🔘 Smooth cursor movement should speed up, if it falls too far behind where it actually is.

- 🔘 Copy output:
	- 🔘 Should only copy program stdout/stderr, and NOT the terminal prompt that resumes afterward.
	- 🔘 The checkbox button and menu item should only be visibly enabled for one pane at a time.
		- 🔘 If you change tabs or panes, the feature gets turned off. (Visibly and actually.)
			- 🔘 Changing windows is OK.
		- 🔘 If you enable the feature on another silkterm window, it gets disabled on other open windows. (Visibly and actually.)

- 🔘 Settings dialog:
	- 🔘 Remove "Settings" heading text, it's redundant with the window title.
	- 🔘 Change the buttons at the top for different pages, to tabs.
		- 🔘 Can cycle through with Ctrl+PgUp|PgDn.

- After startup and enough time to settle down, auto-detect shells in the background. Dynamically pre-populate (or verify) the list of available shells, with user-friendly names. Bash, Dash, Ash, ZSH, PowerShell, Cmd, WSL2 Debian, Fish, PyCmd, YSH, Korn - do a web search for other common shells that might be installed.

- 🔘 Text fields in Settings dialog need to support standard editing functions. (Right-click, editing hotkeys, etc.)

- 🔘 Main menu and right-click menus:
	- 🔘 Accellerators need to be unique. If running out of memorable word/accelerator keys, remove accellerators from the least-used or least-important items, especially ones that already have hotkeys.
	- 🔘 List the hotkeys to activate the same function, if they exist. Keep in mind there might be a dynamic hotkey system soon.

- 🔘 Hyperlinks:
	- 🔘 Clickable - e.g. Ctrl+click, or right-click then includes "Copy link" and "Open link".
	- 🔘 Auto-underline when mouse is underneath.

- 🔘 New defaults: Background image opacity 10%. Background image blur, 10.

- 🔘 New setting: Background image contrast mask % (100% = half of the longest pixel dimension, 0% = none, auto=based on contrast frequency analysis.)

- 🔘 When reducing background image opacity, also reduce contrast and saturation. Add tunable parameters to the config file:
	- 🔘 Minimum contrast % (at 0% background image opacity - not useful but establishes the floor). Lets try a default of 50%.
	- 🔘 Maximum contrast % (at 100% background image opacity). Default 50%.
	- 🔘 Similar settings and defaults for saturation.

- 🔘 Change wording of "background image opacity" to "background image visibility" (text and setting), to reflect that it's not just opacity. Still directly controls image/background color mix, but ALSO the contrast and saturation.

- 🔘 Need a way to detect maximum and average brightness of background image - or some human hueristic of "perceived brightness", and apply a variable ramp to background image visibility, so that it gets darker quicker, as the % goes down.
	- 🔘 Really what I'm after, is this resulting effect. The implimentation is up to research:
		- 🔘 At 100% background image visibility, it's just the image as-is.
		- 🔘 But below that, the opacity % scales with human perception.
			- 🔘 In other words, at say 90%, it is actually scaled to some average of ([perceived brightness], [brightest pixel]).
			- 🔘 As an example, 50% for a very bright image, may be significantly darker than 50% for a very dark image.
		- 🔘 And the inverse, for light-mode themes.
		- 🔘 Need a config file name and a default value for the resulting strength of this calculation.

- 🔘 Option to rotate background images from a folder; in order, or randomly. At startup, or on a timer.

- 🔘 Testing:
	- 🔘 Also try menus and dialogs with 125% larger font than current - independent of existing HiDPI tests.
	- 🛠️ Do full regression testing (and try to keep the tests updated as new features and bugs are added), and against library code as well.
		- Scrolling covered: library tests (`cargo test`) encode the per-app matrix (less/vim slide, nano/muffer hard-cut) plus normal-output invariants (add-a-line vs re-list/jump/bottom-up) and easing monotonicity; a headless harness (`cicd/tests/scroll`) drives deterministic full-redraw scenes off the `SILK_SCROLLDBG` trace and runs in cicd stage 3 (skipped under `--quick`). Still to broaden: other features, and fuzz/security below.
	- 🔘 Add fuzz and security testing suites. Not just for SilkTerm code, but against library code too, so that we can find and patch critical bugs there too.

- 🔘 Build packages when cicd.bash `--quick` isn't specified:
	- 🔘 .deb(s), per-architecture
	- 🔘 Windows installer .exe(s), per-architecture
	- Future:
		- macOS appimage, per-architecture

- 🔘 Scroll-on-output enhancement: One additional setting: (20260629)
	- 🔘 In-view fast output scroll speed. (E.g. for a short directory listing that doesn't exceed a single pane height.)
		- Faster than initial scroll speed, but ramps up slower, and top speed is slower than current.
	- 🔘 Once the top line of new output scrolls above and off the screen, then scroll speed ramps up as fast as necessary to fully keep up.

- 🔘 After startup, auto-detect shells in the background. Dynamically pre-populate (or verify) the list of available shells, with user-friendly names. Bash, Dash, Ash, ZSH, PowerShell, Cmd, WSL2 Debian, Fish, PyCmd, YSH, Korn - do a web search for other common shells that might be installed.

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

- ✅ Inverted text (e.g. Nano headers) is thin and hard-to-read.
	- This turned out to be the owner's ACTUAL nano complaint (the "shadow jump" language was describing this). Dark-on-light (reverse video) renders visually thinner than the same-weight light-on-dark - inherent irradiation + AA, and SilkTerm/xfce-terminal both show it (Terminator renders it bolder). The glow only boosts light-on-dark text (dark halo), so inverse text got no readability help; the earlier own-bg mask removed the eroding bright halo but added no boost.
	- Fix (embolden): new `embolden_inverse` config bool (default true) - reverse-video runs render at Weight::BOLD so they read as strongly as normal text. `pane.rs` build() ORs INVERSE into the run bold flag. Verified headless: inverse text is visibly thicker with it on (bold face applies; the delta is modest with the default DejaVu mono - a font with a heavier bold shows more). Needs owner feel-test; if too subtle, next step is faux-bold (stroke dilation) for stronger control.

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
	- Verified: runtime - launched with a 2nd (active) tab whose shell self-exits after 3s; app stayed alive past the exit (tab closed, window survived) instead of quitting. Builds clean.
	- Re-verified fixed on current main (20260630): the app survives the tab's shell exiting in all 3 tests - CLI active-tab exit, CLI background-tab exit, and typing `exit` interactively in the active tab of a 2-tab window (1 window stays up). If it's still happening for you, the running/dogfood binary predates the fix - rebuild (`cargo run --release`) or re-run cicd to reinstall, then retest.
		- ✅ Still not fixed. With three tabs open, for example:
			- Type "exit" in the anything but the last tab, it closes ALL tabs, except for one. Sometimes, the program becomes unresponsive then and has to be killed.
			- Type "exit" in the last tab, it closes the program.
			- With four tabs open, and type "exit" from the third, closes the first two tabs (and not the third).
		- ✅ REAL cause (20260630): pane ids collided across tabs. Each tab is a separate PaneManager that assigned ids from its own counter (first pane always id 1), so the shell-exit event (carries only the id) resolved to the WRONG tab - the first one with that id - and closed it; dropping that tab's term fired another Exit -> cascade (closed all but one, sometimes hung), exactly as reported. The earlier fix (find the owning tab + cascade) was right in shape but the id lookup was ambiguous. Fix: `alloc_pane_id()` - one global counter, so every pane is unique everywhere. Verified with instrumentation: exit in the 3rd of 4 tabs resolves to tab index 2, exactly one close, no cascade, app alive.

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
	- Cause: with always-GL on X11 the main window holds a glutin GL/EGL context, and the pop-out dialog's `Gfx::new` created a second `wgpu::Instance::default()` (all backends, including GL); wgpu-hal's GL init then panicked in EGL teardown (`unmake_current().unwrap()`, "Another window API already has a current context"). Increment A/B tests used a native-Vulkan main (default config), which masked it; the transparent (GL) main hit it every time.
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
	- Cause: Regression from the menu bar. Cell backgrounds, the cursor, and the bars are all rect quads in absolute framebuffer pixels (same space as the glyphon text viewport), but `rects.set_resolution` (and the bg-image shader) were being fed the content `area` height, which the menu bar made shorter than the window - so the shader's `px.y / resolution.y` mapping pushed every quad down relative to the text.
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
	- Fix: `measure_cell` now measures the real rendered pitch (`Shaping::Advanced`, averaged over 40 `M`s) and `cell_w` is not rounded -> it matches the text, residual drift is sub-pixel. Plus per-cell fallback glyphs are now fit to their cell box: `fill_glyph` returns the glyph's true ink width/offset (rasterized; advance != ink for `➡`/emoji) and `build` scales+centers each via `TextArea.scale` so an over-wide fallback can't spill onto its neighbour. Verified at runtime: cursor sits one cell after `…$ ` with no drift; `A➡B➡C…` and `A😀B…` align at col 0 and col ~40; CJK/emoji = 2 cells; math/box-drawing stay in-buffer and crisp.
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
	- Verified: Linux at runtime (13pt -> 1528x1016), Windows cross-build compiles; macOS path is std-only subprocess, not run-tested here (no mac target).

- ✅ Native keybindings for `less` don't work.
	- Fix: `less` enables application-cursor-keys mode (DECCKM); arrow / Home / End are now encoded as `ESC O x` instead of `ESC [ x` when that mode is active. The mouse wheel also now drives full-screen apps: when the alternate screen / alternate-scroll mode is active it sends cursor-key presses instead of moving the (nonexistent) scrollback.

#### Done - new features and enhancements

- ✅ README screenshots, refreshed after significant visual changes: five anonymized shots (shell session, split panes, transparency + background image + glow, tabs / 24-bit / Unicode, Settings dialog) rendered at 1920x1080 and downsampled to 640x360 thumbnails.
	- Done: originals in `assets/screenshots/large/`, thumbnails in `assets/screenshots/`, shown as a grid in the README that links each thumbnail to its full-size image.
	- Note: the renderer (`utility/screenshots.bash`) runs in cicd before publish (skipped under `--quick`), so regenerated shots get committed with the visual change.

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
		- Done: the dialog is tied to the terminal window. X11 gets a transient-for hint; Windows and macOS use winit's owner/parent. The WM keeps it above the terminal and groups them. Simulated modal: while a dialog is open the main window swallows keyboard, wheel, and IME input, and clicking it re-focuses the dialog. Applies to About too.
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

- Added 'silkterm/github/cicd/utility/gui-headless.bash'. It allows running the terminal for testing in a GUI environment that doesn't interfere with current user session, via use of xvfb in the script.
	- ✅ Update all tests, scripts, and profiling to run in that environment. (20260701)
		- Done: the profiler stage brings up the private Xvfb and runs the app there, so no window pops on the live session. It skips only if Xvfb, python3, or the workload are missing. Unit tests need no display anyway.
		- Verified: the app renders on Xvfb via software GL and the profiler produced a valid flamegraph.

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
	- Note: the cross-platform-windowing-widget question (the `[🚫]` note under "Setting dialog (part 2)") is now decided - chrome stays hand-rolled (egui declined after a real spike). So the bar-title Alt underline is just a normal hand-rolled task.

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
			- Verified on a dark gradient. Visibly smooth. `dump_offscreen` updated to decode f16.

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
	- [🚫] macOS ARM64: Deferred. cross-compiling Linux->macOS needs Apple's SDK (osxcross), which is license-gated; do it on a Mac / in CI.
	- [🚫] macOS x86_64: Deferred. (Same; Mac/CI.)
	- Toolchain setup + commands are in `build.md`; one-time: install zig + `cargo install cargo-zigbuild` + `rustup target add aarch64-unknown-linux-gnu aarch64-pc-windows-gnullvm`. No ARM64 system libs needed (X11/EGL dlopen'd at runtime).

- ✅ True transparency:
	- Bug (fixed): Adjusting the transparency affects only the overall terminal background (including image which already has it's own correctly functioning opacity).
	- Transparency should not affect the Window decorations, menu, focus, or - critically - terminal text.
	- Status: Done. Opt-in `transparent_background = true`; `opacity` is the background alpha; text, decorations, and the menu/tab bars stay opaque. Verified on X11/Compiz/NVIDIA, decorated and borderless. Default (`false`) path unchanged (native wgpu).
	- How: wgpu can't get per-pixel alpha on X11 by itself (its Vulkan swapchain forces an opaque surface; its GL backend won't bind the 32-bit ARGB visual). So on X11 we create the window + a transparent GL context with glutin and run wgpu on top of it via hal interop (`Gfx::new_gl_transparent`), render the scene to an offscreen texture, then blit that into the GL framebuffer. Off X11 (e.g. Wayland) the plain wgpu surface already does premultiplied alpha. `Gfx` is a two-backend enum (native wgpu / GL). No wgpu downgrade, no renderer rewrite.
	- The hard part (cost ~a day; a web search cracked it - gfx-rs/wgpu #8675 + #8676): on NVIDIA/Linux glyphon renders no text on a GL context below 4.2, because wgpu silently no-ops drawing into a texture view there (that's how glyphon builds its atlas). Fix: request GL 4.6, falling back down to 3.3. Diagnostics: `SILK_DUMP=1` dumps the offscreen to `/tmp/silk_offscreen.png`; `diagnostics/glyphon_gl.rs` is an off-screen probe.

- ✅ Make both the main menu, and the right-click menu appearances more traditional:
	- ✅ Use the system proportional font, rather than monospace font. - New `text::sans_attrs()` (cosmic-text `Family::SansSerif` -> the system default proportional font); the menu bar titles, dropdowns, and the right-click menu all use it.
	- [🚫] Use the system menu background and text color if reasonably feasible in a cross-platform way.
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
	- [🚫] Use the system window background and text color if reasonably feasible in a cross-platform way.
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
	- [🚫] Use the system window background and text color, if feasible in a cross-platform way.
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
	- [🚫] Hide scrollbar (toggle with checkmark)
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
	- Solution: Copy-on-select writes to the primary selection (copypasta `X11ClipboardContext<Primary>`, held for the app's lifetime so ownership persists). Middle-click reads primary and writes it to the pane under the cursor, wrapped in bracketed-paste when the app enabled it. Verified: primary -> middle-click -> bytes reached the shell. `src/clipboard.rs`.

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

- ✋ Modal Bug - About only (almost certainly a Compiz issue): with the About/Settings dialog open, selecting another window then re-selecting the dialog leaves the terminal buried behind whatever got in front, instead of both coming to the top together. Settings now works; About still does this on the owner's real Compiz desktop.
	- Almost certainly a Compiz WM issue, not a SilkTerm bug: About and Settings use the exact same dialog code path (window creation, transient-for + EWMH dialog/MODAL/SKIP_TASKBAR hints, and the raise-with-parent restack), so a difference between them is the WM's handling, not our code.
	- General case is fixed: hints set pre-map, plus - since Compiz won't raise a transient's parent - an _NET_RESTACK_WINDOW that slots the terminal under the dialog on focus, re-asserted ~1.2s to outlast Compiz's animated settle. Verified on headless Compiz for both dialogs; the About-only failure couldn't be reproduced headlessly (both pass on ccp Compiz). SILK_MODALDBG=1 traces the restack + the resulting stack order to stderr if it's revisited.

- ✋ Alt-screen enter/exit animated like a scroll (`smooth_scroll_apps`). Two symptoms: (a) opening nano "jiggles"/jelly-bounces or scrolls in from a few lines down; (b) exiting nano scrolls the previous screen contents back in from the bottom, where a normal terminal just cuts.
	- Cause: an alt-screen enter/exit is an instant full-screen swap, but the scroll probes diffed frame-to-frame across it. On enter the app-scroll probe matched blank rows between the old and new screens -> bogus slide (jiggle). On exit `history_size` jumps (the alt grid carries no scrollback) -> the output-ease read it as new output and scrolled the restored screen in.
	- Fixed: track the previous frame's alt-screen state; on a transition hard-cut it - cancel any in-flight slide, skip both probes, suppress the output nudge, and rebaseline the row fingerprints to the new screen.
	- Confirmed fixed by owner (both symptoms). Residual: a very slight 1-line smooth scroll-up still happens on enter and exit - livable, deferred (see the deferred item below).
	- **Verified**: Mostly fixed. Entering and exiting still result in a one-line slow/smooth scroll. Tolerable, but fix someday.

- ✋ Residual 1-line smooth scroll-up on alt-screen enter AND exit (`smooth_scroll_apps`). The enter/exit hard-cut fixed the big jiggle/scroll-in (owner confirmed), but a slight single-line ease still rides the transition. Owner: livable, deferred. Likely the output-ease firing one frame after the transition (the frame after the hard-cut isn't suppressed, and a 1-line history delta on the primary-screen restore nudges it) - a candidate fix is to also rebaseline `last_history` and extend the nudge suppression one frame past the transition.

- ✋ Minority Report mode: Borderless, transparent, changes perspective depending on screen location.

- ✋ Implement branch protection rules on main:
	- ✋ Require a pull request before merging (blocks direct pushes), and
	- ✋ Require review from Code Owners.
	- ✋ In more distant future: Do not allow bypassing / include administrators
		- Without this, I (as OG admin) can still merge around it, which is good early on.

### Canceled

- 🚫 CI/CD scripts:
	- 🚫 Build alternate targets in parallel, to speed process up.
		- Too fiddly. Possibly revisit in future. This lives in `cicd.bash`, which is pseudo-generic and could be made more so. Maybe it can shell out to a hyper-specific build script, or be updated to handle rust, go, and c++. Or more likely, it's just project-specifig, in spite of being originally [re]architected to call a settings script.

- Setting dialog (part 2):
	- [🚫] Adopt a cross-platform GUI / windowing widget toolkit (e.g. egui) for Settings, About, the main menu, and the context menu instead of hand-rolling them.
		- **No**. Results of spike (branch `spike/egui-dialog`): The upside is that egui 0.35 rides our exact wgpu 29 + winit 0.30 (no downgrade, shares our graphics stack) and integrated easily.
		- Drawbacks to egui: it adds ~32% to the release binary for what is secondary chrome, against the minimal-binary-size priority. Hand-rolling also keeps one unified colour/theme + native-OS-font system across the terminal and the chrome. egui would need a separate egui-`Visuals` theme kept in sync, plus its own bundled fonts).
		- Decision: Chrome stays hand-rolled.

- 🚫 Allow toggling from default "Insert" mode, to "Overwrite". (20260629)
	- 🚫 Change cursor in default "Insert" mode, to a thinner bar than the block cursor (but thicker than, say, "|").
	- 🚫 Overwrite mode will be the regular block cursor.
		- Overwrite mode canceled.
	- Backed out (20260630): overwrite mode + the Insert-key toggle removed (a terminal can't force the shell's line editor to overwrite). Kept the cursor work - configurable shape, blink, smooth slide. Insert key now just passes through to the shell.
	- Resolution: This can't be done without wonky hacks.

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
