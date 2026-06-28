<!-- markdownlint-disable MD007 -- Unordered list indentation -->
<!-- markdownlint-disable MD010 -- No hard tabs -->
<!-- markdownlint-disable MD033 -- No inline html -->
<!-- markdownlint-disable MD055 -- Table pipe style [Expected: leading_and_trailing; Actual: leading_only; Missing trailing pipe] -->
<!-- markdownlint-disable MD041 -- First line in a file should be a top-level heading -->
<div align="center">

[![made-with-rust](https://img.shields.io/badge/Made%20with-Rust-1f425f.svg)](https://www.rust-lang.org/)
[![License: GPL v2+](https://img.shields.io/badge/License-GPLv2%2B-blue.svg)](https://www.gnu.org/licenses/old-licenses/gpl-2.0.html)
![Lifecycle: Beta](https://img.shields.io/badge/Lifecycle-Beta-yellow)
![Support](https://img.shields.io/badge/Support-Maintained-brightgreen)
![Status: Passing](https://img.shields.io/badge/Status-Passing-brightgreen)

</div>
<!--
![Go](https://img.shields.io/badge/Go-00ADD8?logo=go&logoColor=white)
[![!#/bin/bash](https://img.shields.io/badge/-%23!%2Fbin%2Fbash-1f425f.svg?logo=gnu-bash)](https://www.gnu.org/software/bash/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![License: GPL v2](https://img.shields.io/badge/License-GPLv2-blue.svg)
![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)
![Lifecycle: Alpha](https://img.shields.io/badge/Lifecycle-Alpha-orange)
![Lifecycle: Beta](https://img.shields.io/badge/Lifecycle-Beta-yellow)
![Lifecycle: RC](https://img.shields.io/badge/Lifecycle-RC-blue)
![Lifecycle: Stable](https://img.shields.io/badge/Lifecycle-Stable-brightgreen)
![Lifecycle: Deprecated](https://img.shields.io/badge/Lifecycle-Deprecated-red)
![Status: Deprecated](https://img.shields.io/badge/Status-Deprecated-orange)
![Status: Archived](https://img.shields.io/badge/Status-Archived-lightgrey)
![Lifecycle: EOL](https://img.shields.io/badge/Lifecycle-EOL-lightgrey)
![Coverage](https://img.shields.io/badge/Coverage-25%25-red)
![Coverage](https://img.shields.io/badge/Coverage-50%25-orange)
![Coverage](https://img.shields.io/badge/Coverage-75%25-yellow)
![Coverage](https://img.shields.io/badge/Coverage-90%25-brightgreen)
![Status: Passing](https://img.shields.io/badge/Status-Passing-brightgreen)
![Status: Failing](https://img.shields.io/badge/Status-Failing-red)
-->

<!-- TOC ignore:true -->
# SilkTerm

<table style="border: none; border-collapse: collapse;">
	<tr style="border: none; border-collapse: collapse;">
		<td style="border: none; border-collapse: collapse;"><img src="assets/logo.png" alt="Silky" width="320"/></td>
		<td style="border: none;">SilkTerm is the only known terminal on Earth that smooth-scrolls lines on output, for silky-smooth and less-tiring long terminal sessions. It also has all the other features of modern advanced terminals, and more.</td>
	</tr style="border: none; border-collapse: collapse;">
</table>

<!-- TOC ignore:true -->
## Table of contents

<!-- TOC -->

- [Why?](#why)
	- [Why smooth-scrolling output](#why-smooth-scrolling-output)
	- [Why text outer glow](#why-text-outer-glow)
- [Features](#features)
- [Limitations](#limitations)
- [Configuration](#configuration)
- [Installing](#installing)
- [Building from source](#building-from-source)
- [Design](#design)
- [Copyright and license](#copyright-and-license)

<!-- /TOC -->

## Why?

### Why smooth-scrolling output

Literally *all* other terminal emulators in existence at the time this was written, currently snap scrolling output to fixed lines. Nothing ever appears in-between those lines (except when mouse-scrolling on some terminals).

For output that can be sporadic - e.g. something scrolling slowly one line-at-a-time sometimes, then jumping several lines at once other times (e.g. while watching a live log file with `tail -f`), [the eye/brain combo can struggle to track the output](https://www.youtube.com/watch?v=yQaC-ZzTf78), and you get "lost" trying to read it.

One analogy is playing a video game with mouse-look at, say, 3 frames-per-second visual output. It is nearly impossible to keep your bearings, when the world view jumps wildly from frame-to-frame. But at say 240 FPS on a matching Hz monitor, it looks buttery smooth and immersive, and the subtle task of mentally maintaining where you are, becomes trivial.

As the youtube video linked above goes into, jerky line-snapped output taxes mental resources - however slightly - in a way that stacks up over long sessions. At the extreme, it can contribute to headaches and fatigue. And that's brainpower that could have been used to solve whatever it is you're working on.

The crazy thing is that **several early CRT text-mode computers offered smooth-scrolling**. (For example, many UNIX client terminal consoles of the 80's.)

So when it's said that SilkTerm is "the only one to offer it", that means *now* - not across time.

The smooth-scrolling output concept was completely abandoned in the 80s and 90s, because:

- Rate-limited output scrolling would cap fast output, and possibly overflow the scrollback buffers resulting in lost output.

	- *SilkTerm solves this problem by automatically ramping up the scroll speed, smoothly, as needed to keep up with output speed.*

- Smooth scroll solved the same "tracking-a-moving-line" problem, that scrollback buffers + pagers (such as `more`, `less`) later solved better, with the technology available at the time.

Video examples of early smooth-scroll displays:

- [DEC VT100 - VT420](https://www.youtube.com/watch?v=tSJfzrSA0ec)
- [Wyse WY*nn*](https://www.youtube.com/watch?v=8q6YPAzH02s)

SilkTerm's smooth-scrolling output is a joy to work with, you really have to try it to "get" it. And the faster your monitor display Hz, the more gorgeous it feels.

### Why text outer glow

Generally speaking, "outer-glow" is a readability aid - whereas angled "drop-shadow" is a creative effect. (This isn't a hard-and-fast graphic design "rule" - as there is lots of overlap in both directions.)

If you've ever used a terminal that supports background transparency, and/or background images (both of which SilkTerm offers), that novelty can quickly wear off. You'll notice that the text might be too hard to read, particulary in a long computing session.

Text can be particularly hard to read, for example when using light text on a normally dark background, and:

- The background is very transparent, and the window is on top of bright and/or visually "busy" content below. And/or,

- The background image is bright.

- (Or vice-versa for dark text on a normally light background, with dark elements under the text.)

"Drop-shadow" is a feature available on at least a half-dozen other terminal emulators, but apparently only for novelty effect. Because if you use it for very long, it can make your mental workload subtly higher, and your visual cortex tires faster - or something. (I don't know, I'm not a neuroscientist, why are you asking me.)

"Outer glow" - or similar techniques by other names (and distinctly *not* angled "drop-shadow") - is used often in graphic design and advertising to aid readability on backgrounds of varying brightness and color. (And some closed-captioning systems use it as an alternative to black bars as a background.)

## Features

- **Smooth pixel-at-a-time scrolling on terminal output**.

	- *You HAVE to see how gorgeous it looks on a high-refresh rate monitor. No animated gif reproduction can do it justice*.

- **Smooth mouse wheel scrolling**. Several other terminals offer this feature.

- **Background tranparency**. The background (with adjustable %) becomes see-through, but not the text.

- **Outer glow behind text**. This optional feature helps keep text readable even over similar-colored backgrounds and/or high transparency. This is the only known terminal to offer it, though there are several terminals that do offer angled *drop-shadow* (which ironically can make text *harder* to read). Outer glow is conceptually similar - but enhances, rather than reduces, readability.

- **User-selectable background image**. User-selectable, with a few dozen cool offerings included.

	- The background image can be dimmed with adjustable %, relative to the background color - and independent of main background transparency.

- **Background image blur**: With an optional gaussian blur radius (without altering the source image).

- **Split panes**: A native feature to arbitrarily split any pane in either direction. Panes can be freely drag-n-dropped to change locations.

- **Window decorations and/or menu can be disabled**, for "nothing but terminal". Fullscreen can also be toggled.

- **Unicode and emoji support**, as well as true-color, 256-color, 16-color text; bold & italic.

- **Internal Unicode fallback rendering** for the glyphs that the chosen display font can't display.

- **Read-only output toggle**.

- **Rich command-line syntax** that allows creating multiple tabs and/or complex pane structure(s) at launch time.

	- This can be very useful for creating one-line shell scripts that launch custom SilkTerm instances with specific size, background, color, opacity, and unique shells per window, tab, and/or pane. (Without overwriting the main config file.)

- **Arbitrary alternate config files possible**, another way to launch SilkTerm with wildly different options, without overwriting the main config file.

- **Written in rust** for minimum executable size, no dependencies, and maximum speed. (Several terminal emulators - such as the revered `terminator` - are written in interpreted Python.)

- **One codebase for Linux + Windows**. The Window and/or ARM versions can be built all at once on x86_64 Linux. *MacOS however requires a native build, but is fully supported*.

- **Loosely based on [Alacritty](https://github.com/alacritty/alacritty)** (not a fork) for the basement plumbing, to avoid rewriting the complex but solved problems of terminal emulation. Alacrity is a high-performance terminal written in Rust.

- **GPU-acellerated** with software fallback.

- **Simple and sane configuration**. No pages of nested tabs representing multiple settings metaphors. (E.g. no separate "Profiles" and "Layouts".) If you want to get fancy with multiple sets of wildly different options - that's easy with alternate config files, and/or scripted launch-time arguments.

## Limitations

- SilkTerm can only smooth-scroll text written to `stdout` and `stderr`.

	- This covers the overwhelming majority of Linux terminal tools and programs.

	- However, some TUI programs - such as `nano`, `vim`, `tmux` - manage scrolling themselves, and directly control the terminal buffer in "raw mode". SilkTerm can't handle such output. Scrolling within such programs behaves the same as on any other terminal - snapped to lines, no in-between.

- Some rare hybrid programs write to `stdout` and `stderr`, *and* write to the terminal buffer in raw mode for fixed portions. If that fixed portion is on the last line, it will visibly "bounce" slightly, as text is pushed up the screen. (But at least the bounce is smooth.)

	- It may be possible to work around this someday, but for the forseeable future, it's just a minor niggle to live with, and (arguably) not annoying.

	- The only program currently known to exhibit this behavior, is `apt`, with the status bar fixed to the bottom. (If you notice any others, please file an issue)

<!--
## Default keybindings

| Key | Action |
| --- | --- |
| `Ctrl+Shift+R` | Split focused pane vertically (side by side) |
| `Ctrl+Shift+D` | Split focused pane horizontally (top / bottom) |
| `Ctrl+Shift+W` | Close focused pane |
| `Ctrl+Shift+Tab` | Cycle focus to the next pane |
| Right-click | Open a context menu (Split Vertical / Split Horizontal / Close Pane) |
| Left-click | Focus the pane under the cursor (or pick a menu item) |
| `Escape` | Dismiss the context menu |
| Mouse wheel | Smooth-scroll the pane under the cursor through scrollback |

Typing routes to the focused pane and snaps it back to the bottom.
-->

## Configuration

On first run SilkTerm writes a commented config file with all defaults to:

```bash
$XDG_CONFIG_HOME/silkterm/config.toml   (falls back to ~/.config/...)
```

Edit it and restart. Unknown or malformed entries fall back to defaults; delete the file to regenerate it.

## Installing

## Building from source

See [build.md](build.md). Quick start on Linux:

```bash
cargo run --release
```

<!--
## Renaming the project

The display name lives in one place (`APP_NAME` in `src/config.rs`); the lowercase identifier (`silkterm`) is the cargo package, binary, and config directory. To rename everything at once during development:

```sh
tools/rename.sh NewName
cargo build
```

It rewrites `Cargo.toml`, the Rust sources, and the docs (review `git diff`
afterwards); `cargo build` regenerates `Cargo.lock`.
-->

## Design

See [design.md](project/design.md) for the general architecture, and initial bugs and features before the first release. (All of which are tracked as [Issues](https://github.com/jim-collier/silkterm/issues) after initial release.)

## Copyright and license

> Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)<br />
> Licensed under the GNU General Public License v2.0 or later ([GPL-2.0-or-later](https://spdx.org/licenses/GPL-2.0-or-later.html)). No warranty.
