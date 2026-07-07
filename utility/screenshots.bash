#!/usr/bin/env bash

#  shellcheck disable=1091  ## 'source is valid here, but shellcheck doesn't know the path to it.'
#  shellcheck disable=2001  ## 'See if you can use ${variable//search/replace} instead.' Complains about good uses of sed.
#  shellcheck disable=2016  ## 'Expressions don't expand in single quotes.' Often an explicit '$' is wanted.
#  shellcheck disable=2034  ## 'variable appears unused.' Variable indirection / name-refs.
#  shellcheck disable=2046  ## 'Quote to prevent word-splitting.' OK for integers / built arg lists.
#  shellcheck disable=2086  ## 'Double quote to prevent globbing and word splitting.' OK for integers / arg arrays.
#  shellcheck disable=2155  ## 'Declare and assign separately.' Cumbersome and unnecessary here.
#  shellcheck disable=2317  ## 'Can't reach.' Debug exits.

##	Purpose:
##		Regenerate SilkTerm's README screenshots. Renders a spread of anonymized
##		scenes on a private Xvfb (never the visible :0 session) and downsamples
##		them into the repo's assets/screenshots tree. No real user/host/path or
##		personal config leaks in - every scene runs against a throwaway config.
##		Meant to run after a significant visual change; cicd runs it before the
##		publish stage (skipped under --quick), so refreshed images get committed.
##	Syntax:
##		screenshots.bash [REPO_DIR]
##		  REPO_DIR   the 'github' working tree (default: derived from this script's
##		             location, ../ - it lives in github/utility). Env overrides:
##		               SILK_BIN            silkterm binary (default REPO/target/release/silkterm)
##		               SILK_SHOT_DISPLAY   Xvfb display (default :98)
##	Notes:
##		Captures the main window via SILK_DUMP (the GL offscreen readback - the
##		client area only, no WM chrome, at exactly the window's pixel size), which
##		is why the scenes run fullscreen on a 16:9 virtual screen: the dump is a
##		clean 2560x1440 that downsamples to 1920x1080 / 640x360 with no upscaling.
##		The Settings dialog is a separate (non-GL) window, grabbed with 'import'.
##		Non-fatal: any single scene failing just warns and moves on.
##	History: at bottom.

##	Copyright © 2026 Jim Collier
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT


set -euo pipefail
shopt -s inherit_errexit

##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Globals / small helpers

declare meDir=""
declare repoDir=""
declare binPath=""
declare display=""
declare authFile=""
declare workDir=""
declare outLarge=""
declare outThumb=""
declare -a launchedPids=()

## 16:9 virtual screen so the fullscreen dump is a clean 16:9 (no crop needed).
declare -r screenSize="2560x1440x24"
## "Medium" weight isn't separately selectable (font_family picks a family, not a
## weight), so the stack lands on the Argon NF family, falling back to DejaVu.
declare -r fontName="Monaspace Argon NF Medium, Monaspace Argon NF, DejaVu Sans Mono"
declare -ri fontSize=24

fInfo()  { printf '  %s\n' "$*"; }
fStep()  { printf '\n[ %s ]\n' "$*"; }
fWarn()  { printf 'WARNING: %s\n' "$*" >&2; }
fErr()   { printf 'ERROR: %s\n'   "$*" >&2; }

## esc/ANSI helper embedded into scene scripts (kept as a here-string preamble).
declare -r sceneHead='#!/bin/dash
e=$(printf "\033"); c(){ printf "%s[%sm" "$e" "$1"; }; r(){ printf "%s[0m" "$e"; }
'


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Setup / teardown

fResolvePaths() {
	meDir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
	repoDir="${1:-}"
	if [[ -z "$repoDir" ]]; then
		## github/utility -> github
		repoDir="$(cd -- "${meDir}/.." 2>/dev/null && pwd || true)"
	fi
	[[ -n "$repoDir" && -d "$repoDir" ]] || { fErr "repo dir not found: ${repoDir:-<unset>}"; return 1; }
	binPath="${SILK_BIN:-${repoDir}/target/release/silkterm}"
	[[ -x "$binPath" ]] || { fErr "silkterm binary not found/executable: $binPath"; return 1; }
	display="${SILK_SHOT_DISPLAY:-:98}"
	local -r num="${display#:}"
	authFile="/tmp/cicd-gui-headless-${USER}/Xauthority-${num}"
	outLarge="${repoDir}/assets/screenshots/large"
	outThumb="${repoDir}/assets/screenshots"
	mkdir -p "$outLarge" "$outThumb"
	workDir="$(mktemp -d "/tmp/silk-shots-XXXXXX")"
}

fStartHeadless() {
	fStep "Starting private Xvfb on $display ($screenSize)"
	CICD_HEADLESS_DISPLAY="$display" "${repoDir}/cicd/utility/gui-headless.bash" stop >/dev/null 2>&1 || true
	CICD_HEADLESS_DISPLAY="$display" CICD_HEADLESS_SIZE="$screenSize" \
		"${repoDir}/cicd/utility/gui-headless.bash" start --wm
}

fStopHeadless() {
	fKillLaunched
	CICD_HEADLESS_DISPLAY="$display" "${repoDir}/cicd/utility/gui-headless.bash" stop >/dev/null 2>&1 || true
}

## Kill only the silkterm instances we launched, by captured PID - never by name.
fKillLaunched() {
	local p
	for p in "${launchedPids[@]:-}"; do
		[[ -n "$p" ]] || continue
		kill "$p" 2>/dev/null || true
	done
	launchedPids=()
}

fCleanup() {
	fStopHeadless || true
	[[ -n "$workDir" && -d "$workDir" ]] && rm -rf "$workDir"
}
trap fCleanup EXIT


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Assets: throwaway configs, scene scripts, a synthetic background image

## Write a per-scene config. Args: file, then extra 'key = value' lines.
fWriteConfig() {
	local -r file="$1"; shift
	{
		printf 'use_system_font = false\n'
		printf 'font_family = "%s"\n' "$fontName"
		printf 'font_size = %s\n' "$fontSize"
		printf 'remember_size = false\n'
		printf 'theme = "SilkTerm"\n'
		printf 'theme_mode = "dark"\n'
		printf 'cursor_animation = "none"\n'   ## static cursor -> byte-stable shots run-to-run
		local kv
		for kv in "$@"; do printf '%s\n' "$kv"; done
	} > "$file"
}

## Synthetic, abstract, personal-content-free background (plasma, blurred, dark).
fMakeBgImage() {
	local -r out="$1"
	## seeded plasma -> soft, saturated, darkened. The fixed -seed keeps the image
	## byte-stable run-to-run so cicd doesn't churn the committed PNG each time.
	magick -seed 7 -size 2560x1440 plasma:'#0b2550'-'#2a0b4a' \
		-blur 0x12 -modulate 82,120,100 -brightness-contrast -6x8 "$out" 2>/dev/null \
		|| magick -size 2560x1440 gradient:'#0b1e3a-#1a0b2e' "$out"
}

fWriteScenes() {
	## --- 01 shell: a fuller coding session ---------------------------------
	cat > "${workDir}/shell.sh" <<EOF
${sceneHead}
p(){ printf "%suser@silk%s:%s~/projects/webapp%s\$ " "\$(c '1;32')" "\$(r)" "\$(c '1;34')" "\$(r)"; }
p; printf "git status\n"
printf "On branch %sfeature/smooth-scroll%s\n" "\$(c '1;36')" "\$(r)"
printf "Your branch is up to date with 'origin/feature/smooth-scroll'.\n\n"
printf "Changes not staged for commit:\n"
printf "  %smodified:   src/render/glow.rs%s\n"   "\$(c 31)" "\$(r)"
printf "  %smodified:   src/render/scroll.rs%s\n" "\$(c 31)" "\$(r)"
printf "  %smodified:   src/pane.rs%s\n"          "\$(c 31)" "\$(r)"
printf "Untracked files:\n"
printf "  %ssrc/render/blur.rs%s\n\n"             "\$(c 31)" "\$(r)"
p; printf "ls --color\n"
printf "%sCargo.toml%s   %sREADME.md%s   %sLICENSE%s   %ssrc%s/   %sassets%s/   %starget%s/\n\n" \
  "\$(r)" "\$(r)" "\$(r)" "\$(r)" "\$(r)" "\$(r)" "\$(c '1;34')" "\$(r)" "\$(c '1;34')" "\$(r)" "\$(c '1;34')" "\$(r)"
p; printf "bat src/render/scroll.rs\n"
printf "%s   1%s %s//%s smooth output easing: nudge the visual offset, never snap\n" "\$(c 90)" "\$(r)" "\$(c 90)" "\$(r)"
printf "%s   2%s %spub fn%s %snudge_output%s(&%smut%s self, grew: %su32%s) {\n" "\$(c 90)" "\$(r)" "\$(c 35)" "\$(r)" "\$(c '1;33')" "\$(r)" "\$(c 35)" "\$(r)" "\$(c 36)" "\$(r)"
printf "%s   3%s     %sself%s.backlog = (%sself%s.backlog + grew).min(%sMAX_BACKLOG%s);\n" "\$(c 90)" "\$(r)" "\$(c 35)" "\$(r)" "\$(c 35)" "\$(r)" "\$(c 36)" "\$(r)"
printf "%s   4%s     %sself%s.target = %s0.0%s;\n" "\$(c 90)" "\$(r)" "\$(c 35)" "\$(r)" "\$(c '1;36')" "\$(r)"
printf "%s   5%s }\n\n" "\$(c 90)" "\$(r)"
p; printf "cargo test\n"
printf "   %sCompiling%s silkterm v1.0.0-beta1\n" "\$(c '1;32')" "\$(r)"
printf "    %sFinished%s test [unoptimized + debuginfo] in 8.44s\n" "\$(c '1;32')" "\$(r)"
printf "     %sRunning%s unittests src/main.rs\n\n" "\$(c '1;32')" "\$(r)"
printf "running 86 tests\n"
printf "%stest%s scroll::eases_to_rest ... %sok%s\n" "\$(r)" "\$(r)" "\$(c '1;32')" "\$(r)"
printf "%stest%s pane::split_thirds ... %sok%s\n"    "\$(r)" "\$(r)" "\$(c '1;32')" "\$(r)"
printf "%stest%s glow::border_dilates ... %sok%s\n\n" "\$(r)" "\$(r)" "\$(c '1;32')" "\$(r)"
printf "test result: %sok%s. 86 passed; 0 failed; 0 ignored\n\n" "\$(c '1;32')" "\$(r)"
p; printf "git log --oneline -4\n"
printf "%s1a8bc3f%s pane: equalize a same-dir split run\n" "\$(c 33)" "\$(r)"
printf "%sc894ebf%s scroll: ramp ease speed on burst\n"    "\$(c 33)" "\$(r)"
printf "%se25498e%s glow: dilate outline before composite\n" "\$(c 33)" "\$(r)"
printf "%s7c137bc%s themes: SilkTerm / Matrix / Amber\n\n"  "\$(c 33)" "\$(r)"
p; printf "du -sh target/release/silkterm\n"
printf "%s12M%s\ttarget/release/silkterm\n\n" "\$(c '1;32')" "\$(r)"
p; printf "cargo run --release\n"
printf "    %sFinished%s release [optimized] in 0.19s\n" "\$(c '1;32')" "\$(r)"
printf "     %sRunning%s target/release/silkterm\n\n" "\$(c '1;32')" "\$(r)"
p
exec sleep 3600
EOF

	## --- split panes: three distinct panes ---------------------------------
	cat > "${workDir}/split-edit.sh" <<EOF
${sceneHead}
printf "%s src/pane.rs %s\n" "\$(c '7;36')" "\$(r)"
i=1
prn(){ printf "%s%3d%s %s\n" "\$(c 90)" "\$i" "\$(r)" "\$1"; i=\$((i+1)); }
prn "$(printf '%s' 'use crate::grid::Grid;')"
prn "$(printf '%s' 'use crate::scroll::Scroll;')"
prn ""
prn "$(printf '%s' 'pub struct Pane {')"
prn "$(printf '%s' '    grid:   Grid,')"
prn "$(printf '%s' '    scroll: Scroll,')"
prn "$(printf '%s' '    ratio:  f32,')"
prn "$(printf '%s' '}')"
prn ""
prn "$(printf '%s' 'impl Pane {')"
prn "$(printf '%s' '    pub fn split(&mut self, dir: Dir) -> PaneId {')"
prn "$(printf '%s' '        let id = alloc_pane_id();')"
prn "$(printf '%s' '        self.equalize_run(id, dir);')"
prn "$(printf '%s' '        id')"
prn "$(printf '%s' '    }')"
prn "$(printf '%s' '}')"
exec sleep 3600
EOF
	cat > "${workDir}/split-build.sh" <<EOF
${sceneHead}
printf "%s~/projects/webapp%s\$ cargo build --release\n" "\$(c '1;34')" "\$(r)"
printf "   %sCompiling%s libc v0.2.169\n" "\$(c '1;32')" "\$(r)"
printf "   %sCompiling%s winit v0.30.5\n" "\$(c '1;32')" "\$(r)"
printf "   %sCompiling%s wgpu v29.0.0\n" "\$(c '1;32')" "\$(r)"
printf "%swarning%s: unused import: 'std::fmt'\n" "\$(c '1;33')" "\$(r)"
printf "  %s-->%s src/text.rs:4:5\n" "\$(c '1;34')" "\$(r)"
printf "   %sCompiling%s silkterm v1.0.0-beta1\n" "\$(c '1;32')" "\$(r)"
printf "    %sFinished%s release [optimized] in 1m 58s\n" "\$(c '1;32')" "\$(r)"
printf "%s~/projects/webapp%s\$ " "\$(c '1;34')" "\$(r)"
exec sleep 3600
EOF
	cat > "${workDir}/split-log.sh" <<EOF
${sceneHead}
g(){ printf "%s%s%s " "\$(c '1;33')" "\$1" "\$(r)"; }
printf "%s*%s %s(%sHEAD -> %sfeature/smooth-scroll%s%s)%s smooth output easing\n" "\$(c 31)" "\$(r)" "\$(c 33)" "\$(c '1;36')" "\$(c '1;32')" "\$(r)" "\$(c 33)" "\$(r)"; g 4a1c8f0
printf "%s*%s glow: dilate outline before composite\n" "\$(c 31)" "\$(r)"; g 9d2e7b1
printf "%s*%s pane: equalize a same-dir split run\n" "\$(c 31)" "\$(r)"; g 1a8bc3f
printf "%s*%s scroll: ramp ease speed on burst\n" "\$(c 31)" "\$(r)"; g c894ebf
printf "%s*%s config: reorder to template on load\n" "\$(c 31)" "\$(r)"; g e25498e
printf "%s*%s themes: SilkTerm / Matrix / Amber\n" "\$(c 31)" "\$(r)"; g 7c137bc
exec sleep 3600
EOF

	## --- glow / transparency / bg-image flair (neofetch-ish) ---------------
	cat > "${workDir}/glow.sh" <<EOF
${sceneHead}
k(){ printf "%s%-9s%s %s\n" "\$(c '1;36')" "\$1" "\$(r)" "\$2"; }
printf "\n"
printf "        %s.--.%s        %sSilkTerm%s\n"     "\$(c '1;35')" "\$(r)" "\$(c '1;35')" "\$(r)"
printf "       %s/ .. \\\\%s       %s--------%s\n"  "\$(c '1;35')" "\$(r)" "\$(c 90)" "\$(r)"
printf "      %s| (--) |%s      " "\$(c '1;35')" "\$(r)"; k "OS"      "Linux x86_64"
printf "       %s\\\\ '' /%s       " "\$(c '1;35')" "\$(r)"; k "WM"      "Compiz"
printf "        %s'--'%s        " "\$(c '1;35')" "\$(r)"; k "Shell"   "bash 5.2"
printf "      %s smooth %s      " "\$(c '1;36')" "\$(r)"; k "Term"    "SilkTerm 1.0"
printf "      %s scroll %s      " "\$(c '1;36')" "\$(r)"; k "Theme"   "SilkTerm dark"
printf "                    "; k "GPU"     "hardware accelerated"
printf "\n   "
for i in 1 2 3 4 5 6; do printf "%s   %s" "\$(c "4\$i")" "\$(r)"; done
printf "  "
for i in 1 2 3 4 5 6; do printf "%s   %s" "\$(c "10\$i")" "\$(r)"; done
printf "\n"
exec sleep 3600
EOF

	## --- tabs + 256 colour + unicode/powerline showcase --------------------
	cat > "${workDir}/showcase.sh" <<EOF
${sceneHead}
printf "%s 256-colour + Unicode %s\n\n" "\$(c '7;35')" "\$(r)"
row=0
for base in 16 52 88 124 160 196; do
  printf "  "
  n=\$base
  while [ \$n -lt \$((base+36)) ]; do printf "%s  %s" "\$(printf "%s[48;5;%dm" "\$e" "\$n")" "\$(r)"; n=\$((n+1)); done
  printf "\n"
done
printf "\n  Box:  %s+------+------+%s   Powerline: %s%s%s master %s%s%s ~/src %s%s\n" \
  "\$(c 36)" "\$(r)" "\$(c '38;5;235;48;5;114')" "" "" "\$(c '38;5;114;48;5;240')" "" "\$(c '38;5;250;48;5;240')" "\$(c '38;5;240')" "\$(r)"
printf "  CJK:  %s\344\275\240\345\245\275%s   Emoji: \360\237\232\200 \342\255\220 \342\234\250 \360\237\224\245   Math: %s\342\210\221 \342\210\253 \342\210\232 \317\200%s\n" \
  "\$(c '1;33')" "\$(r)" "\$(c '1;32')" "\$(r)"
printf "  RTL:  %s\330\247\331\204\330\263\331\204\330\247\331\205%s   Braille: \342\240\213\342\240\231\342\240\271   Arrows: \342\206\220 \342\206\221 \342\206\222 \342\206\223 \342\236\241\n\n" \
  "\$(c '1;36')" "\$(r)"
printf "  24-bit gradient:\n  "
n=0
while [ \$n -lt 76 ]; do
  rr=\$((40 + n*2)); gg=\$((180 - n)); bb=\$((120 + n))
  printf "%s \\033[0m" "\$(printf "%s[48;2;%d;%d;%dm" "\$e" "\$rr" "\$gg" "\$bb")"
  n=\$((n+1))
done
printf "\n\n"
printf "  %s%-14s%s%-10s%s%s%s\n" "\$(c '7;34')" " Package" " Version" " Status " "\$(r)" "" ""
printf "   %-13s %-9s %sok%s\n"  "wgpu"        "29.0.0"  "\$(c '1;32')" "\$(r)"
printf "   %-13s %-9s %sok%s\n"  "winit"       "0.30.5"  "\$(c '1;32')" "\$(r)"
printf "   %-13s %-9s %sok%s\n"  "cosmic-text" "0.18.2"  "\$(c '1;32')" "\$(r)"
printf "   %-13s %-9s %sbeta%s\n" "silkterm"   "1.0.0"   "\$(c '1;33')" "\$(r)"
exec sleep 3600
EOF
	cat > "${workDir}/tab-logs.sh" <<EOF
${sceneHead}
printf "%s[info]%s  server listening on :8080\n" "\$(c '1;32')" "\$(r)"
printf "%s[warn]%s  slow query 214ms\n" "\$(c '1;33')" "\$(r)"
printf "%s[info]%s  200 GET /api/health\n" "\$(c '1;32')" "\$(r)"
exec sleep 3600
EOF

	chmod +x "${workDir}"/*.sh
}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Capture core

## Launch silkterm on the private display; record its PID for a targeted kill.
## Args: configFile scaleFactor -- <silkterm cli args...>
fLaunch() {
	local -r cfg="$1" scale="$2"; shift 3   ## drop cfg, scale, and the '--'
	DISPLAY="$display" XAUTHORITY="$authFile" LIBGL_ALWAYS_SOFTWARE=1 SHELL=/bin/dash \
		SILK_DUMP=1 WINIT_X11_SCALE_FACTOR="$scale" \
		"$binPath" --fullscreen --config "$cfg" "$@" \
		>"${workDir}/silk.log" 2>&1 &
	local -ri pid=$!
	launchedPids+=("$pid")
	printf '%s' "$pid"
}

## Wait for the GL offscreen dump to be produced and for scene output to settle.
fWaitDump() {
	local i sz
	rm -f /tmp/silk_offscreen.png
	for i in $(seq 1 30); do
		if [[ -s /tmp/silk_offscreen.png ]]; then
			sz=$(stat -c%s /tmp/silk_offscreen.png 2>/dev/null || echo 0)
			[[ "$sz" -gt 60000 ]] && { sleep 3; return 0; }   ## let the PTY output paint + settle
		fi
		sleep 1
	done
	return 1
}

## Software GL (llvmpipe, multithreaded) isn't byte-deterministic frame-to-frame,
## so a straight overwrite would churn the committed PNGs on every cicd run even
## with nothing changed. Keep the existing file unless the new render differs
## VISUALLY (RMSE over a small threshold) - sub-pixel jitter stays, real visual
## changes land.
fEmitOne() {
	local -r dst="$1" geom="$2" raw="$3"
	local -r tmp="${workDir}/emit.png"
	magick "$raw" -filter Lanczos -resize "$geom" -background '#0c0c10' -flatten "$tmp"
	if [[ -f "$dst" ]] && fVisuallySame "$tmp" "$dst"; then
		rm -f "$tmp"          ## within jitter threshold - leave the committed file be
	else
		mv -f "$tmp" "$dst"
	fi
}

## 0 if two same-size PNGs are within the jitter threshold (normalised RMSE).
fVisuallySame() {
	local out norm
	out="$(magick compare -metric RMSE "$1" "$2" null: 2>&1 || true)"
	norm="$(printf '%s' "$out" | sed -n 's/.*(\([0-9.]*\)).*/\1/p')"
	[[ -n "$norm" ]] || return 1   ## unparseable (e.g. size mismatch) -> treat as changed
	## 0.05 clears llvmpipe's threaded sub-pixel jitter (worst on the dense colour
	## palette) while any *significant* visual change lands well above it.
	awk -v v="$norm" 'BEGIN { exit !(v + 0 < 0.05) }'
}

## Downsample a raw 16:9 dump into the repo (1920x1080 original + 640x360 thumb).
fEmit() {
	local -r name="$1" raw="$2"
	fEmitOne "${outLarge}/${name}.png" 1920x1080 "$raw"
	fEmitOne "${outThumb}/${name}.png" 640x360   "$raw"
	fInfo "wrote ${name}: $(identify -format '%wx%h' "${outLarge}/${name}.png") + 640x360 thumb"
}

## Copy a stable frame out of the live dump. SILK_DUMP rewrites the file every
## frame, so a plain cp can catch a half-written PNG - retry until it decodes.
fGrabStable() {
	local -r dst="$1"; local i
	for i in $(seq 1 12); do
		cp -f /tmp/silk_offscreen.png "$dst" 2>/dev/null || true
		identify "$dst" >/dev/null 2>&1 && return 0
		sleep 0.4
	done
	return 1
}

## A whole main-window scene: launch, wait, copy the dump, kill, downsample.
## Args: name configFile -- <cli args...>
fMainShot() {
	local -r name="$1" cfg="$2"; shift 3
	fStep "Scene ${name}"
	local pid; pid="$(fLaunch "$cfg" 1 -- "$@")"
	if fWaitDump && fGrabStable "${workDir}/${name}.raw.png"; then
		kill "$pid" 2>/dev/null || true
		fEmit "$name" "${workDir}/${name}.raw.png"
	else
		kill "$pid" 2>/dev/null || true
		fWarn "scene ${name}: no valid dump produced (see ${workDir}/silk.log)"
		return 1
	fi
}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Scenes

fSceneShell() {
	local -r cfg="${workDir}/shell.toml"
	fWriteConfig "$cfg" 'text_glow = true' 'text_glow_radius = 3.0' 'text_glow_softness = 0.6' 'text_outline = 1.5'
	fMainShot "01-shell" "$cfg" -- --shell "${workDir}/shell.sh"
}

fSceneSplits() {
	local -r cfg="${workDir}/splits.toml"
	fWriteConfig "$cfg" 'text_glow = true' 'text_glow_radius = 2.5' 'text_outline = 1.5'
	fMainShot "02-splits" "$cfg" -- \
		--font-size 18 \
		--shell "${workDir}/split-edit.sh" \
		--new-pane=b --right --size=44% --shell "${workDir}/split-build.sh" \
		--new-pane --splits=b --down --size=46% --shell "${workDir}/split-log.sh"
}

fSceneGlow() {
	local -r cfg="${workDir}/glow.toml"
	local -r bg="${workDir}/bg.png"
	fMakeBgImage "$bg"
	fWriteConfig "$cfg" \
		'transparent_background = true' 'opacity = 0.78' \
		"background_image = \"${bg}\"" 'background_opacity = 0.55' \
		'background_fit = "zoom"' 'background_blur = 6.0' \
		'text_glow = true' 'text_glow_radius = 5.0' 'text_glow_softness = 0.35' 'text_outline = 2.0'
	fMainShot "03-glow" "$cfg" -- --font-size 26 --hide-menu=yes --shell "${workDir}/glow.sh"
}

fSceneTabs() {
	local -r cfg="${workDir}/tabs.toml"
	fWriteConfig "$cfg" 'text_glow = true' 'text_glow_radius = 3.0' 'text_outline = 1.5'
	## main tab (0) runs the showcase; two more tabs make the tab bar visible;
	## re-select tab 0 so the showcase is the active/front tab in the shot.
	fMainShot "04-tabs" "$cfg" -- \
		--shell "${workDir}/showcase.sh" \
		--new-tab=logs --shell "${workDir}/tab-logs.sh" \
		--new-tab=build --shell "${workDir}/split-build.sh" \
		--tab=0
}

fSceneSettings() {
	fStep "Scene 05-settings"
	local -r cfg="${workDir}/settings.toml"
	fWriteConfig "$cfg" 'text_glow = true' 'text_glow_radius = 3.0'
	## render at 2x so the (small) dialog is crisp and large enough to fill a shot.
	local pid; pid="$(fLaunch "$cfg" 2 -- --shell "${workDir}/shell.sh")"
	fWaitDump || true   ## ensures the main window is up before we poke it
	## open Settings (Ctrl+comma is flaky here) - retry until the dialog appears.
	local dlg="" tries
	for tries in $(seq 1 20); do
		DISPLAY="$display" XAUTHORITY="$authFile" xdotool key --clearmodifiers ctrl+comma 2>/dev/null || true
		sleep 0.6
		dlg="$(DISPLAY="$display" XAUTHORITY="$authFile" xdotool search --name 'Settings' 2>/dev/null | head -1 || true)"
		[[ -n "$dlg" ]] && break
	done
	if [[ -z "$dlg" ]]; then
		kill "$pid" 2>/dev/null || true
		fWarn "settings dialog never opened; skipping 05-settings"
		return 1
	fi
	## nudge a redraw (dialog only paints on input), then grab just the dialog.
	DISPLAY="$display" XAUTHORITY="$authFile" xdotool key --window "$dlg" shift 2>/dev/null || true
	sleep 1
	local -r raw="${workDir}/05-settings.raw.png"
	DISPLAY="$display" XAUTHORITY="$authFile" import -window "$dlg" "$raw"
	kill "$pid" 2>/dev/null || true
	## letterbox the dialog (portrait-ish) onto a 16:9 canvas so the grid is uniform.
	local -r canvas="${workDir}/05-settings.png"
	## +repage: import bakes the window's screen offset into the page geometry,
	## which -extent would otherwise honour and shift the dialog off-centre.
	magick "$raw" +repage -filter Lanczos -resize '1560x1000>' -background '#0c0c10' \
		-gravity center -extent 2560x1440 "$canvas"
	fEmit "05-settings" "$canvas"
}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Entry

fMain() {
	fResolvePaths "$@"
	fStep "SilkTerm screenshots"
	fInfo "repo:   $repoDir"
	fInfo "binary: $binPath"
	fInfo "out:    ${outThumb} (+ large/)"
	fWriteScenes
	fStartHeadless

	fSceneShell    || fWarn "01-shell failed"
	fSceneSplits   || fWarn "02-splits failed"
	fSceneGlow     || fWarn "03-glow failed"
	fSceneTabs     || fWarn "04-tabs failed"
	fSceneSettings || fWarn "05-settings failed"

	fStep "Done"
	fInfo "screenshots in ${outThumb} and ${outLarge}"
	ls -1 "$outThumb"/*.png 2>/dev/null | sed 's/^/  /' || true
}


##	Check if sourced (not meant to be); else run.
if ! (return 0 2>/dev/null); then
	fMain "$@"
fi


##	Script history:
##		- 20260704 JC: Created.
