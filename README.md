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
![Status: Failing](https://img.shields.io/badge/Status-Failing-red)
-->

<!-- TOC ignore:true -->
# SilkTerm

<table style="border: none; border-collapse: collapse;">
	<tr style="border: none; border-collapse: collapse;">
		<td style="border: none; border-collapse: collapse;"><img src="source/assets/logo.png" alt="Silky" width="320"/></td>
		<td style="border: none;">SilkTerm is the only known terminal currently in existence, that smooth-scrolls lines on output - for silky-smooth and less-tiring long terminal sessions. It also has smooth cursor options such as phase effect for blinking, and smooth movement.<br /><br />SilkTerm also has detachable multi-tabs, split-panes, transparency and blur, background image and blur, text outer-glow, and can run without a menu and/or window decorations.<br /><br />Cross-platform. Written in Rust for a small single executable, and blazing speed.</td>
	</tr style="border: none; border-collapse: collapse;">
</table>

<!-- TOC ignore:true -->
## Table of contents

<!-- TOC -->

- [Why?](#why)
	- [Why smooth-scrolling output](#why-smooth-scrolling-output)
	- [Why text outer glow](#why-text-outer-glow)
- [Features](#features)
	- [One minor limitation inherent to all terminals](#one-minor-limitation-inherent-to-all-terminals)
- [Configuration](#configuration)
- [Installing](#installing)
- [Building from source](#building-from-source)
- [Design](#design)
- [Copyrights and licenses](#copyrights-and-licenses)

<!-- /TOC -->

## Why?

### Why smooth-scrolling output

Literally *all* other terminal emulators in existence at the time this was written, currently snap scrolling output to fixed lines. Nothing can appear in-between those lines (except when mouse-scrolling on some terminals).

For output that can be sporadic - e.g. something scrolling slowly one line-at-a-time sometimes, then jumping several lines at once other times (e.g. while watching a live log file with `tail -f`), [the eye/brain combo can struggle to track the output](https://www.youtube.com/watch?v=yQaC-ZzTf78), and you get "lost" trying to follow it.

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

Generally speaking, "outer-glow" (usually of the opposite luminosity to the text) is a readability aid - whereas angled "drop-shadow" is a creative effect. (Though, this isn't a hard-and-fast graphic design "rule" - as there is lots of overlap in both directions.)

If you've ever used a terminal that supports background transparency, and/or background images (both of which SilkTerm offers), that novelty can quickly wear off. You'll notice that the text might be too hard to read, particularly in a long computing session.

Text can be particularly hard to read, for example when using light text on a normally dark background, and:

- The background is very transparent, and the terminal is on top of bright and/or visually "busy" content below. And/or,

- The background image is bright.

(*Or vice-versa for dark text on a normally light background, with dark elements under the text.*)

"Drop-shadow" is a feature available on at least a half-dozen other terminal emulators, but apparently only for novelty effect. Because if you use it for very long, it can make your mental workload subtly higher, and your visual cortex tires faster - or something. (I don't know, I'm not a neuroscientist, why are you asking me.)

"Outer glow" - or similar techniques by other names (and distinctly *not* angled "drop-shadow") - is used often in graphic design and advertising to aid readability on backgrounds of varying brightness and color. (And some closed-captioning systems use it as an alternative to black bars as a background.)

## Features

- **Smooth pixel-at-a-time scrolling on terminal output**.

	- *You HAVE to see how gorgeous it looks on a high-refresh rate monitor. No animated gif reproduction can do it justice*.

- **Smooth mouse wheel scrolling**. Several other terminals offer this feature.

- **Smooth cursor movement**. This is the cherry on top of "smooth".

- **Outer glow behind text**. This optional feature helps keep text readable even when the text is on top of similar-colored backgrounds and/or when using high background transparency. This is the only known terminal to offer it, though there are several terminals that offer angled *drop-shadow* (which ironically can make text *harder* to read). Outer glow is conceptually similar - but enhances, rather than reduces, readability.

- **Cursor size and animation options**. Phased blinking, or smoothly pulsing in size. (Or just regular.) Adjustable rate.

- **Background transparency**. The background (with adjustable %) becomes see-through, but not the text.

- **Background transparency blur**. If using background transparency and this is enabled, everything behind the terminal is blurred. Supported on most window compositors. (But limited to the compositor's options. SilkTerm just talks to the WM to enable it.)

- **User-selectable background image**. User-selectable, with a few dozen cool offerings included.

	- The background image can be dimmed with adjustable %, relative to the background color - and independent of main background transparency.

- **Background image blur**: With an optional Gaussian blur radius (without altering the source image), also independent of transparency blur.

- **Split panes**: A native feature to arbitrarily split any pane in either direction. Panes can be freely drag-n-dropped to change locations. Panes split in successive directions are automatically evenly distributed, unless adjusted (with the mouse).

- **Window decorations and/or the menu can be disabled**, for "nothing but terminal". Fullscreen can also be toggled.

- **Robust Unicode and emoji support**. With internal Unicode fallback rendering for the glyphs that the chosen display font can't display.

- **True-color, 256-color, and 16-color text support, as well as standard bold & italic**.

- **Read-only output toggle**.

- **Rich command-line syntax** that allows creating multiple tabs and/or complex pane structure(s) at launch time.

	- This can be very useful for creating one-line shell scripts that launch custom SilkTerm instances with specific size, background, color, opacity, and unique shells per window, tab, and/or pane. (Without overwriting the main config file.)

- **Arbitrary alternate config files**, another way to launch SilkTerm with wildly different options, without overwriting the main config file.

- **Written in Rust** for minimum executable size, no runtime dependencies, and maximum speed. (Several terminal emulators - such as the revered `terminator` - are written in interpreted Python.)

- **One codebase for Linux + Windows, both with x86_64 and ARM builds**. The Window and/or ARM versions can be built all at once on x86_64 Linux. *MacOS is built natively on a Mac from the same codebase, but is so far untested (no releases target it yet)*.

- **Loosely based on [Alacritty](https://github.com/alacritty/alacritty)** (not a fork), just for the basement plumbing - to avoid rewriting the complex but solved problems of terminal emulation. Alacritty is also a high-performance, open-source terminal written in Rust.

	- *Fun fact: SilkTerm has more lines of code than Alacritty, especially compared to the subset we use. Which is part of why we chose it for the bare guts without reinventing a thoroughly-and-repeatedly-invented wheel.*

- **GPU-accelerated** with software fallback.

- **Simple and sane configuration**. No pages of nested tabs representing multiple settings metaphors. (E.g. no separate "Profiles" and "Layouts".) If you want to get fancy with multiple sets of wildly different options - that's easy with alternate config files, and/or scripted launch-time arguments.

### One minor limitation inherent to all terminals

- SilkTerm can only smooth-scroll text written to `stdout` and `stderr`.

	- This covers the overwhelming majority of Linux terminal tools and programs.

	- However, some TUI programs - such as `nano`, `vim`, `tmux` - directly control the terminal buffer in "raw mode", and handle everything themselves. Scrolling within such programs behaves the same as on any other terminal - snapped to lines, no in-between.

		- But the other features still work in that case: smooth-moving and phased cursor, text outer-glow, background options, etc.

## Configuration

On first run SilkTerm writes a commented config file with all defaults to:

```bash
$XDG_CONFIG_HOME/silkterm/config.toml   (falls back to ~/.config/...)
```

If making changes directly (rather than through Settings), you can apply them immediately with the "Reload config" menu item.

## Installing

Pre-built releases are not published yet - build from source per the Compiling section. Optional: copy the example config tree in [`filesystem/home/`](filesystem/home/) over your own `$HOME` for a starter config and the background image pack.

## Building from source

See [build.md](build.md).

Quick start on Linux:

```bash
cargo run --release
```

Or for the full CI/CD pipeline (lint, debug compile, regression test, profile, release builds, versioned backup, commit to git, push):

```bash
cicd/cicd.bash [--quick]
```

<!--
## Renaming the project

The display name lives in one place (`APP_NAME` in `source/src/config.rs`); the lowercase identifier (`silkterm`) is the cargo package, binary, and config directory. To rename everything at once during development:

```sh
utility/rename.bash NewName
cargo build
```

It rewrites `Cargo.toml`, the Rust sources, and the docs (review `git diff`
afterwards); `cargo build` regenerates `Cargo.lock`.
-->

## Design

See [design.md](project/design.md) for the general architecture and decisions, and [backlog.md](project/backlog.md) for bugs and features tracked before the first release. (Github [Issues](https://github.com/jim-collier/silkterm/issues) are used after the first release.)

## Copyrights and licenses

[Alacritty](https://github.com/alacritty/alacritty) is dual-licensed under the [Apache License, Version 2.0](https://github.com/alacritty/alacritty/blob/master/LICENSE-APACHE) and [MIT License](https://github.com/alacritty/alacritty/blob/master/LICENSE-MIT).

SilkTerm's license, although different, is fully compatible with Alacritty's:

> Copyright © 2026 Jim Collier (ID: 1cv◂‡Vᛦ)<br />
> Licensed under the GNU General Public License v2.0 or later ([GPL-2.0-or-later](https://spdx.org/licenses/GPL-2.0-or-later.html)). No warranty.