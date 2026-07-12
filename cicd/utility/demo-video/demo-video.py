#!/usr/bin/env python3

##	Purpose:
##		Record the SilkTerm demo video: drives a real SilkTerm on a private Xvfb
##		(never :0), types at a realistic pace (variable wpm, occasional fixed typos),
##		captures 1920x1080@60 with ffmpeg, lays down keyboard/mouse foley synced
##		to the actual input timestamps, overlays per-segment banners, and encodes
##		an mp4 plus a half-size gif. Outputs GFS-rotate into private/demo-video/;
##		the newest gif is copied to github/assets/ for the README.
##	Syntax:
##		demo-video.py [--segments a,b,...] [--seed N] [--keep-work] [--no-gif]
##		              [--display :98] [--out-dir DIR]
##		Env: SILK_BIN overrides the binary (default REPO/target/release/silkterm).
##	Notes:
##		AV sync needs no calibration: before the app launches, the bare root is
##		flashed white (xsetroot) at a recorded wall-clock time; the bright frame
##		is found in the capture afterwards, anchoring every event epoch to video
##		time exactly. Sound assets + licenses live in ./sounds/ (see LICENSES.txt).
##	History: at bottom.

##	Copyright © 2026 Jim Collier
##	Licensed under The MIT License (MIT). Full text at:
##		https://mit-license.org/
##	SPDX-License-Identifier: MIT

import argparse
import os
import random
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import wave
from pathlib import Path

import numpy as np

ME_DIR   = Path(__file__).resolve().parent
REPO     = ME_DIR.parents[2]                  # github/cicd/utility/demo-video -> github
PRIVATE  = REPO.parent / "private" / "demo-video"
SOUNDS   = ME_DIR / "sounds"

FPS        = 60
SIZE       = (1920, 1080)
SR         = 48000                            # audio mix rate
FONT_STACK = "Monaspace Argon NF Medium, Monaspace Argon NF, DejaVu Sans Mono"
FONT_SIZE  = 24                               # ~33% over a normal 18px-at-1080p
BANNER_TTF = "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf"
BANNER_MIN = 4.0                              # every banner stays up at least this long
LEAD_S     = 0.8                              # quiet lead-in kept before the first segment

def log(msg):
	print(f"[demo] {msg}", flush=True)

def run(cmd, **kw):
	return subprocess.run(cmd, check=True, **kw)

def out_of(cmd):
	return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Recorder: display/app/capture lifecycle + the event/banner logs

class Rec:
	def __init__(self, args):
		self.display  = args.display
		self.num      = self.display.lstrip(":")
		self.auth     = f"/tmp/cicd-gui-headless-{os.environ['USER']}/Xauthority-{self.num}"
		self.bin      = os.environ.get("SILK_BIN", str(REPO / "target/release/silkterm"))
		self.work     = Path(tempfile.mkdtemp(prefix="silk-demo-"))
		self.keep     = args.keep_work
		self.events   = []      # (epoch, kind) kind: key:NAME / rel:NAME / mouse:NAME
		self.banners  = []      # (epoch_start, epoch_end, text)
		self.app      = None
		self.ff       = None
		self.flash_e  = 0.0     # wall-clock epoch of the white sync flash
		self.t0_e     = 0.0     # wall-clock epoch where trimmed content starts
		self.started_x = False

	def env(self):
		e = dict(os.environ)
		e.update(DISPLAY=self.display, XAUTHORITY=self.auth, LIBGL_ALWAYS_SOFTWARE="1")
		return e

	def xdo(self, *a):
		subprocess.run(["xdotool", *a], env=self.env(), check=False,
			stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

	def start_display(self):
		gh = str(REPO / "cicd/utility/gui-headless.bash")
		e = dict(os.environ, CICD_HEADLESS_DISPLAY=self.display,
			CICD_HEADLESS_SIZE=f"{SIZE[0]}x{SIZE[1]}x24")
		st = subprocess.run([gh, "status"], env=e, capture_output=True, text=True)
		if "up on" not in st.stdout:
			run([gh, "start", "--wm"], env=e)
			self.started_x = True
		subprocess.run(["xsetroot", "-solid", "black"], env=self.env(), check=False)

	def stop_display(self):
		if self.started_x:
			gh = str(REPO / "cicd/utility/gui-headless.bash")
			e = dict(os.environ, CICD_HEADLESS_DISPLAY=self.display)
			subprocess.run([gh, "stop"], env=e, capture_output=True)

	def start_capture(self):
		self.raw = self.work / "raw.mkv"
		self.ff = subprocess.Popen([
			"ffmpeg", "-hide_banner", "-loglevel", "error",
			"-f", "x11grab", "-framerate", str(FPS),
			"-video_size", f"{SIZE[0]}x{SIZE[1]}", "-i", self.display,
			"-c:v", "libx264", "-preset", "ultrafast", "-crf", "15",
			"-pix_fmt", "yuv420p", str(self.raw)],
			env=self.env(), stdin=subprocess.DEVNULL)
		time.sleep(1.5)                          # let x11grab settle before the sync flash
		subprocess.run(["xsetroot", "-solid", "white"], env=self.env(), check=False)
		self.flash_e = time.time()
		time.sleep(0.25)
		subprocess.run(["xsetroot", "-solid", "black"], env=self.env(), check=False)
		time.sleep(0.4)

	def stop_capture(self):
		if self.ff:
			self.ff.send_signal(signal.SIGINT)
			try:
				self.ff.wait(timeout=15)
			except subprocess.TimeoutExpired:
				self.ff.kill()
			self.ff = None

	def launch_app(self, cfg, shell_cmd, cwd=None, extra_env=None):
		e = self.env()
		# --norc/--noprofile below skips even Debian's /etc/bash.bashrc (a --rcfile
		# shell still reads it, and this box's spews real paths); the prompt comes
		# in via the environment instead. Gray prompt, rose user, sand host.
		e.update(SHELL="/bin/dash", HOME=str(self.work / "home"),
			PS1="\\[\\e[38;2;224;144;158m\\]juno\\[\\e[38;2;150;156;162m\\]@"
				"\\[\\e[38;2;222;178;134m\\]vela\\[\\e[38;2;150;156;162m\\]:\\w\\$ \\[\\e[0m\\]",
			HISTFILE="/dev/null")
		if extra_env:
			e.update(extra_env)
		self.app = subprocess.Popen(
			[self.bin, "--fullscreen", "--config", str(cfg), "--shell", shell_cmd],
			env=e, cwd=cwd, stdout=open(self.work / "silk.log", "w"), stderr=subprocess.STDOUT)
		# ready = window exists AND something painted (llvmpipe compiles pipelines first)
		deadline = time.time() + 60
		win = ""
		while time.time() < deadline and not win:
			r = subprocess.run(["xdotool", "search", "--class", "silkterm"],
				env=self.env(), capture_output=True, text=True)
			win = r.stdout.split()[0] if r.stdout.strip() else ""
			time.sleep(0.5)
		if not win:
			raise RuntimeError("silkterm window never appeared (see silk.log)")
		self.win = win
		while time.time() < deadline:
			shot = self.work / "probe.png"
			subprocess.run(["import", "-window", "root", str(shot)],
				env=self.env(), check=False, capture_output=True)
			try:
				mean = float(out_of(["magick", str(shot), "-format", "%[fx:mean]", "info:"]))
			except Exception:
				mean = 0.0
			if mean > 0.002:
				break
			time.sleep(0.8)
		self.xdo("windowactivate", win)
		time.sleep(0.3)
		self.mouse_park()

	def kill_app(self):
		if self.app:
			self.app.terminate()
			try:
				self.app.wait(timeout=5)
			except subprocess.TimeoutExpired:
				self.app.kill()
			self.app = None

	# --- event log -------------------------------------------------------------
	def ev(self, kind):
		self.events.append((time.time(), kind))

	def mouse_park(self):
		self.xdo("mousemove", str(SIZE[0] - 4), str(SIZE[1] - 4))

	def cleanup(self):
		self.stop_capture()
		self.kill_app()
		self.stop_display()
		if not self.keep and self.work.exists():
			shutil.rmtree(self.work, ignore_errors=True)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Typing engine

# qwerty neighbours for plausible typos
NEIGH = {
	"a": "sq", "b": "vn", "c": "xv", "d": "sf", "e": "wr", "f": "dg", "g": "fh",
	"h": "gj", "i": "uo", "j": "hk", "k": "jl", "l": "k", "m": "n", "n": "bm",
	"o": "ip", "p": "o", "q": "wa", "r": "et", "s": "ad", "t": "ry", "u": "yi",
	"v": "cb", "w": "qe", "x": "zc", "y": "tu", "z": "x",
}
# keyboard row of a char -> which GENERIC_R* sample it thocks with
ROW1 = set("1234567890-=!@#$%^&*()_+")
ROW2 = set("qwertyuiop[]{}")
ROW3 = set("asdfghjkl;:'\"")
ROW4 = set("zxcvbnm,./<>?")

def key_sound(ch):
	c = ch.lower()
	if c in ROW2: return "key:GENERIC_R2"
	if c in ROW3: return "key:GENERIC_R3"
	if c in ROW4: return "key:GENERIC_R4"
	if c in ROW1: return "key:GENERIC_R1"
	return "key:GENERIC_R0"

class Typist:
	def __init__(self, rec, rng):
		self.rec = rec
		self.rng = rng
		self.wpm = rng.uniform(120, 160)

	def _delay(self):
		# per-char delay from current wpm, lognormal jitter; wpm drifts as it would
		self.wpm += self.rng.uniform(-8, 8)
		self.wpm = max(100.0, min(200.0, self.wpm))
		d = 12.0 / self.wpm                      # 60 / (5 * wpm)
		return d * self.rng.lognormvariate(0.0, 0.22)

	def _emit(self, ch):
		self.rec.ev(key_sound(ch) if ch != " " else "key:SPACE")
		if ch == " ":
			self.rec.xdo("key", "--clearmodifiers", "space")
		else:
			subprocess.run(["xdotool", "type", "--delay", "0", "--", ch],
				env=self.rec.env(), check=False,
				stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
		self.rec.events.append((time.time() + self.rng.uniform(0.05, 0.09), "rel:GENERIC"))

	def _backspace(self, n):
		for _ in range(n):
			time.sleep(self.rng.uniform(0.09, 0.16))
			self.rec.ev("key:BACKSPACE")
			self.rec.xdo("key", "--clearmodifiers", "BackSpace")

	def type(self, text, typos=0.018):
		i = 0
		while i < len(text):
			ch = text[i]
			time.sleep(self._delay() * (1.6 if ch == " " else 1.0))
			# an expert's slip: wrong neighbour, maybe one more char, catch it, fix it
			if ch.lower() in NEIGH and self.rng.random() < typos:
				wrong = self.rng.choice(NEIGH[ch.lower()])
				self._emit(wrong)
				extra = 0
				if self.rng.random() < 0.4 and i + 1 < len(text) and text[i + 1] != " ":
					time.sleep(self._delay())
					self._emit(text[i + 1])
					extra = 1
				time.sleep(self.rng.uniform(0.22, 0.45))   # the "oops" beat
				self._backspace(1 + extra)
				time.sleep(self.rng.uniform(0.08, 0.2))
				self._emit(ch)
				if extra:
					time.sleep(self._delay())
					self._emit(text[i + 1])
				i += 1 + extra
				continue
			self._emit(ch)
			i += 1

	def enter(self):
		time.sleep(self.rng.uniform(0.15, 0.4))
		self.rec.ev("key:ENTER")
		self.rec.xdo("key", "--clearmodifiers", "Return")
		self.rec.events.append((time.time() + 0.07, "rel:ENTER"))

	def key(self, keysym, sound="key:GENERIC_R0", hold=None):
		self.rec.ev(sound)
		self.rec.xdo("key", "--clearmodifiers", keysym)

	def keys(self, keysym, n, hz=8.0, sound="key:GENERIC_R0"):
		# repeated taps (arrow scrolling); slight cadence wobble
		for _ in range(n):
			self.key(keysym, sound)
			time.sleep(max(0.03, self.rng.uniform(0.8, 1.2) / hz))

	def cmd(self, text, settle=1.0, typos=0.018):
		self.type(text, typos)
		self.enter()
		time.sleep(settle)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Mouse

class Mouse:
	def __init__(self, rec, rng):
		self.rec = rec
		self.rng = rng
		self.pos = (SIZE[0] - 4, SIZE[1] - 4)

	def move(self, x, y, dur=0.6):
		# eased path so it reads as a hand, not a teleport
		x0, y0 = self.pos
		steps = max(6, int(dur * 40))
		for i in range(1, steps + 1):
			t = i / steps
			t = t * t * (3 - 2 * t)              # smoothstep
			self.rec.xdo("mousemove", str(int(x0 + (x - x0) * t)), str(int(y0 + (y - y0) * t)))
			time.sleep(dur / steps)
		self.pos = (x, y)

	def click(self, quiet=False):
		self.rec.ev("mouse:CLICK_Q" if quiet else "mouse:CLICK")
		self.rec.xdo("click", "1")

	def double(self):
		self.rec.ev("mouse:CLICK")
		time.sleep(0.11)
		self.rec.ev("mouse:CLICK")
		self.rec.xdo("click", "--repeat", "2", "--delay", "110", "1")

	def drag(self, x1, y1, x2, y2, dur=0.9):
		self.move(x1, y1, 0.5)
		self.rec.ev("mouse:CLICK")
		self.rec.xdo("mousedown", "1")
		time.sleep(0.15)
		self.move(x2, y2, dur)
		time.sleep(0.1)
		self.rec.ev("mouse:CLICK_Q")
		self.rec.xdo("mouseup", "1")

	def park(self):
		self.rec.mouse_park()
		self.pos = (SIZE[0] - 4, SIZE[1] - 4)

	def wheel(self, up, n, hz=7.0):
		for _ in range(n):
			self.rec.ev("mouse:WHEEL")
			self.rec.xdo("click", "4" if up else "5")
			time.sleep(self.rng.uniform(0.8, 1.2) / hz)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Banner bookkeeping

class Banner:
	def __init__(self, rec, text, pos="bottom"):
		self.rec, self.text, self.pos = rec, text, pos

	def __enter__(self):
		self.start = time.time()
		return self

	def __exit__(self, *exc):
		self.rec.banners.append((self.start, time.time(), self.text, self.pos))


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Scene content: throwaway config, rc file, a synthetic project tree

def write_config(work):
	# copy the image next to the config and use the bare name: the Settings
	# dialog shows this value verbatim, and a real absolute path would leak
	shutil.copy2(REPO / "filesystem/home/.config/silkterm/backgrounds/background24.jpg",
		work / "background24.jpg")
	bg = "background24.jpg"
	cfg = work / "config.toml"
	cfg.write_text(f'''use_system_font = false
font_family = "{FONT_STACK}"
font_size = {FONT_SIZE}.0
remember_size = false
theme = "SilkTerm"
theme_mode = "dark"
background_image = "{bg}"
background_opacity = 0.10
background_fit = "zoom"
background_blur = 10.0
text_scrim = true
text_scrim_radius = 5.0
text_scrim_softness = 0.5
text_outline = 2.0
smooth_scroll_apps = true
scrollback = 10000

[colors]
background = "#000000"
foreground = "#88ffee"
''')
	return cfg

RUST_SCROLL = '''// smooth output easing: nudge the visual offset toward rest, never snap
use crate::grid::Grid;

pub struct Scroll {
	visual:  f64,
	target:  f64,
	backlog: u32,
	tau_ms:  f32,
}

impl Scroll {
	pub fn new(tau_ms: f32) -> Self {
		Self { visual: 0.0, target: 0.0, backlog: 0, tau_ms }
	}

	pub fn nudge_output(&mut self, grew: u32) {
		self.backlog = (self.backlog + grew).min(MAX_BACKLOG);
		self.target = 0.0;
	}

	pub fn step(&mut self, dt_ms: f32) -> bool {
		let tau = self.effective_tau(dt_ms);
		let k = 1.0 - (-dt_ms / tau).exp();
		self.visual += (self.target - self.visual) * k as f64;
		(self.visual - self.target).abs() > SETTLE_EPS
	}

	fn effective_tau(&self, dt_ms: f32) -> f32 {
		// a burst ramps the ease speed so the view keeps up, then relaxes
		let load = self.backlog as f32 / MAX_BACKLOG as f32;
		self.tau_ms * (1.0 - 0.8 * load.min(1.0))
	}
}
'''

def write_tree(work, rng):
	home = work / "home"
	proj = home / "projects" / "pulsar"
	src = proj / "src"
	src.mkdir(parents=True)
	(proj / "docs").mkdir()
	(proj / "assets").mkdir()
	(proj / "Cargo.toml").write_text(
		'[package]\nname = "pulsar"\nversion = "0.4.1"\nedition = "2024"\n')
	(proj / "README.md").write_text("# pulsar\n\nA tiny GPU particle toy.\n")
	(proj / "LICENSE").write_text("MIT\n")
	body = RUST_SCROLL * 5                       # long enough that nano has to scroll
	(src / "scroll.rs").write_text(body)
	(src / "main.rs").write_text('fn main() {\n\tpulsar::run();\n}\n')
	(src / "render.rs").write_text(RUST_SCROLL)

	# build.sh: cargo-flavoured output with varied pacing and burst sizes
	crates = ["proc-macro2", "quote", "syn", "libc", "bitflags", "smallvec",
		"cfg-if", "log", "parking_lot", "raw-window-handle", "wayland-client",
		"x11-dl", "ash", "naga", "wgpu-hal", "wgpu-core", "wgpu", "winit",
		"glam", "bytemuck", "pollster", "image", "rayon", "pulsar"]
	lines = ["#!/bin/dash", 'g="\\033[1;32m"; y="\\033[1;33m"; b="\\033[1;34m"; r="\\033[0m"']
	lines.append('printf "   ${g}Compiling${r} pulsar workspace\\n"')
	for i, c in enumerate(crates):
		v = f"{rng.randint(0,3)}.{rng.randint(1,30)}.{rng.randint(0,9)}"
		lines.append(f'printf "   ${{g}}Compiling${{r}} {c} v{v}\\n"')
		if rng.random() < 0.35:
			lines.append(f"sleep 0.{rng.randint(15, 45):02d}")
	lines += [
		'printf "${y}warning${r}: unused variable: ${b}lift${r}\\n"',
		'printf "  ${b}-->${r} src/render.rs:141:9\\n"',
		'sleep 0.4',
		'printf "   ${g}Compiling${r} pulsar v0.4.1\\n"',
		'sleep 0.9',
		'printf "    ${g}Finished${r} release [optimized] in 12.31s\\n"',
	]
	sh = proj / "build.sh"
	sh.write_text("\n".join(lines) + "\n")
	sh.chmod(0o755)

	# test.sh: one dense instant burst (the fast-ramp case)
	tests = ["scroll::eases_to_rest", "scroll::burst_ramps", "grid::wraps_wide",
		"grid::resize_reflows", "render::scrim_corners", "render::srgb_once",
		"pane::split_thirds", "pane::equalize_run", "input::mod_arrows",
		"select::word_pairs", "select::block_mode", "theme::dark_light",
		"config::reorder_stable", "config::lenient_floats", "easing::tau_ramp",
		"easing::no_overshoot"] * 3
	lines = ["#!/bin/dash", 'g="\\033[1;32m"; r="\\033[0m"',
		'printf "    ${g}Finished${r} test [unoptimized] in 4.02s\\n"',
		f'printf "running {len(tests)} tests\\n"']
	for t in tests:
		lines.append(f'printf "test {t} ... ${{g}}ok${{r}}\\n"')
	lines.append(f'printf "\\ntest result: ${{g}}ok${{r}}. {len(tests)} passed; 0 failed\\n"')
	sh = proj / "test.sh"
	sh.write_text("\n".join(lines) + "\n")
	sh.chmod(0o755)

	# a long colourised log for the `less` scene
	lvl = [("32", "info"), ("33", "warn"), ("36", "dbug")]
	rows = []
	t = 91250.114
	for i in range(420):
		c, name = lvl[0] if rng.random() < 0.75 else rng.choice(lvl[1:])
		t += rng.uniform(0.002, 0.4)
		msg = rng.choice([
			"frame presented in %.1fms" % rng.uniform(0.8, 6.0),
			"atlas grew to %dx%d" % (512 * rng.randint(1, 4), 512 * rng.randint(1, 4)),
			"pipeline cache hit (%d entries)" % rng.randint(4, 96),
			"pty read %d bytes" % rng.randint(24, 4096),
			"ease settled after %dms" % rng.randint(80, 420),
			"resized grid to %dx%d" % (rng.randint(80, 200), rng.randint(24, 60)),
			"scrollback trimmed to %d lines" % rng.randint(5000, 10000),
		])
		rows.append(f"\033[90m{t:10.3f}\033[0m \033[{c}m[{name}]\033[0m {msg}")
	(proj / "docs" / "render.log").write_text("\n".join(rows) + "\n")
	return home


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Segments

def seg_intro(r, t, m):
	with Banner(r, "SilkTerm - a smooth-scrolling, GPU-accelerated terminal"):
		time.sleep(1.2)
		t.cmd("ls --color", settle=1.4)
		t.cmd("tree -C src docs", settle=1.6)
		time.sleep(1.0)

def seg_bursts(r, t, m):
	with Banner(r, "Output glides into place - never snaps - at any burst size"):
		t.cmd("./build.sh", settle=7.5)      # covers the script's own runtime
		time.sleep(1.2)
		t.cmd("./test.sh", settle=3.0)
		time.sleep(0.8)

def seg_wheel(r, t, m):
	with Banner(r, "Scrollback rides the wheel just as smoothly"):
		m.move(SIZE[0] // 2, SIZE[1] // 2, 0.7)
		m.wheel(up=True, n=14, hz=6.5)
		time.sleep(0.9)
		m.wheel(up=False, n=18, hz=9.0)
		m.park()
		time.sleep(0.8)

def seg_less(r, t, m):
	with Banner(r, "Full-screen apps slide too: less", pos="top"):
		t.cmd("less -R docs/render.log", settle=1.4)
		t.keys("Down", 16, hz=7.0)
		time.sleep(0.7)
		t.keys("Up", 8, hz=6.0)
		time.sleep(0.6)
		t.key("q")
		time.sleep(0.8)

def seg_nano(r, t, m):
	with Banner(r, "...and nano", pos="top"):
		t.cmd("nano src/scroll.rs", settle=1.6)
		t.keys("Down", 26, hz=9.0)
		time.sleep(0.8)
		t.keys("Up", 10, hz=7.0)
		time.sleep(0.6)
		t.key("ctrl+x")                      # nothing was modified, so this exits directly
		time.sleep(1.0)

def seg_mouse(r, t, m):
	with Banner(r, "Mouse selection: word, run, and block"):
		# fill the screen deterministically so the click targets always hold
		t.cmd("clear", settle=0.4, typos=0.0)
		t.cmd("tail -n 40 docs/render.log", settle=1.2)
		m.move(430, 820, 0.8)
		m.double()                           # word
		time.sleep(1.3)
		m.drag(140, 880, 1050, 880, 1.0)     # run
		time.sleep(1.2)
		r.xdo("keydown", "ctrl")             # block
		m.drag(180, 560, 720, 800, 1.2)
		r.xdo("keyup", "ctrl")
		time.sleep(1.4)
		m.move(1300, 460, 0.5)
		m.click(quiet=True)
		m.park()
		time.sleep(0.8)

def seg_settings(r, t, m):
	with Banner(r, "Live Settings - every change applies instantly"):
		r.ev("key:GENERIC_R0")
		r.xdo("key", "--clearmodifiers", "ctrl+comma")
		time.sleep(2.0)
		dlg = ""
		for _ in range(10):
			out = subprocess.run(["xdotool", "search", "--name", "Settings"],
				env=r.env(), capture_output=True, text=True).stdout.strip()
			if out:
				dlg = out.split()[0]
				break
			time.sleep(0.5)
		if dlg:
			# park it mid-right so the live change shows on the terminal behind it
			r.xdo("windowmove", dlg, "1180", "120")
			r.xdo("windowactivate", dlg)
			time.sleep(1.0)
			# transparency is off, so its greyed rows are skipped: three Tabs land
			# on the "Bg image opacity" slider track
			for _ in range(3):
				t.key("Tab", "key:GENERIC_R0")
				time.sleep(0.6)
			for _ in range(35):              # 0.10 -> ~0.80, arrow step is 0.02
				t.key("Right", "key:GENERIC_R0")
				time.sleep(0.11)
			# changes land on Apply; click it with the mouse (dialog sits at a
			# fixed spot, so the button is at a known point)
			m.move(1583, 727, 0.8)
			m.click()
			time.sleep(2.4)                  # linger on the brightened background
			for _ in range(35):
				t.key("Left", "key:GENERIC_R0")
				time.sleep(0.09)
			m.click()                        # mouse is still on Apply
			time.sleep(1.2)
			t.key("Escape")
			time.sleep(0.8)
			m.park()
		r.xdo("windowactivate", r.win)
		time.sleep(0.6)

def seg_outro(r, t, m):
	with Banner(r, "github.com/jim-collier/silkterm"):
		t.cmd("# smooth. silky. SilkTerm.", settle=0.5, typos=0.0)
		time.sleep(3.5)

SEGMENTS = [
	("intro",    seg_intro),
	("bursts",   seg_bursts),
	("wheel",    seg_wheel),
	("less",     seg_less),
	("nano",     seg_nano),
	("mouse",    seg_mouse),
	("settings", seg_settings),
	("outro",    seg_outro),
]


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Audio: mix the event log into a wav

SOUND_FILES = {
	"key:GENERIC_R0": SOUNDS / "keys/GENERIC_R0.mp3",
	"key:GENERIC_R1": SOUNDS / "keys/GENERIC_R1.mp3",
	"key:GENERIC_R2": SOUNDS / "keys/GENERIC_R2.mp3",
	"key:GENERIC_R3": SOUNDS / "keys/GENERIC_R3.mp3",
	"key:GENERIC_R4": SOUNDS / "keys/GENERIC_R4.mp3",
	"key:SPACE":      SOUNDS / "keys/SPACE.mp3",
	"key:ENTER":      SOUNDS / "keys/ENTER.mp3",
	"key:BACKSPACE":  SOUNDS / "keys/BACKSPACE.mp3",
	"rel:GENERIC":    SOUNDS / "keys/release/GENERIC.mp3",
	"rel:ENTER":      SOUNDS / "keys/release/ENTER.mp3",
	"mouse:CLICK":    SOUNDS / "mouse/click.wav",
	"mouse:CLICK_Q":  SOUNDS / "mouse/click_quiet.wav",
	"mouse:WHEEL":    SOUNDS / "mouse/click_quiet.wav",
}
GAIN = {"key": 0.85, "rel": 0.30, "mouse:CLICK": 0.55, "mouse:CLICK_Q": 0.40,
	"mouse:WHEEL": 0.10}

def load_samples(work):
	cache = {}
	for kind, path in SOUND_FILES.items():
		wav = work / (re.sub(r"[^A-Za-z0-9]", "_", kind) + ".wav")
		run(["ffmpeg", "-v", "error", "-y", "-i", str(path),
			"-ar", str(SR), "-ac", "2", "-f", "wav", str(wav)])
		with wave.open(str(wav), "rb") as w:
			data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
		cache[kind] = data.astype(np.float32).reshape(-1, 2) / 32768.0
	return cache

def build_audio(rec, work, duration, rng):
	cache = load_samples(work)
	mix = np.zeros((int(duration * SR) + SR, 2), dtype=np.float32)
	for epoch, kind in rec.events:
		t_rel = epoch - rec.t0_e
		if t_rel < -0.5 or t_rel > duration:
			continue
		s = cache.get(kind)
		if s is None:
			continue
		gain = GAIN.get(kind, GAIN.get(kind.split(":")[0], 0.8))
		gain *= rng.uniform(0.82, 1.0)
		# tiny per-hit pitch wobble keeps repeats from sounding stamped
		rate = rng.uniform(0.96, 1.05)
		n = int(len(s) / rate)
		idx = np.linspace(0, len(s) - 1, n)
		samp = np.stack([np.interp(idx, np.arange(len(s)), s[:, c]) for c in (0, 1)], axis=1)
		at = int(max(0.0, t_rel) * SR)
		end = min(at + len(samp), len(mix))
		mix[at:end] += samp[: end - at] * gain
	# level toward -8 dBFS peak so the foley reads clearly (bounded boost)
	peak = np.abs(mix).max()
	if peak > 0:
		mix *= min(0.40 / peak, 4.0)
	out = work / "audio.wav"
	with wave.open(str(out), "wb") as w:
		w.setnchannels(2)
		w.setsampwidth(2)
		w.setframerate(SR)
		w.writeframes((mix * 32767.0).astype(np.int16).tobytes())
	return out


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Post: sync-flash location, trim, banners, encode, gif

def find_flash(raw, work):
	# the white root flash is the brightest thing the capture will ever see
	stats = work / "stats.txt"
	run(["ffmpeg", "-v", "error", "-t", "6", "-i", str(raw),
		"-vf", f"signalstats,metadata=print:key=lavfi.signalstats.YAVG:file={stats}",
		"-f", "null", "-"])
	best_t, best_y = 0.0, -1.0
	pts = 0.0
	for line in stats.read_text().splitlines():
		mo = re.search(r"pts_time:([0-9.]+)", line)
		if mo:
			pts = float(mo.group(1))
		mo = re.search(r"YAVG=([0-9.]+)", line)
		if mo and float(mo.group(1)) > best_y:
			best_y, best_t = float(mo.group(1)), pts
	if best_y < 180:
		raise RuntimeError(f"sync flash not found (max YAVG {best_y})")
	return best_t

def esc_drawtext(work, i, text):
	f = work / f"banner{i}.txt"
	f.write_text(text)
	return f

def encode_final(rec, work, out_mp4, video_end_e):
	flash_t = find_flash(rec.raw, work)
	log(f"sync flash at video t={flash_t:.3f}s")
	to_vt = lambda epoch: flash_t + (epoch - rec.flash_e)
	trim = to_vt(rec.t0_e)
	dur = video_end_e - rec.t0_e

	filters = []
	for i, (s_e, e_e, text, pos) in enumerate(rec.banners):
		s = max(0.0, to_vt(s_e) - trim)
		e = max(s + BANNER_MIN, to_vt(e_e) - trim)
		tf = esc_drawtext(work, i, text)
		# "top" = upper-right, clear of nano/less title rows and left-aligned text
		x, y = ("w-text_w-48", "64") if pos == "top" else ("(w-text_w)/2", "h-150")
		filters.append(
			f"drawtext=fontfile={BANNER_TTF}:textfile={tf}:fontsize=40:"
			f"fontcolor=white@0.94:box=1:boxcolor=black@0.55:boxborderw=16:"
			f"x={x}:y={y}:enable='between(t,{s:.3f},{e:.3f})'")
	vf = ",".join(filters) if filters else "null"

	rng = random.Random(1)
	audio = build_audio(rec, work, dur, rng)
	run(["ffmpeg", "-v", "error", "-y",
		"-ss", f"{trim:.3f}", "-i", str(rec.raw), "-i", str(audio),
		"-t", f"{dur:.3f}", "-vf", vf,
		"-c:v", "libx264", "-preset", "slow", "-crf", "18", "-pix_fmt", "yuv420p",
		"-r", str(FPS), "-c:a", "aac", "-b:a", "160k",
		"-movflags", "+faststart", str(out_mp4)])
	return out_mp4

# the README gif is a highlight reel, not the whole video: a full-length
# 50fps half-size gif measures ~100 MiB, which no repo should carry. The mp4
# is the real deliverable; this window covers intro + the burst-scroll money shot.
GIF_START = 1.0
GIF_DUR   = 26.0
GIF_FPS   = 30

def encode_gif(mp4, out_gif, work):
	pal = work / "pal.png"
	flt = f"fps={GIF_FPS},scale=960:540:flags=lanczos"
	cut = ["-ss", str(GIF_START), "-t", str(GIF_DUR)]
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(mp4),
		"-vf", f"{flt},palettegen=stats_mode=diff", str(pal)])
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(mp4), "-i", str(pal),
		"-lavfi", f"{flt}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle",
		str(out_gif)])
	return out_gif


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Output placement + rotation

def place_outputs(mp4, gif, out_dir, no_rotate):
	out_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	dst_mp4 = out_dir / f"silkterm-demo_{stamp}.mp4"
	shutil.copy2(mp4, dst_mp4)
	if not no_rotate:
		inc = REPO / "cicd/utility/include/gfs-rotate.bash"
		subprocess.run(["bash", "-c",
			f'source "{inc}" && gfs_rotate "{out_dir}" silkterm-demo mp4'],
			check=False)
	assets = REPO / "assets"
	if gif:
		shutil.copy2(gif, assets / "demo.gif")
	log(f"video: {dst_mp4}")
	if gif:
		log(f"gif:   {assets / 'demo.gif'} ({(assets / 'demo.gif').stat().st_size // (1 << 20)} MiB)")


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Entry

def main():
	ap = argparse.ArgumentParser(description="Record the SilkTerm demo video.")
	ap.add_argument("--display", default=os.environ.get("SILK_DEMO_DISPLAY", ":98"))
	ap.add_argument("--segments", default="", help="comma list; default all")
	ap.add_argument("--seed", type=int, default=None)
	ap.add_argument("--keep-work", action="store_true")
	ap.add_argument("--no-gif", action="store_true")
	ap.add_argument("--no-rotate", action="store_true")
	ap.add_argument("--out-dir", default=str(PRIVATE))
	args = ap.parse_args()

	seed = args.seed if args.seed is not None else int(time.time()) & 0xFFFF
	rng = random.Random(seed)
	log(f"seed {seed}")

	rec = Rec(args)
	try:
		write_config(rec.work)
		home = write_tree(rec.work, rng)
		cfg = rec.work / "config.toml"
		proj = home / "projects" / "pulsar"

		rec.start_display()
		rec.start_capture()
		log("capture running; launching app")
		rec.launch_app(cfg, "/bin/bash --noprofile --norc -i", cwd=str(proj))
		time.sleep(2.0)
		rec.t0_e = time.time() - LEAD_S

		t = Typist(rec, rng)
		m = Mouse(rec, rng)
		want = [s.strip() for s in args.segments.split(",") if s.strip()]
		for name, fn in SEGMENTS:
			if want and name not in want:
				continue
			log(f"segment: {name}")
			fn(rec, t, m)
		time.sleep(1.5)
		video_end_e = time.time()

		rec.stop_capture()
		rec.kill_app()

		out_mp4 = rec.work / "demo.mp4"
		encode_final(rec, rec.work, out_mp4, video_end_e)
		gif = None
		if not args.no_gif:
			gif = encode_gif(out_mp4, rec.work / "demo.gif", rec.work)
		place_outputs(out_mp4, gif, Path(args.out_dir), args.no_rotate)
		if rec.keep:
			log(f"work dir kept: {rec.work}")
	finally:
		rec.cleanup()

if __name__ == "__main__":
	main()


##	Script history:
##		- 20260711 JC: Created.
