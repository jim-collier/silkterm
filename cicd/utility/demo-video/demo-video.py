#!/usr/bin/env python3

##	Purpose:
##		Record the SilkTerm demo video and README gif: drives a real SilkTerm on a
##		private Xvfb (never :0) inside a decorated window, types at a realistic pace
##		(variable wpm, occasional fixed typos), lays down keyboard/mouse foley
##		synced to the actual input timestamps, overlays per-segment narration, and
##		encodes the deliverables. Two recordings from one script, each maxing out
##		its format:
##		  video: 1920x1080@60 h265, font 1.5x the defined size, with audio
##		  gif:   960x540@50 native, defined font size, optimized palette, silent
##		The app is rendered ON THE GPU via VirtualGL (vglrun -d egl); on plain
##		llvmpipe the Xvfb caps it near 10fps and the scroll judders, which no
##		capture rate or frame-averaging can fix (the frames aren't there to blend).
##		On the GPU it paints a true ~60fps, so we grab straight at the delivery
##		rate. The window size is passed at LAUNCH (--pixel-width/height), never
##		resized after: the VGL EGL present latches the surface size at creation
##		(the app's xcb event connection bypasses VGL's Xlib interposer), so a
##		post-launch xdotool resize leaves a stale-offset blit (clipped video /
##		band-at-top gif). The outro comment goes gray via a prompt flag (no
##		ble.sh - it drops the odd first keystroke and breaks commands).
##		Both profiles start opaque on a plain black background (no image); the
##		wallpaper scenes bring the imagery in through the app's own --wallpaper.
##	Syntax:
##		demo-video.py [--profile video,gif] [--segments a,b,...] [--seed N]
##		              [--keep-work] [--no-rotate] [--display :98] [--out-dir DIR]
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
import getpass
import json
import math
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
from scipy import signal as spsig

ME_DIR   = Path(__file__).resolve().parent
REPO     = ME_DIR.parents[2]                  # github/cicd/utility/demo-video -> github
PRIVATE  = REPO.parent / "private" / "demo-video"
SOUNDS   = ME_DIR / "sounds"
BACKGNDS = REPO / "filesystem/home/.config/silkterm/backgrounds"

SR         = 48000                            # audio mix rate
BANNER_TTF = "/usr/share/fonts/truetype/lato/Lato-Semibold.ttf"
LEAD_S     = 0.8                              # quiet lead-in kept before the first segment
TAIL_HOLD_S  = 3.0                            # freeze the final frame this long at the end...
TAIL_BLACK_S = 2.0                            # ...then a fully black screen this long
TAIL_EXTRA   = TAIL_HOLD_S + TAIL_BLACK_S     # total appended tail (added at encode, not captured)
FOLEY_LAG  = 0.03                             # foley sits this far after the key event (the app
                                              # paints the glyph a frame or two later; sound-to-
                                              # picture reads tighter than sound-to-keypress)

# The app is driven through the GPU (VirtualGL, see launch_app) so it renders a
# genuine ~60fps on the headless Xvfb - on plain llvmpipe it only manages ~10
# distinct frames/sec, which no capture rate or frame-averaging can un-judder
# (the frames simply aren't there to blend). With the GPU the source is smooth,
# so we grab at the delivery rate straight: cap_fps == what the app paints.
PROFILES = {
	"video": dict(
		size=(1920, 1080), cap_fps=60, out_fps=60, mono_pt=19.5, ui_pt=11,
		banner_fs=38, banner_pad=18, audio=True, banner_min=4.0,
	),
	"gif": dict(
		size=(960, 540), cap_fps=50, out_fps=50, mono_pt=13, ui_pt=10,
		banner_fs=24, banner_pad=12, audio=False, banner_min=3.0,
	),
}

def log(msg):
	print(f"[demo] {msg}", flush=True)

def run(cmd, **kw):
	return subprocess.run(cmd, check=True, **kw)

def out_of(cmd):
	return subprocess.run(cmd, check=True, capture_output=True, text=True).stdout


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Recorder: display/app/capture lifecycle + the event/banner logs

class Rec:
	def __init__(self, args, profile):
		self.p        = profile
		self.size     = profile["size"]
		self.cap_fps  = profile["cap_fps"]
		self.out_fps  = profile["out_fps"]
		self.display  = args.display
		self.num      = self.display.lstrip(":")
		self.auth     = f"/tmp/cicd-gui-headless-{os.environ['USER']}/Xauthority-{self.num}"
		self.bin      = os.environ.get("SILK_BIN", str(REPO / "target/release/silkterm"))
		self.work     = Path(tempfile.mkdtemp(prefix="silk-demo-"))
		self.home     = self.work / "home"
		self.keep     = args.keep_work
		self.events   = []      # (epoch, kind) kind: key:NAME / mouse:NAME
		self.banners  = []      # (epoch_start, epoch_end, text, pos)
		self.app      = None
		self.ff       = None
		self.flash_e  = 0.0     # wall-clock epoch of the white sync flash
		self.t0_e     = 0.0     # wall-clock epoch where trimmed content starts
		self.seg_marks = {}     # segment name -> wall-clock epoch it started

	def env(self):
		e = dict(os.environ)
		e.update(DISPLAY=self.display, XAUTHORITY=self.auth, LIBGL_ALWAYS_SOFTWARE="1")
		return e

	def xdo(self, *a):
		subprocess.run(["xdotool", *a], env=self.env(), check=False,
			stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

	def start_display(self):
		# each profile records at its own resolution, so cycle the display; the WM
		# is ours (not gui-headless --wm) so xfconf can pick a quiet dark titlebar
		# theme - the window's real decoration is what frames the shot
		gh = str(REPO / "cicd/utility/gui-headless.bash")
		e = dict(os.environ, CICD_HEADLESS_DISPLAY=self.display,
			CICD_HEADLESS_SIZE=f"{self.size[0]}x{self.size[1]}x24")
		subprocess.run([gh, "stop"], env=e, capture_output=True)
		run([gh, "start"], env=e)
		self.wm = subprocess.Popen(["dbus-run-session", "--", "sh", "-c",
			'xfconf-query -c xfwm4 -p /general/theme --create -t string -s "Arctodon-Dark"; '
			'xfconf-query -c xfwm4 -p /general/title_font --create -t string -s "Lato Bold 10"; '
			'xfconf-query -c xfwm4 -p /general/button_layout --create -t string -s "O|HMC"; '
			"exec xfwm4 --compositor=off --vblank=off"],
			env=self.env(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
		time.sleep(2.0)
		subprocess.run(["xsetroot", "-solid", "#0a0c10"], env=self.env(), check=False)

	def stop_display(self):
		if getattr(self, "wm", None):
			self.wm.terminate()
			try:
				self.wm.wait(timeout=5)
			except subprocess.TimeoutExpired:
				self.wm.kill()
			self.wm = None
		gh = str(REPO / "cicd/utility/gui-headless.bash")
		e = dict(os.environ, CICD_HEADLESS_DISPLAY=self.display)
		subprocess.run([gh, "stop"], env=e, capture_output=True)

	def start_capture(self):
		self.raw = self.work / "raw.mkv"
		self.ff = subprocess.Popen([
			"ffmpeg", "-hide_banner", "-loglevel", "error",
			"-progress", str(self.work / "ffprogress.txt"),
			"-f", "x11grab", "-framerate", str(self.cap_fps),
			"-video_size", f"{self.size[0]}x{self.size[1]}", "-i", self.display,
			"-c:v", "libx264", "-preset", "ultrafast", "-qp", "0",
			"-pix_fmt", "yuv444p", str(self.raw)],
			env=self.env(), stdin=subprocess.DEVNULL,
			stderr=open(self.work / "ffmpeg.log", "w"))
		# flash only once frames are actually flowing - a slow-opening ffmpeg
		# would otherwise miss the sync flash and break the whole AV anchor
		prog = self.work / "ffprogress.txt"
		deadline = time.time() + 30
		while time.time() < deadline:
			if prog.exists() and re.search(r"(?m)^frame=([1-9]\d*)", prog.read_text()):
				break
			time.sleep(0.3)
		else:
			raise RuntimeError("x11grab produced no frames (see ffmpeg.log)")
		time.sleep(0.8)
		subprocess.run(["xsetroot", "-solid", "white"], env=self.env(), check=False)
		self.flash_e = time.time()
		time.sleep(0.25)
		subprocess.run(["xsetroot", "-solid", "#0a0c10"], env=self.env(), check=False)
		time.sleep(0.4)

	def stop_capture(self):
		if self.ff:
			self.ff.send_signal(signal.SIGINT)
			try:
				self.ff.wait(timeout=30)
			except subprocess.TimeoutExpired:
				self.ff.kill()
			self.ff = None

	def launch_app(self, shell_cmd):
		e = self.env()
		e.pop("LIBGL_ALWAYS_SOFTWARE", None)      # the app runs on the GPU (vglrun)
		# the pop-out dialogs (Settings/About) are static wgpu/Vulkan windows; pin
		# them to lavapipe so they don't chase a GPU Vulkan surface Xvfb can't present
		# gray prompt, rose user, sand host. The trailing bit grays whatever is TYPED
		# after the prompt WHEN a flag file exists - that's how the outro comment goes
		# gray ("as if ble.sh") without ble.sh, which drops the odd first keystroke.
		gray_flag = ("\\[$(test -f \"$HOME/.silk-gray\" && "
			"printf '\\033[38;5;245m')\\]")
		e.update(SHELL="/bin/bash", HOME=str(self.home),
			XDG_CONFIG_HOME=str(self.home / ".config"),
			PATH=f"{self.home}/bin:{os.environ['PATH']}",
			VK_ICD_FILENAMES="/usr/share/vulkan/icd.d/lvp_icd.json",
			PS1="\\[\\e[38;2;224;144;158m\\]juno\\[\\e[38;2;150;156;162m\\]@"
				"\\[\\e[38;2;222;178;134m\\]vela\\[\\e[38;2;150;156;162m\\]:\\w\\$ "
				"\\[\\e[0m\\]" + gray_flag,
			HISTFILE="/dev/null")
		# VirtualGL routes the app's GL to the real GPU (EGL backend, no 3D X
		# server needed) - without it llvmpipe caps the app at ~10fps and the
		# scroll judders. Fall back to software if vgl is missing.
		cmd = [self.bin, "--config", str(self.home / ".config/silkterm/config.toml"),
			"--shell", shell_cmd]
		if shutil.which("vglrun"):
			cmd = ["vglrun", "-d", "egl", *cmd]
		else:
			log("WARNING: vglrun not found - falling back to software GL (scroll will judder)")
			e["LIBGL_ALWAYS_SOFTWARE"] = "1"
		# a decorated (non-fullscreen) window: xfwm4 draws the full frame + the
		# titlebar with buttons, which is the "fake decoration" the shot wants.
		# Sized to leave a small dark margin so it reads as a floating window.
		# The client size goes in at LAUNCH (--pixel-width/height) and the window
		# is never resized after - the VGL EGL present latches the surface size at
		# creation, so a post-launch xdotool resize breaks the blit (moving is fine).
		W, H = self.size
		mx, my = int(W * 0.03), int(H * 0.05)
		cmd += ["--pixel-width", str(W - 2 * mx), "--pixel-height", str(H - 2 * my - 24)]
		self.app = subprocess.Popen(cmd, env=e, cwd=str(self.home),
			stdout=open(self.work / "silk.log", "w"), stderr=subprocess.STDOUT)
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
		self.xdo("windowmove", win, str(mx), str(my))
		time.sleep(4.0)                           # GPU GL bring-up + first frames
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
		self.xdo("mousemove", str(self.size[0] - 6), str(self.size[1] - 6))

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
# char -> XT scancode: the key bank has one unique slice per physical key, so
# every key thocks with its own sample; a shifted symbol thocks with its base
# key, same as a real board
_SHIFTED = dict(zip('!@#$%^&*()_+{}:"<>?~|', "1234567890-=[];',./`\\"))
_SCAN = {c: 2 + i for i, c in enumerate("1234567890-=")}
_SCAN |= {c: 16 + i for i, c in enumerate("qwertyuiop[]")}
_SCAN |= {c: 30 + i for i, c in enumerate("asdfghjkl;'")}
_SCAN |= {c: 44 + i for i, c in enumerate("zxcvbnm,./")}
_SCAN |= {"`": 41, "\\": 43, " ": 57}
KEY_CODES = {"SPACE": 57, "ENTER": 28, "BACKSPACE": 14, "TAB": 15,
	"ESC": 1, "ESCAPE": 1, "UP": 57416, "DOWN": 57424, "LEFT": 57419,
	"RIGHT": 57421, "PGUP": 3657, "PGDN": 3665}

def key_sound(ch):
	c = _SHIFTED.get(ch, ch.lower())
	return f"key:{_SCAN.get(c, 30)}"          # unknown lands on 'a'

def keysym_sound(keysym):
	if len(keysym) == 1:
		return key_sound(keysym)
	return f"key:{KEY_CODES.get(keysym.upper(), 30)}"

class Typist:
	def __init__(self, rec, rng):
		self.rec = rec
		self.rng = rng
		self.wpm = rng.uniform(120, 160)

	def _delay(self):
		# per-char delay from current wpm, lognormal jitter; wpm drifts as it would
		self.wpm += self.rng.uniform(-8, 8)
		self.wpm = max(100.0, min(220.0, self.wpm))
		d = 12.0 / self.wpm                      # 60 / (5 * wpm)
		return d * self.rng.lognormvariate(0.0, 0.22)

	def _emit(self, ch):
		# timestamp AFTER the send so the xdotool spawn latency never skews the
		# foley; the event epoch is the moment X actually got the key
		if ch == " ":
			self.rec.xdo("key", "--clearmodifiers", "space")
			self.rec.ev("key:SPACE")
		else:
			subprocess.run(["xdotool", "type", "--delay", "0", "--", ch],
				env=self.rec.env(), check=False,
				stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
			self.rec.ev(key_sound(ch))

	def _backspace(self, n):
		for _ in range(n):
			time.sleep(self.rng.uniform(0.09, 0.16))
			self.rec.xdo("key", "--clearmodifiers", "BackSpace")
			self.rec.ev("key:BACKSPACE")

	def type(self, text, typos=0.018, wpm=None):
		if wpm is not None:
			self.wpm = wpm
		# ensure the terminal has focus before the first keystroke: after a dialog
		# closes the first char can race the focus handoff and drop (which turned
		# "silkterm" into "ilkterm" and broke the wallpaper command)
		self.rec.xdo("windowactivate", self.rec.win)
		time.sleep(0.3)
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
		self.rec.xdo("key", "--clearmodifiers", "Return")
		self.rec.ev("key:ENTER")

	def key(self, keysym, sound=None):
		self.rec.xdo("key", "--clearmodifiers", keysym)
		if sound is None:
			sound = keysym_sound(keysym)
		if sound:
			self.rec.ev(sound)

	def keys(self, keysym, n, hz=8.0, sound=None):
		# repeated taps (arrow scrolling); slight cadence wobble
		for _ in range(n):
			self.key(keysym, sound)
			time.sleep(max(0.03, self.rng.uniform(0.8, 1.2) / hz))

	def hold(self, keysym, count, hz=55.0, first_sound=None):
		# a held key, faked as fast discrete repeats (Xvfb has no autorepeat, so a
		# real keydown/keyup delivers just one press): one click on the first
		# press, silence for the rest - reads as press-and-hold
		if first_sound is None:
			first_sound = keysym_sound(keysym)
		if first_sound:
			self.rec.ev(first_sound)
		self.rec.xdo("key", "--clearmodifiers", "--repeat", str(count),
			"--delay", str(int(1000 / hz)), keysym)

	def cmd(self, text, settle=1.0, typos=0.018, wpm=None):
		self.type(text, typos, wpm)
		self.enter()
		time.sleep(settle)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Mouse

class Mouse:
	def __init__(self, rec, rng):
		self.rec = rec
		self.rng = rng
		self.pos = (rec.size[0] - 6, rec.size[1] - 6)

	def move(self, x, y, dur=0.6):
		x0, y0 = self.pos
		steps = max(6, int(dur * 40))
		for i in range(1, steps + 1):
			t = i / steps
			t = t * t * (3 - 2 * t)              # smoothstep
			self.rec.xdo("mousemove", str(int(x0 + (x - x0) * t)), str(int(y0 + (y - y0) * t)))
			time.sleep(dur / steps)
		self.pos = (x, y)

	def circle(self, cx, cy, r, loops=2.0, dur=4.0):
		steps = max(30, int(dur * 30))
		for i in range(steps + 1):
			a = 2 * math.pi * loops * i / steps - math.pi / 2
			self.rec.xdo("mousemove",
				str(int(cx + r * math.cos(a))), str(int(cy + r * 0.7 * math.sin(a))))
			time.sleep(dur / steps)
		self.pos = (cx, cy - r)

	def click(self, quiet=False):
		self.rec.xdo("click", "1")
		self.rec.ev("mouse:CLICK_Q" if quiet else "mouse:CLICK")

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
		self.pos = (self.rec.size[0] - 6, self.rec.size[1] - 6)

	def wheel(self, up, n, hz=7.0):
		for _ in range(n):
			self.rec.ev("mouse:WHEEL")
			self.rec.xdo("click", "4" if up else "5")
			time.sleep(self.rng.uniform(0.8, 1.2) / hz)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Banner bookkeeping

class Banner:
	def __init__(self, rec, text, pos="tr"):
		self.rec, self.text, self.pos = rec, text, pos

	def __enter__(self):
		self.start = time.time()
		return self

	def __exit__(self, *exc):
		self.rec.banners.append((self.start, time.time(), self.text, self.pos))


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Scene content: recording fonts, config, the synthetic desktop, home tree

def write_dconf(home, profile):
	# the app reads both recording fonts through gsettings; with XDG_CONFIG_HOME
	# on the fake home a compiled dconf db is all it takes. Chrome/dialogs get a
	# clean modern sans; the terminal gets the defined mono at the profile's size.
	src = home.parent / "dconf-src"
	src.mkdir(exist_ok=True)
	(src / "ifc.txt").write_text(
		"[org/gnome/desktop/interface]\n"
		f"font-name='Lato {profile['ui_pt']}'\n"
		f"monospace-font-name='Monaspace Argon Semi-Bold {profile['mono_pt']}'\n")
	dst = home / ".config" / "dconf"
	dst.mkdir(parents=True, exist_ok=True)
	run(["dconf", "compile", str(dst / "user"), str(src)])

def write_config(home, profile):
	# mirrors the real defined config. Both profiles start opaque on plain black:
	# no background_image (numbered filenames dodge the auto-detect), image
	# opacity at the 0.10 default - the wallpaper scenes bring the imagery in.
	cfgdir = home / ".config" / "silkterm"
	bgdir = cfgdir / "backgrounds"
	bgdir.mkdir(parents=True, exist_ok=True)
	shutil.copy2(BACKGNDS / "background41.jpg", bgdir / "background41.jpg")
	shutil.copy2(BACKGNDS / "background45.jpg", bgdir / "background45.jpg")
	(cfgdir / "config.toml").write_text('''use_system_font = true
line_height_scale = 1.22
margin = 8.0
remember_size = false
columns = 160
rows = 48
transparent_background = false
background_opacity = 0.10
background_fit = "zoom"
background_blur = 10.0
text_scrim = true
text_outline = 2.0
text_scrim_ramp = "gaussian"
cursor_size_height = 100
cursor_size_width = 25
cursor_animation = "pulse_vertical"
cursor_animation_input = "continuous"
cursor_blink_rate_ms = 500
word_separators = "=,|:\\"' ()[]{}<>"
scrollback = 10000
scroll_tau_ms = 230.0
wheel_lines = 3.0
alt_scroll_lines = 3.0
output_ease_lines = 1.0
smooth_scroll_apps = true
theme = "SilkTerm"
theme_mode = "dark"
''')

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

# a believable generic home: enough entries that `ls -lA` runs past the bottom
HOME_DIRS = ["Desktop", "Documents", "Downloads", "Music", "Pictures", "Public",
	"Templates", "Videos", "bin", "projects",
	".cache", ".config", ".gnupg", ".local", ".mozilla", ".npm", ".ssh",
	".thunderbird", ".vim"]
HOME_DOTFILES = [(".bash_aliases", 361), (".bash_logout", 220), (".bashrc", 3526),
	(".curlrc", 74), (".dircolors", 4291), (".gitconfig", 412), (".gtkrc-2.0", 156),
	(".inputrc", 289), (".profile", 807), (".selected_editor", 66),
	(".tmux.conf", 1184), (".vimrc", 1204), (".wgetrc", 118), (".Xresources", 688)]
HOME_FILES = [("backup-2025.tar.gz", 1483477621), ("notes.md", 8412),
	("photo-kyoto.jpg", 3318554), ("pulsar-flame.svg", 96214),
	("resume.pdf", 188416), ("shopping.txt", 973), ("soundtrack.flac", 38119433),
	("todo.md", 2101)]

def write_tree(rec, rng):
	home = rec.home
	proj = home / "projects" / "pulsar"
	src = proj / "src"
	src.mkdir(parents=True)
	(proj / "docs").mkdir()
	(proj / "assets").mkdir()
	(proj / "Cargo.toml").write_text(
		'[package]\nname = "pulsar"\nversion = "0.4.1"\nedition = "2024"\n')
	(proj / "README.md").write_text("# pulsar\n\nA tiny GPU particle toy.\n")
	(proj / "LICENSE").write_text("MIT\n")
	(src / "scroll.rs").write_text(RUST_SCROLL * 5)
	(src / "main.rs").write_text('fn main() {\n\tpulsar::run();\n}\n')
	(src / "render.rs").write_text(RUST_SCROLL)

	for name in HOME_DIRS:
		(home / name).mkdir(parents=True, exist_ok=True)
	for name, size in HOME_DOTFILES + HOME_FILES:
		f = home / name
		f.touch()
		os.truncate(f, size)

	# `ls` wrapper: the on-camera alias resolves here first; a real listing would
	# print the real username as owner/group, so map it to the fake one
	bind = home / "bin"
	bind.mkdir(exist_ok=True)
	user = getpass.getuser()
	wrapper = bind / "ls"
	wrapper.write_text(f'#!/bin/dash\n/usr/bin/ls "$@" | sed "s/{user}/juno/g"\n')
	wrapper.chmod(0o755)
	(bind / "silkterm").symlink_to(rec.bin)
	# pin nano to no-softwrap so a config line stays on one screen row
	(home / ".nanorc").write_text("unset softwrap\nunset breaklonglines\n")

	# build.sh: cargo-flavoured output with varied pacing and burst sizes
	crates = ["proc-macro2", "quote", "syn", "libc", "bitflags", "smallvec",
		"cfg-if", "log", "parking_lot", "raw-window-handle", "wayland-client",
		"x11-dl", "ash", "naga", "wgpu-hal", "wgpu-core", "wgpu", "winit",
		"glam", "bytemuck", "pollster", "image", "rayon", "pulsar"]
	lines = ["#!/bin/dash", 'g="\\033[1;32m"; y="\\033[1;33m"; b="\\033[1;34m"; r="\\033[0m"']
	lines.append('printf "   ${g}Compiling${r} pulsar workspace\\n"')
	for c in crates:
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

def prep_content(rec, rng):
	write_dconf(rec.home, rec.p)
	write_config(rec.home, rec.p)
	write_tree(rec, rng)


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Settings dialog driving

def open_settings(rec):
	rec.ev(key_sound(","))
	rec.xdo("key", "--clearmodifiers", "ctrl+comma")
	time.sleep(2.0)
	for _ in range(12):
		out = subprocess.run(["xdotool", "search", "--name", "Settings"],
			env=rec.env(), capture_output=True, text=True).stdout.strip()
		if out:
			dlg = out.split()[0]
			break
		time.sleep(0.5)
	else:
		return None
	# park it right of center so the live change shows on the terminal around it
	gx = rec.size[0] - 600 if rec.size[0] > 1200 else rec.size[0] - 560
	gy = 150 if rec.size[1] > 700 else 14
	rec.xdo("windowmove", dlg, str(max(0, gx)), str(gy))
	rec.xdo("windowactivate", dlg)
	time.sleep(1.0)
	return dlg

def dlg_client(rec, dlg):
	# xwininfo reports the CLIENT rect; xdotool's geometry includes the WM frame,
	# which once put a computed click on the titlebar border
	out = subprocess.run(["xwininfo", "-id", dlg], env=rec.env(),
		capture_output=True, text=True).stdout
	def grab(pat):
		return int(re.search(pat + r":\s+(-?\d+)", out).group(1))
	return (grab(r"Absolute upper-left X"), grab(r"Absolute upper-left Y"),
		grab(r"Width"), grab(r"Height"))


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Segments (each takes the recorder, typist, mouse)

def seg_alias(r, t, m):
	# no narration - it is a plain shell alias, nothing to explain
	t.cmd('alias ls="ls -lA --color --group-directories-first"', settle=1.2,
		wpm=200, typos=0.0)
	time.sleep(0.6)

def seg_settings(r, t, m):
	# open Settings, dwell ~4s slowly circling the text-scrim rows, then cancel
	# with Esc - no trip to a button, and NO mouse motion after the dwell (the
	# park is a single off-frame jump). The point is readable text everywhere.
	with Banner(r, "Readable text over any background", pos="tl"):
		dlg = open_settings(r)
		if not dlg:
			return
		x, y, w, h = dlg_client(r, dlg)
		# the scrim controls (radius/softness/outline/function/falloff) sit in the
		# lower half of the Appearance tab
		m.circle(x + int(w * 0.45), y + int(h * 0.66), int(w * 0.22), loops=1.5, dur=4.0)
		time.sleep(0.3)
		r.ev(keysym_sound("Escape"))
		r.xdo("key", "--clearmodifiers", "Escape")   # cancel, nothing changed
		time.sleep(0.5)
		r.xdo("windowactivate", r.win)
		m.park()
		time.sleep(1.0)

def seg_wp41(r, t, m):
	with Banner(r, "Per-window wallpaper, from the shell", pos="top"):
		t.cmd("silkterm --wallpaper ~/.config/silkterm/backgrounds/background41.jpg",
			settle=3.2)
		time.sleep(1.0)

def seg_wp45(r, t, m):
	# no narration - the second image change speaks for itself
	t.cmd("silkterm --wallpaper ~/.config/silkterm/backgrounds/background45.jpg",
		settle=3.0)
	time.sleep(0.8)

def seg_ls(r, t, m):
	with Banner(r, "...smooth output scroll...", pos="top"):
		t.cmd("ls ~/", settle=3.6)
		time.sleep(1.0)

def seg_build(r, t, m):
	with Banner(r, "Smooth cursor. Smooth scroll.", pos="top"):
		t.cmd("cd projects/pulsar", settle=0.6, typos=0.0)
		t.cmd("./build.sh", settle=7.5)          # covers the script's own runtime
		time.sleep(1.0)

def seg_less(r, t, m):
	with Banner(r, "Full-screen apps glide too", pos="top"):
		t.cmd("less -R docs/render.log", settle=1.4)
		t.keys("Down", 16, hz=7.0)
		time.sleep(0.7)
		t.keys("Up", 8, hz=6.0)
		time.sleep(0.6)
		# re-assert focus before quitting - a stray focus loss during the arrow
		# scrolling would leave less open and swallow the commands that follow
		r.xdo("windowactivate", r.win)
		time.sleep(0.3)
		t.key("q")
		time.sleep(1.0)

def seg_outro(r, t, m):
	# drop the flag the prompt watches for; a plain Return then draws a FRESH prompt
	# that grays whatever is typed next - so the comment goes gray from the '#' on,
	# as if ble.sh were installed, but with plain reliable bash typing. (ctrl+l was
	# avoided - clearing right after less's alt-screen exit could swallow the line.)
	(r.home / ".silk-gray").touch()
	with Banner(r, "github.com/jim-collier/silkterm", pos="top"):
		r.xdo("windowactivate", r.win)
		time.sleep(0.3)
		r.xdo("key", "--clearmodifiers", "Return")   # fresh prompt picks up the flag
		time.sleep(0.7)
		t.cmd("# Smooth. Silky. ...SilkTerm.", settle=0.5, typos=0.0)
		time.sleep(3.2)

# one script, both profiles (video and gif differ only in size/fonts/audio)
_SCRIPT = [
	("alias",    seg_alias),
	("ls",       seg_ls),
	("build",    seg_build),
	("settings", seg_settings),
	("wp41",     seg_wp41),
	("less",     seg_less),
	("wp45",     seg_wp45),
	("outro",    seg_outro),
]
SEGMENTS = {"video": _SCRIPT, "gif": _SCRIPT}


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Audio: process the key bank, mix the event log into a wav

SOUND_FILES = {
	"mouse:CLICK":    SOUNDS / "mouse/click.wav",
	"mouse:CLICK_Q":  SOUNDS / "mouse/click_quiet.wav",
}
KEYPACK = SOUNDS / "keys-oreo"     # mechvibes "EG Oreo": one recording, one slice per key
GAIN = {"key": 0.85, "mouse:CLICK": 0.5, "mouse:CLICK_Q": 0.36, "mouse:WHEEL": 0.5}

# the bank is quiet and slice loudness wanders ~6 dB; even each slice out to a
# consistent body presence (space/enter a touch prouder) but keep every key's
# own transient and timbre - that natural variety is the whole point of a
# per-key bank
KEY_BODY = {57: 0.085, 28: 0.085, 14: 0.07}

def shape_slice(s, code):
	rms = np.sqrt((s ** 2).mean()) + 1e-9
	s = s * (KEY_BODY.get(code, 0.062) / rms)
	n_in, n_out = int(SR * 0.001), int(SR * 0.006)
	s[:n_in] *= np.linspace(0.0, 1.0, n_in)[:, None]      # slice edges must not click
	s[-n_out:] *= np.linspace(1.0, 0.0, n_out)[:, None]
	peak = np.abs(s).max()
	if peak > 0.7:                                # keep one loud hit from owning the mix
		s *= 0.7 / peak
	return s.astype(np.float32)

def load_keypack(work, cache):
	cfg = json.loads((KEYPACK / "config.json").read_text())
	raw = work / "keypack.pcm"
	run(["ffmpeg", "-v", "error", "-y", "-i", str(KEYPACK / cfg["sound"]),
		"-ar", str(SR), "-ac", "2", "-f", "s16le", str(raw)])
	pcm = np.frombuffer(raw.read_bytes(), dtype=np.int16) \
		.astype(np.float32).reshape(-1, 2) / 32768.0
	for code, span in cfg["defines"].items():
		if not span:
			continue
		start, dur = span
		s = pcm[int(start * SR / 1000):int((start + dur) * SR / 1000)].copy()
		if len(s) < SR // 100:
			continue
		cache[f"key:{code}"] = shape_slice(s, int(code))
	for name, code in KEY_CODES.items():
		if f"key:{code}" in cache:
			cache[f"key:{name}"] = cache[f"key:{code}"]

def synth_wheel(sr):
	# a soft scroll-wheel detent: a short muffled tick, much softer and darker
	# than a mouse click - a hair of noise on a low damped thonk, low-passed
	n = int(sr * 0.030)
	tt = np.arange(n) / sr
	noise = np.random.default_rng(3).standard_normal(n) * np.exp(-tt * 320)
	body = np.sin(2 * math.pi * 175 * tt) * np.exp(-tt * 150)
	mix = noise * 0.45 + body * 0.55
	sos = spsig.butter(2, 1700, btype="low", fs=sr, output="sos")
	mix = spsig.sosfilt(sos, mix)
	mix /= np.abs(mix).max() + 1e-9
	return np.stack([mix, mix], axis=1).astype(np.float32) * 0.28

def load_samples(work):
	cache = {}
	for kind, path in SOUND_FILES.items():
		wav = work / (re.sub(r"[^A-Za-z0-9]", "_", kind) + ".wav")
		run(["ffmpeg", "-v", "error", "-y", "-i", str(path),
			"-ar", str(SR), "-ac", "2", "-f", "wav", str(wav)])
		with wave.open(str(wav), "rb") as w:
			data = np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16)
		s = data.astype(np.float32).reshape(-1, 2) / 32768.0
		cache[kind] = s
	load_keypack(work, cache)
	cache["mouse:WHEEL"] = synth_wheel(SR)
	return cache

def build_audio(rec, work, duration, rng):
	cache = load_samples(work)
	mix = np.zeros((int(duration * SR) + SR, 2), dtype=np.float32)
	for epoch, kind in rec.events:
		t_rel = epoch - rec.t0_e + FOLEY_LAG
		if t_rel < -0.5 or t_rel > duration:
			continue
		s = cache.get(kind)
		if s is None:
			continue
		gain = GAIN.get(kind, GAIN.get(kind.split(":")[0], 0.8))
		gain *= rng.uniform(0.85, 1.05)           # stroke-force wobble; samples are raw
		samp = s
		at = int(max(0.0, t_rel) * SR)
		end = min(at + len(samp), len(mix))
		mix[at:end] += samp[: end - at] * gain
	peak = np.abs(mix).max()
	if peak > 0:
		mix *= min(0.40 / peak, 4.0)              # ~ -8 dBFS, bounded boost
	out = work / "audio.wav"
	with wave.open(str(out), "wb") as w:
		w.setnchannels(2)
		w.setsampwidth(2)
		w.setframerate(SR)
		w.writeframes((mix * 32767.0).astype(np.int16).tobytes())
	return out


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Post: sync-flash location, motion-blur downsample, banners, encode

def check_drift(rec, video_end_e):
	dur = float(out_of(["ffprobe", "-v", "error", "-show_entries", "format=duration",
		"-of", "csv=p=0", str(rec.raw)]))
	expect = (video_end_e - rec.flash_e) + rec.flash_vt
	if abs(dur - expect) > max(0.5, expect * 0.02):
		log(f"WARNING: capture drift - raw {dur:.1f}s vs expected {expect:.1f}s; "
			"AV sync may be off (X server starved the grab loop?)")

def find_flash(raw, work):
	stats = work / "stats.txt"
	run(["ffmpeg", "-v", "error", "-t", "8", "-i", str(raw),
		"-vf", f"signalstats,metadata=print:key=lavfi.signalstats.YAVG:file={stats}",
		"-f", "null", "-"])
	best_t, best_y, pts = 0.0, -1.0, 0.0
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

# banner -> (x, y) expressions. The caption sits over the window's titlebar/menu
# chrome (out of the way of the terminal text - the owner would rather it cover
# the title bar than the content). `top` is centered there; tl/bl are corners for
# when the action itself is center/right (the Settings dialog).
def banner_xy(pos, size):
	m = int(size[0] * 0.025)
	chrome_y = int(size[1] * 0.05) + 6        # the title/menu strip at the top
	bot = "h-{}".format(int(size[1] * 0.05) + 118)
	return {
		"top": ("(w-text_w)/2", str(chrome_y)),
		"tr":  ("w-text_w-{}".format(m), str(chrome_y)),
		"tl":  (str(m), str(chrome_y)),
		"bl":  (str(m), bot),
		"br":  ("w-text_w-{}".format(m), bot),
	}.get(pos, ("(w-text_w)/2", str(chrome_y)))

# a quick damped-spring vertical bounce for the pop-in / pop-out (~0.6s each): the
# caption springs in from just below its rest line, rings down, and springs back
# out as it fades. `base` is the rest y (may be an expr like "h-118").
def wobble_y(base, s, e, amp):
	win = 0.6
	ring = f"{amp}*exp(-6*T)*cos(2*PI*2.6*T)"
	win_in  = ring.replace("T", f"(t-{s:.3f})")
	win_out = ring.replace("T", f"({e:.3f}-t)")
	return (f"({base})"
		f"+if(between(t,{s:.3f},{s + win:.3f}),{win_in},0)"
		f"+if(between(t,{e - win:.3f},{e:.3f}),{win_out},0)")

def vf_chain(rec, work, trim, dur, tail=False):
	p = rec.p
	to_vt = lambda epoch: rec.flash_vt + (epoch - rec.flash_e)
	# the GPU source is genuinely smooth, so just pin CFR at the delivery rate -
	# no frame-averaging needed (and none to fake, the frames are real)
	filters = [f"fps={rec.out_fps}"]
	# resolve each banner's [s,e]; then clamp every end to the next banner's start
	# minus a gap, so only ONE banner is ever on screen (consecutive banners were
	# crossfading into an overlapping smear)
	spans = []
	for s_e, e_e, text, pos in rec.banners:
		s = max(0.0, to_vt(s_e) - trim)
		e = max(s + p["banner_min"], to_vt(e_e) - trim)
		spans.append([s, e, text, pos])
	spans.sort(key=lambda b: b[0])
	GAP = 0.4
	for i in range(len(spans) - 1):
		spans[i][1] = min(spans[i][1], spans[i + 1][0] - GAP)
	amp = int(rec.size[1] * 0.018)            # bounce height ~19px @1080p
	for i, (s, e, text, pos) in enumerate(spans):
		if e <= s:
			continue
		tf = esc_drawtext(work, i, text)
		x, base_y = banner_xy(pos, rec.size)
		y = wobble_y(base_y, s, e, amp)
		# quick alpha pop (~0.15s) - the bounce carries the motion
		fade = f"clip((t-{s:.3f})/0.15,0,1)*clip(({e:.3f}-t)/0.15,0,1)"
		filters.append(
			f"drawtext=fontfile={BANNER_TTF}:textfile={tf}:fontsize={p['banner_fs']}:"
			f"fontcolor=white:box=1:boxcolor=0x333333:boxborderw={p['banner_pad']}:"
			f"x={x}:y='{y}':alpha='{fade}':enable='between(t,{s:.3f},{e:.3f})'")
	# no head/tail fades: the fade gradient is a fresh frame every step, which
	# bloats the gif enormously (palette churn + huge inter-frame deltas)
	# flatten to rgb24 so palettegen/paletteuse never see a stray alpha channel
	filters.append("format=rgb24")
	# end tail: hold the final frame (no motion) then a fully black screen. Only the
	# full-length outputs get it - not the looping highlight gif (default tail=False).
	if tail:
		filters.append(f"tpad=stop_mode=clone:stop_duration={TAIL_HOLD_S}")
		filters.append(f"tpad=stop_mode=add:color=black:stop_duration={TAIL_BLACK_S}")
	return ",".join(filters)

def encode_video(rec, work, out_mp4, video_end_e):
	rec.flash_vt = find_flash(rec.raw, work)
	log(f"sync flash at video t={rec.flash_vt:.3f}s")
	check_drift(rec, video_end_e)
	trim = rec.flash_vt + (rec.t0_e - rec.flash_e)
	dur = video_end_e - rec.t0_e
	vf = vf_chain(rec, work, trim, dur, tail=True)
	rng = random.Random(1)
	audio = build_audio(rec, work, dur, rng)   # tail is silent (freeze + black)
	run(["ffmpeg", "-v", "error", "-y",
		"-ss", f"{trim:.3f}", "-i", str(rec.raw), "-i", str(audio),
		"-t", f"{dur + TAIL_EXTRA:.3f}", "-vf", vf,
		"-c:v", "libx265", "-preset", "slow", "-crf", "20", "-pix_fmt", "yuv420p",
		"-tag:v", "hvc1", "-x265-params", "log-level=error",
		"-r", str(rec.out_fps), "-c:a", "aac", "-b:a", "160k",
		"-movflags", "+faststart", str(out_mp4)])
	return out_mp4

# the full 50fps 540p gif of nonstop SMOOTH scrolling is dense (~0.7 MB/s - denser
# than the old juddery one, since every frame now differs) - far past what a README
# should carry, so it goes to private/ and a lighter highlight is cut for the README:
# fewer fps + colors + a shorter window, still plainly smooth, small enough to inline.
# It opens where the smooth scrolling begins (the whole point), not at the top.
GIF_HL_SEG    = "ls"    # scene to open the highlight on
GIF_HL_DUR    = 9.0     # ls + build - enough to sell the scroll
GIF_HL_FPS    = 25      # half the full rate keeps it smooth at ~half the bytes
GIF_HL_COLORS = 128

def gif_pass(rec, work, out_gif, trim, dur, fps=None, colors=160, tail=False):
	vf = vf_chain(rec, work, trim, dur, tail=tail)
	if fps:                                   # highlight renders at a lighter rate
		vf = vf.replace(f"fps={rec.out_fps}", f"fps={fps}", 1)
	pal = work / "pal.png"
	cut = ["-ss", f"{trim:.3f}", "-t", f"{dur + (TAIL_EXTRA if tail else 0.0):.3f}"]
	# ONE global palette (stats_mode=full) applied uniformly: stats_mode=diff +
	# diff_mode=rectangle mis-handled the big inter-frame jumps of fast scrolling
	# and left white/ghosted blocks. Ordered bayer stays temporally stable (error
	# diffusion shimmers and bloats a gif).
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(rec.raw),
		"-vf", f"{vf},palettegen=stats_mode=full:max_colors={colors}", str(pal)])
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(rec.raw), "-i", str(pal),
		"-lavfi", f"{vf}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=4",
		str(out_gif)])
	return out_gif

def encode_gif(rec, work, out_gif, video_end_e):
	rec.flash_vt = find_flash(rec.raw, work)
	log(f"sync flash at video t={rec.flash_vt:.3f}s")
	check_drift(rec, video_end_e)
	trim = rec.flash_vt + (rec.t0_e - rec.flash_e)
	dur = video_end_e - rec.t0_e
	gif_pass(rec, work, out_gif, trim, dur, tail=True)
	# open the highlight on the scrolling; fall back to 1s in if the mark is absent
	mark = rec.seg_marks.get(GIF_HL_SEG)
	hl_start = (rec.flash_vt + (mark - rec.flash_e) - trim) if mark else 1.0
	hl_start = max(0.0, min(hl_start, dur - 2.0))
	hl = work / "demo-hl.gif"
	gif_pass(rec, work, hl, trim + hl_start, min(GIF_HL_DUR, dur - hl_start),
		fps=GIF_HL_FPS, colors=GIF_HL_COLORS)
	return out_gif, hl


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Output placement + rotation (video and gif in their own dirs)

GIF_ASSET_MAX_MB = 12

def rotate(out_dir, prefix, ext, no_rotate):
	if no_rotate:
		return
	inc = REPO / "cicd/utility/include/gfs-rotate.bash"
	subprocess.run(["bash", "-c",
		f'source "{inc}" && gfs_rotate "{out_dir}" {prefix} {ext}'], check=False)

def place_video(mp4, out_dir, no_rotate):
	out_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	dst = out_dir / f"silkterm-demo_{stamp}.mp4"
	shutil.copy2(mp4, dst)
	mb = dst.stat().st_size / (1 << 20)
	rotate(out_dir, "silkterm-demo", "mp4", no_rotate)
	log(f"video: {dst} ({mb:.1f} MiB)")

def place_gif(full, hl, out_dir, no_rotate):
	out_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	dst = out_dir / f"silkterm-demo_{stamp}.gif"
	shutil.copy2(full, dst)
	full_mb = dst.stat().st_size / (1 << 20)
	rotate(out_dir, "silkterm-demo", "gif", no_rotate)
	log(f"gif (full): {dst} ({full_mb:.1f} MiB)")
	mb = hl.stat().st_size / (1 << 20)
	if mb <= GIF_ASSET_MAX_MB:
		asset = REPO / "assets" / "demo.gif"
		shutil.copy2(hl, asset)
		log(f"gif (README highlight): {asset} ({mb:.1f} MiB)")
	else:
		log(f"WARNING: highlight gif is {mb:.1f} MiB (> {GIF_ASSET_MAX_MB}); "
			"assets/demo.gif left untouched - trim GIF_HL_DUR")


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Entry

def record(args, name, seed):
	rng = random.Random(seed)
	rec = Rec(args, PROFILES[name])
	try:
		prep_content(rec, rng)
		rec.start_display()
		rec.start_capture()
		log(f"[{name}] capture running; launching app")
		# --norc/--noprofile skips even the system bashrc (which spews real paths on
		# this box); PS1 comes in via the environment
		rec.launch_app("/bin/bash --noprofile --norc -i")
		time.sleep(2.5)
		rec.t0_e = time.time() - LEAD_S

		t = Typist(rec, rng)
		m = Mouse(rec, rng)
		want = [s.strip() for s in args.segments.split(",") if s.strip()]
		for seg, fn in SEGMENTS[name]:
			if want and seg not in want:
				continue
			log(f"[{name}] segment: {seg}")
			rec.seg_marks[seg] = time.time()
			fn(rec, t, m)
		time.sleep(0.3)                       # brief settle; the 3s hold is added at encode (tpad)
		video_end_e = time.time()

		rec.stop_capture()
		rec.kill_app()

		if name == "video":
			out = rec.work / "demo.mp4"
			encode_video(rec, rec.work, out, video_end_e)
			place_video(out, Path(args.out_dir) / "video", args.no_rotate)
		else:
			out = rec.work / "demo.gif"
			full, hl = encode_gif(rec, rec.work, out, video_end_e)
			place_gif(full, hl, Path(args.out_dir) / "gif", args.no_rotate)
		if rec.keep:
			log(f"[{name}] work dir kept: {rec.work}")
	finally:
		rec.cleanup()

def main():
	ap = argparse.ArgumentParser(description="Record the SilkTerm demo video + gif.")
	ap.add_argument("--display", default=os.environ.get("SILK_DEMO_DISPLAY", ":98"))
	ap.add_argument("--profile", default="video,gif", help="comma list: video,gif")
	ap.add_argument("--segments", default="", help="comma list; default all")
	ap.add_argument("--seed", type=int, default=None)
	ap.add_argument("--keep-work", action="store_true")
	ap.add_argument("--no-rotate", action="store_true")
	ap.add_argument("--out-dir", default=str(PRIVATE))
	args = ap.parse_args()

	seed = args.seed if args.seed is not None else int(time.time()) & 0xFFFF
	log(f"seed {seed}")
	for name in [p.strip() for p in args.profile.split(",") if p.strip()]:
		if name not in PROFILES:
			sys.exit(f"unknown profile: {name}")
		record(args, name, seed)

if __name__ == "__main__":
	main()


##	Script history:
##		- 20260713 JC: per-key sound bank (mechvibes EG Oreo, one slice per
##		  physical key) replaces the per-row bank; chars map to their real key's
##		  sample, so variety is natural - dropped the pitch-shift/spectral-tilt/
##		  mid-click processing and the separate release sounds.
##		- 20260713 JC: window size passed at launch (--pixel-width/height), never
##		  resized after - fixes the clipped video / band-at-top gif (VGL EGL
##		  latches the surface size at creation); both profiles start opaque on
##		  black (no bg image, image opacity 0.10); scene order alias-ls-build-
##		  settings-wp41-less-wp45-outro with two wallpaper scenes; synth desktop
##		  dropped.
##		- 20260712 JC: GPU render via VirtualGL (real ~60fps, the actual judder
##		  fix - dropped the high-fps+tmix hack); one unified script for both
##		  profiles; gray-# outro via a prompt flag; solid-gray captions with a
##		  wobble pop, moved onto the title/menu chrome; Settings scene circles the
##		  scrim rows then Esc-cancels; focus-settle before typing (fixes a dropped
##		  first keystroke after the dialog).
##		- 20260712 JC: Real window decoration; high-fps capture + motion-blur
##		  downsample (judder fix); dim vague dark desktop behind the glass; new
##		  scene order + mouse toggle/hold-arrow/gray-outro/wallpaper-clear;
##		  processed key bank (mid-click + variety) + soft wheel; top-right
##		  narration; video/gif split into their own output dirs.
##		- 20260712 JC: Two recordings (1080p60 h265 + native 540p50 gif),
##		  see-through desktop via config+socket reload, Lato narration.
##		- 20260711 JC: Created.
