#!/usr/bin/env python3

##	Purpose:
##		Record the SilkTerm demo video and README gif: drives a real SilkTerm on a
##		private Xvfb (never :0), types at a realistic pace (variable wpm, occasional
##		fixed typos), captures with ffmpeg, lays down keyboard/mouse foley synced to
##		the actual input timestamps, overlays per-segment narration, frames the
##		window with a thin border, and encodes the deliverables. Two independent
##		recordings, each maxing out its format:
##		  video: 1920x1080@60, h265, font 1.5x the defined size, with audio
##		  gif:   960x540@50 native, defined font size, optimized palette, silent
##		The see-through-terminal look is produced with the app's own background
##		pipeline: a generated dark-desktop image (code editor + file manager) is
##		swapped in as background_image at the moment OK lands in the Settings scene
##		(config rewrite + control-socket reload), standing in for the desktop
##		compositor, whose blur is approximated by background_blur.
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
import os
import random
import re
import shutil
import signal
import socket as socketmod
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
BACKGNDS = REPO / "filesystem/home/.config/silkterm/backgrounds"

SR         = 48000                            # audio mix rate
BANNER_TTF = "/usr/share/fonts/truetype/lato/Lato-Semibold.ttf"
LEAD_S     = 0.8                              # quiet lead-in kept before the first segment
FOLEY_LAG  = 0.03                             # foley sits this far after the key event: the
                                              # app paints the glyph a frame or two later, and
                                              # sound-matching-picture reads tighter than
                                              # sound-matching-keypress
FADE_S     = 1.1                              # end-of-video fade to black

# One recording per profile; each maxes out its format's frame rate.
PROFILES = {
	"video": dict(
		size=(1920, 1080), fps=60, mono_pt=19.5, ui_pt=11,  # 19.5pt = 1.5x the defined 13pt
		banner_fs=38, banner_pad=18, border=3, audio=True,
		bg="background41.jpg", bg_opacity=0.10, blur=10.0,
		transparent=False, opacity=0.95, banner_min=4.0,
	),
	"gif": dict(
		size=(960, 540), fps=50, mono_pt=13, ui_pt=10,      # the defined size, native 540p
		banner_fs=24, banner_pad=12, border=2, audio=False,
		bg="desktop.png", bg_opacity=0.75, blur=5.0,        # starts see-through over the desktop
		transparent=True, opacity=0.75, banner_min=3.0,
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
		self.fps      = profile["fps"]
		self.display  = args.display
		self.num      = self.display.lstrip(":")
		self.auth     = f"/tmp/cicd-gui-headless-{os.environ['USER']}/Xauthority-{self.num}"
		self.bin      = os.environ.get("SILK_BIN", str(REPO / "target/release/silkterm"))
		self.work     = Path(tempfile.mkdtemp(prefix="silk-demo-"))
		self.home     = self.work / "home"
		self.keep     = args.keep_work
		self.events   = []      # (epoch, kind) kind: key:NAME / rel:NAME / mouse:NAME
		self.banners  = []      # (epoch_start, epoch_end, text, pos)
		self.app      = None
		self.ff       = None
		self.flash_e  = 0.0     # wall-clock epoch of the white sync flash
		self.t0_e     = 0.0     # wall-clock epoch where trimmed content starts

	def env(self):
		e = dict(os.environ)
		e.update(DISPLAY=self.display, XAUTHORITY=self.auth, LIBGL_ALWAYS_SOFTWARE="1")
		return e

	def xdo(self, *a):
		subprocess.run(["xdotool", *a], env=self.env(), check=False,
			stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

	def start_display(self):
		# each profile records at its own resolution, so cycle the display; the
		# WM is ours (not gui-headless --wm): a dbus session lets xfconf pick a
		# quiet dark titlebar instead of xfwm4's built-in green
		gh = str(REPO / "cicd/utility/gui-headless.bash")
		e = dict(os.environ, CICD_HEADLESS_DISPLAY=self.display,
			CICD_HEADLESS_SIZE=f"{self.size[0]}x{self.size[1]}x24")
		subprocess.run([gh, "stop"], env=e, capture_output=True)
		run([gh, "start"], env=e)
		self.wm = subprocess.Popen(["dbus-run-session", "--", "sh", "-c",
			'xfconf-query -c xfwm4 -p /general/theme --create -t string -s "Arctodon-Dark"; '
			'xfconf-query -c xfwm4 -p /general/title_font --create -t string -s "Lato Bold 10"; '
			"exec xfwm4 --compositor=off --vblank=off"],
			env=self.env(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
		time.sleep(2.0)
		subprocess.run(["xsetroot", "-solid", "black"], env=self.env(), check=False)

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
			"-f", "x11grab", "-framerate", str(self.fps),
			"-video_size", f"{self.size[0]}x{self.size[1]}", "-i", self.display,
			"-c:v", "libx264", "-preset", "ultrafast", "-crf", "15",
			"-pix_fmt", "yuv420p", str(self.raw)],
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

	def launch_app(self, shell_cmd, cwd=None):
		e = self.env()
		# --norc/--noprofile below skips even Debian's /etc/bash.bashrc (a --rcfile
		# shell still reads it, and this box's spews real paths); the prompt comes
		# in via the environment instead. Gray prompt, rose user, sand host.
		# XDG_CONFIG_HOME lands on the fake home: the app finds config.toml there
		# AND gsettings reads the compiled dconf db (recording fonts), AND the
		# typed `nano ~/.config/silkterm/config.toml` edits the live config.
		e.update(SHELL="/bin/dash", HOME=str(self.home),
			XDG_CONFIG_HOME=str(self.home / ".config"),
			PATH=f"{self.home}/bin:{os.environ['PATH']}",
			PS1="\\[\\e[38;2;224;144;158m\\]juno\\[\\e[38;2;150;156;162m\\]@"
				"\\[\\e[38;2;222;178;134m\\]vela\\[\\e[38;2;150;156;162m\\]:\\w\\$ \\[\\e[0m\\]",
			HISTFILE="/dev/null")
		self.app = subprocess.Popen(
			[self.bin, "--fullscreen", "--shell", shell_cmd],
			env=e, cwd=cwd or str(self.home),
			stdout=open(self.work / "silk.log", "w"), stderr=subprocess.STDOUT)
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

	# --- control socket (the running instance's reload/wallpaper channel) -------
	def ctl(self, line):
		rund = os.environ.get("XDG_RUNTIME_DIR", tempfile.gettempdir())
		path = Path(rund) / f"silkterm-ctl-{self.app.pid}.sock"
		with socketmod.socket(socketmod.AF_UNIX) as s:
			s.settimeout(5)
			s.connect(str(path))
			s.sendall(line.encode() + b"\n")
			return s.recv(64).decode().strip()

	# --- event log -------------------------------------------------------------
	def ev(self, kind):
		self.events.append((time.time(), kind))

	def mouse_park(self):
		self.xdo("mousemove", str(self.size[0] - 4), str(self.size[1] - 4))

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
		# timestamp AFTER the send: the xdotool spawn latency then never skews the
		# foley, and the event epoch is the moment X actually got the key
		if ch == " ":
			self.rec.xdo("key", "--clearmodifiers", "space")
			self.rec.ev("key:SPACE")
		else:
			subprocess.run(["xdotool", "type", "--delay", "0", "--", ch],
				env=self.rec.env(), check=False,
				stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
			self.rec.ev(key_sound(ch))
		self.rec.events.append((time.time() + self.rng.uniform(0.05, 0.09), "rel:GENERIC"))

	def _backspace(self, n):
		for _ in range(n):
			time.sleep(self.rng.uniform(0.09, 0.16))
			self.rec.xdo("key", "--clearmodifiers", "BackSpace")
			self.rec.ev("key:BACKSPACE")

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
		self.rec.xdo("key", "--clearmodifiers", "Return")
		self.rec.ev("key:ENTER")
		self.rec.events.append((time.time() + 0.07, "rel:ENTER"))

	def key(self, keysym, sound="key:GENERIC_R0", hold=None):
		self.rec.xdo("key", "--clearmodifiers", keysym)
		self.rec.ev(sound)

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
		self.pos = (rec.size[0] - 4, rec.size[1] - 4)

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

	def circle(self, cx, cy, r, loops=2.0, dur=4.0):
		# lazy hand-circles (the "look around here" gesture)
		import math
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
		self.pos = (self.rec.size[0] - 4, self.rec.size[1] - 4)

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
	# mirrors the real defined config; only the demo-driven keys differ per
	# profile. background_image is a BARE filename next to config.toml - the
	# Settings dialog shows the value with any directory part resolved to an
	# absolute (temp) path, and a bare name is the one form it shows verbatim.
	cfgdir = home / ".config" / "silkterm"
	bgdir = cfgdir / "backgrounds"
	bgdir.mkdir(parents=True, exist_ok=True)
	shutil.copy2(BACKGNDS / "background41.jpg", cfgdir / "background41.jpg")
	shutil.copy2(BACKGNDS / "background45.jpg", bgdir / "background45.jpg")
	p = profile
	(cfgdir / "config.toml").write_text(f'''use_system_font = true
line_height_scale = 1.22
margin = 8.0
remember_size = false
columns = 160
rows = 48
transparent_background = {str(p["transparent"]).lower()}
opacity = {p["opacity"]}
background_image = "{p["bg"]}"
background_opacity = {p["bg_opacity"]}
background_fit = "zoom"
background_blur = {p["blur"]}
text_scrim = true
text_outline = 2.0
text_scrim_ramp = "gaussian"
cursor_size_height = 100
cursor_size_width = 25
cursor_animation = "pulse_vertical"
cursor_animation_input = "pause"
cursor_blink_rate_ms = 500
word_separators = "=,|:\\"' ()[]{{}}<>"
scrollback = 10000
scroll_tau_ms = 230.0
wheel_lines = 3.0
alt_scroll_lines = 3.0
output_ease_lines = 1.0
smooth_scroll_apps = true
theme = "SilkTerm"
theme_mode = "dark"
''')

# swap the demo-driven background keys in the live config, then hot-reload the
# running window through its control socket - this is what makes the Settings
# scene's OK visibly "apply". The app rewrote the file canonically at launch,
# so a plain line replace per key is safe.
def set_background(rec, image, opacity, blur):
	cfg = rec.home / ".config" / "silkterm" / "config.toml"
	text = cfg.read_text()
	for key, val in [("background_image", f'"{image}"'),
			("background_opacity", str(opacity)), ("background_blur", str(blur))]:
		text = re.sub(rf"(?m)^{key} = .*$", f"{key} = {val}", text)
	cfg.write_text(text)
	reply = rec.ctl("reload")
	if reply != "ok":
		log(f"WARNING: ctl reload replied {reply!r}")

def synth_desktop(work, out_png):
	# a generic dark-mode desktop (code editor + file manager) rendered from
	# rects only - it is always seen through background_blur, where colored bars
	# read exactly like syntax-highlighted code. Fixed seed: same desktop every run.
	rng = random.Random(7)
	d = ["fill #1c2129 roundrectangle 70,50 1210,930 10,10",
		"fill #262c36 roundrectangle 70,50 1210,92 10,10",
		"fill #e0655a circle 100,71 108,71",
		"fill #e2b054 circle 130,71 138,71",
		"fill #69bf65 circle 160,71 168,71",
		"fill #2c333f roundrectangle 200,58 330,92 6,6",
		"fill #20252d roundrectangle 336,58 466,92 6,6",
		"fill #171b22 rectangle 70,92 128,920",
		"fill #14181e rectangle 128,92 176,920"]
	for i in range(5):
		d.append(f"fill #39404d roundrectangle 86,{124 + i * 54} 112,{150 + i * 54} 5,5")
	palette = ["#7aa2f7", "#9ece6a", "#e0af68", "#bb9af7", "#7dcfff", "#a9b1d6", "#565f89"]
	y = 122
	indent = 0
	while y < 900:
		indent = max(0, min(5, indent + rng.choice([-2, -1, 0, 0, 1, 1])))
		x = 200 + indent * 34
		for _ in range(rng.randint(1, 3)):
			w = rng.randint(50, 240)
			c = rng.choice(palette)
			d.append(f"fill {c} roundrectangle {x},{y} {x + w},{y + 11} 5,5")
			x += w + rng.randint(14, 30)
			if x > 1080:
				break
		y += 22
	# file manager, partially tucked behind the terminal's right edge
	d += ["fill #20252e roundrectangle 1250,330 1858,1010 10,10",
		"fill #2a303b roundrectangle 1250,330 1858,372 10,10",
		"fill #e0655a circle 1276,351 1283,351",
		"fill #e2b054 circle 1302,351 1309,351",
		"fill #69bf65 circle 1328,351 1335,351",
		"fill #232933 rectangle 1250,372 1858,410",
		"fill #1a1f27 rectangle 1250,410 1395,1000"]
	for i in range(6):
		d.append(f"fill #242b36 roundrectangle 1262,{428 + i * 44} 1380,{454 + i * 44} 6,6")
	for row in range(4):
		for col in range(3):
			x, y = 1430 + col * 140, 444 + row * 136
			d.append(f"fill #3d4a61 roundrectangle {x},{y} {x + 84},{y + 62} 8,8")
			d.append(f"fill #2a3140 roundrectangle {x + 6},{y + 72} {x + 78},{y + 82} 3,3")
	# a small centered dock
	d.append("fill #10131a roundrectangle 660,1026 1260,1062 14,14")
	for i, c in enumerate(["#5580c8", "#69bf65", "#e2b054", "#bb9af7", "#7dcfff",
			"#e0655a", "#9ece6a", "#a9b1d6"]):
		x = 700 + i * 70
		d.append(f"fill {c} roundrectangle {x},1034 {x + 22},1056 6,6")
	run(["magick", "-size", "1920x1080", "gradient:#232c40-#0c0f16",
		"-draw", " ".join(d), str(out_png)])

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
	body = RUST_SCROLL * 5
	(src / "scroll.rs").write_text(body)
	(src / "main.rs").write_text('fn main() {\n\tpulsar::run();\n}\n')
	(src / "render.rs").write_text(RUST_SCROLL)

	# the fake home `ls ~/` lists: real entries, believable sizes (sparse files)
	for name in HOME_DIRS:
		(home / name).mkdir(parents=True, exist_ok=True)
	for name, size in HOME_DOTFILES + HOME_FILES:
		f = home / name
		f.touch()
		os.truncate(f, size)

	# `ls` wrapper: the on-camera alias resolves here first; the real listing
	# would print the real username as owner/group, so map it to the fake one
	bind = home / "bin"
	bind.mkdir(exist_ok=True)
	user = getpass.getuser()
	wrapper = bind / "ls"
	wrapper.write_text(f'#!/bin/dash\n/usr/bin/ls "$@" | sed "s/{user}/juno/g"\n')
	wrapper.chmod(0o755)
	# the typed `silkterm --wallpaper ...` client
	(bind / "silkterm").symlink_to(rec.bin)
	# pin nano to no-softwrap so the config scene's Down count is line-exact
	(home / ".nanorc").write_text("unset softwrap\nunset breaklonglines\n")

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

def prep_content(rec, rng):
	write_dconf(rec.home, rec.p)
	write_config(rec.home, rec.p)
	write_tree(rec, rng)
	synth_desktop(rec.work, rec.work / "desktop-src.png")
	dst = rec.home / ".config" / "silkterm" / "desktop.png"
	if rec.size[0] != 1920:
		run(["magick", str(rec.work / "desktop-src.png"),
			"-resize", f"{rec.size[0]}x{rec.size[1]}", str(dst)])
	else:
		shutil.copy2(rec.work / "desktop-src.png", dst)
	# the wallpaper the demo sets on camera: background45 toned down - at the
	# demo's high image opacity the raw file overwhelms the text (and costs a
	# fortune in gif bytes); darker + desaturated keeps the wow without the wash
	w, h = rec.size
	run(["magick", str(BACKGNDS / "background45.jpg"),
		"-resize", f"{w}x{h}^", "-gravity", "center", "-extent", f"{w}x{h}",
		"-modulate", "60,72",
		str(rec.home / ".config" / "silkterm" / "backgrounds" / "background45.jpg")])


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Settings dialog driving

DLG_PAD, DLG_BTN_W = 18, 76      # settings_ui.rs consts (ui scale is 1 on the Xvfb)

def open_settings(rec, t, m):
	rec.ev("key:GENERIC_R0")
	rec.xdo("key", "--clearmodifiers", "ctrl+comma")
	time.sleep(2.0)
	for _ in range(10):
		out = subprocess.run(["xdotool", "search", "--name", "Settings"],
			env=rec.env(), capture_output=True, text=True).stdout.strip()
		if out:
			dlg = out.split()[0]
			break
		time.sleep(0.5)
	else:
		return None
	# park it right of center so the live change shows on the terminal around it
	gx = rec.size[0] - 600 if rec.size[0] > 1200 else rec.size[0] - 580
	gy = 110 if rec.size[1] > 700 else 6
	rec.xdo("windowmove", dlg, str(max(0, gx)), str(gy))
	rec.xdo("windowactivate", dlg)
	time.sleep(1.0)
	return dlg

def dlg_geometry(rec, dlg):
	# xwininfo reports the CLIENT rect; xdotool's geometry includes the WM
	# frame, which put the computed OK point on the titlebar's border once
	out = subprocess.run(["xwininfo", "-id", dlg], env=rec.env(),
		capture_output=True, text=True).stdout
	def grab(pat):
		return int(re.search(pat + r":\s+(-?\d+)", out).group(1))
	return (grab(r"Absolute upper-left X"), grab(r"Absolute upper-left Y"),
		grab(r"Width"), grab(r"Height"))

def dlg_button_xy(rec, dlg, which):
	# Cancel/Apply/OK right-aligned along the bottom (settings_ui::buttons)
	x, y, w, h = dlg_geometry(rec, dlg)
	ok_cx = x + w - DLG_PAD - DLG_BTN_W // 2
	cy = y + h - DLG_PAD - 15
	off = {"ok": 0, "apply": DLG_BTN_W + 10, "cancel": 2 * (DLG_BTN_W + 10)}[which]
	return ok_cx - off, cy


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Segments

def seg_intro(r, t, m):
	with Banner(r, "SilkTerm - a smooth-scrolling, GPU-accelerated terminal"):
		time.sleep(1.4)
		t.cmd('alias ls="ls -lA --color --group-directories-first"', settle=0.9)
		time.sleep(0.6)

def seg_ls(r, t, m):
	with Banner(r, "Output never snaps - plain shell output glides"):
		t.cmd("ls ~/", settle=3.4)
		time.sleep(1.0)

def seg_settings(r, t, m):
	# on-camera: Transparency on, Opacity slid to 0.75, a look at the scrim
	# rows, OK. The moment OK lands, the generated desktop slides in behind
	# (config rewrite + socket reload) - the compositor's part is played by
	# background_blur, since a headless X has no compositor to do it live.
	with Banner(r, "Live Settings - changes apply on the spot"):
		dlg = open_settings(r, t, m)
		if not dlg:
			return
		t.key("Tab", "key:GENERIC_R0")            # -> Transparency checkbox
		time.sleep(0.9)
		t.key("space", "key:SPACE")               # on
		time.sleep(1.2)
		t.key("Tab", "key:GENERIC_R0")            # -> Opacity slider track
		time.sleep(0.7)
		for _ in range(20):                       # 0.95 -> 0.75, arrow step 0.01
			t.key("Left", "key:GENERIC_R0")
			time.sleep(0.10)
		time.sleep(0.9)
	with Banner(r, "Text readability enhancements - for background images and transparency"):
		x, y, w, h = dlg_geometry(r, dlg)
		m.circle(x + int(w * 0.45), y + int(h * 0.60), int(w * 0.22), loops=1.6, dur=4.4)
		time.sleep(0.4)
		bx, by = dlg_button_xy(r, dlg, "ok")
		m.move(bx, by, 1.0)
		time.sleep(0.4)
		m.click()
		time.sleep(0.7)                           # let the OK persist land first
		set_background(r, "desktop.png", 0.75, r.p["blur"])
		r.xdo("windowactivate", r.win)
		m.park()
		time.sleep(2.6)                           # linger: the desktop shows through

def seg_settings_gif(r, t, m):
	# gif starts see-through; here Transparency goes OFF, with a slow hand so
	# the other settings register, then OK snaps it opaque
	with Banner(r, "Live Settings - changes apply on the spot"):
		dlg = open_settings(r, t, m)
		if not dlg:
			return
		t.key("Tab", "key:GENERIC_R0")            # -> Transparency checkbox
		time.sleep(1.0)
		t.key("space", "key:SPACE")               # off
		time.sleep(1.4)
		bx, by = dlg_button_xy(r, dlg, "ok")
		x, y, w, h = dlg_geometry(r, dlg)
		m.move(x + int(w * 0.5), y + int(h * 0.45), 1.2)
		m.move(bx, by, 2.4)                       # the slow "taking it in" drift
		time.sleep(0.5)
		m.click()
		time.sleep(0.7)
		set_background(r, "background41.jpg", 0.10, r.p["blur"])
		r.xdo("windowactivate", r.win)
		m.park()
		time.sleep(2.0)

def seg_wallpaper(r, t, m):
	with Banner(r, "Set a per-window wallpaper straight from the shell"):
		t.cmd("silkterm --wallpaper ~/.config/silkterm/backgrounds/background45.jpg",
			settle=3.0)
		time.sleep(1.2)

def seg_nano(r, t, m):
	# scroll down to transparent_background (nano slides, line by line), flip it
	# to false, save, quit. The line number is read fresh: the app rewrote the
	# file canonically at launch and again when OK persisted.
	cfg = r.home / ".config" / "silkterm" / "config.toml"
	lineno = next((i + 1 for i, l in enumerate(cfg.read_text().splitlines())
		if re.match(r"transparent_background\s*=", l)), None)
	with Banner(r, "The config is plain TOML - and nano glides too", pos="top"):
		# cd first: a tilde path typed at the prompt reaches nano expanded, and
		# nano's titlebar would print the whole (temp) expansion on camera
		t.cmd("cd ~/.config/silkterm", settle=0.5, typos=0.0)
		t.cmd("nano config.toml", settle=1.8)
		if lineno:
			t.keys("Down", lineno - 1, hz=11.0)
			time.sleep(0.9)
			t.key("End")
			time.sleep(0.5)
			t._backspace(4)                       # true -> (gone)
			time.sleep(0.4)
			t.type("false", typos=0.0)
			time.sleep(1.0)
			t.key("ctrl+o")
			time.sleep(0.6)
			t.enter()
			time.sleep(0.8)
		t.key("ctrl+x")
		time.sleep(1.0)
		t.cmd("cd ~", settle=0.5, typos=0.0)

def seg_build(r, t, m):
	with Banner(r, "Bursts glide into place - never snap - at any size"):
		t.cmd("cd projects/pulsar", settle=0.6, typos=0.0)
		t.cmd("./build.sh", settle=7.5)          # covers the script's own runtime
		time.sleep(1.0)
		t.cmd("./test.sh", settle=3.0)
		time.sleep(0.8)

def seg_build_gif(r, t, m):
	with Banner(r, "Bursts glide into place - never snap"):
		t.cmd("cd projects/pulsar", settle=0.6, typos=0.0)
		t.cmd("./build.sh", settle=7.5)
		time.sleep(0.8)

def seg_wheel(r, t, m):
	with Banner(r, "Scrollback rides the wheel just as smoothly"):
		m.move(r.size[0] // 2, r.size[1] // 2, 0.7)
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

def seg_select(r, t, m):
	W, H = r.size
	with Banner(r, "Mouse selection: word, run, and block"):
		# fill the screen deterministically so the click targets always hold
		t.cmd("clear", settle=0.4, typos=0.0)
		t.cmd("tail -n 40 docs/render.log", settle=1.2)
		m.move(int(W * 0.22), int(H * 0.76), 0.8)
		m.double()                               # word
		time.sleep(1.3)
		m.drag(int(W * 0.07), int(H * 0.81), int(W * 0.55), int(H * 0.81), 1.0)
		time.sleep(1.2)
		r.xdo("keydown", "ctrl")                 # block
		m.drag(int(W * 0.09), int(H * 0.52), int(W * 0.38), int(H * 0.74), 1.2)
		r.xdo("keyup", "ctrl")
		time.sleep(1.4)
		m.move(int(W * 0.68), int(H * 0.43), 0.5)
		m.click(quiet=True)
		m.park()
		time.sleep(0.8)

def seg_outro(r, t, m):
	with Banner(r, "github.com/jim-collier/silkterm"):
		t.cmd("# smooth. silky. SilkTerm.", settle=0.5, typos=0.0)
		time.sleep(3.4)

SEGMENTS = {
	"video": [
		("intro",     seg_intro),
		("ls",        seg_ls),
		("settings",  seg_settings),
		("wallpaper", seg_wallpaper),
		("nano",      seg_nano),
		("build",     seg_build),
		("wheel",     seg_wheel),
		("less",      seg_less),
		("select",    seg_select),
		("outro",     seg_outro),
	],
	"gif": [
		("intro",     seg_intro),
		("ls",        seg_ls),
		("settings",  seg_settings_gif),
		("wallpaper", seg_wallpaper),
		("build",     seg_build_gif),
		("less",      seg_less),
		("outro",     seg_outro),
	],
}


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
		t_rel = epoch - rec.t0_e + FOLEY_LAG
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
##	Post: sync-flash location, trim, banners, border, fade, encode

def check_drift(rec, video_end_e):
	# the grab loop stamps CFR pts, so if the X server ever starves it the
	# video timeline compresses and every anchored epoch lands early - loud
	# warning beats silently drifted foley/banners
	dur = float(out_of(["ffprobe", "-v", "error", "-show_entries", "format=duration",
		"-of", "csv=p=0", str(rec.raw)]))
	expect = (video_end_e - rec.flash_e) + rec.flash_vt
	if abs(dur - expect) > max(0.5, expect * 0.02):
		log(f"WARNING: capture drift - raw {dur:.1f}s vs expected {expect:.1f}s; "
			"AV sync may be off (X server starved the grab loop?)")

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

def vf_chain(rec, work, trim, dur):
	# narration overlays (soft fade in/out), then the window frame, then the
	# closing fade. The frame: 1px near-black outline with a slate face inside -
	# square, quiet, reads as a window edge and nothing more.
	p = rec.p
	to_vt = lambda epoch: rec.flash_vt + (epoch - rec.flash_e)
	filters = []
	for i, (s_e, e_e, text, pos) in enumerate(rec.banners):
		s = max(0.0, to_vt(s_e) - trim)
		e = max(s + p["banner_min"], to_vt(e_e) - trim)
		tf = esc_drawtext(work, i, text)
		# "top" = upper-right, clear of nano/less title rows and left-aligned text
		x, y = ("w-text_w-48", "62") if pos == "top" else ("(w-text_w)/2", "h-{}".format(
			int(rec.size[1] * 0.135)))
		fade = (f"clip((t-{s:.3f})/0.35,0,1)*clip(({e:.3f}-t)/0.35,0,1)")
		filters.append(
			f"drawtext=fontfile={BANNER_TTF}:textfile={tf}:fontsize={p['banner_fs']}:"
			f"fontcolor=0xf2f4f8:box=1:boxcolor=0x0c0e12@0.55:boxborderw={p['banner_pad']}:"
			f"x={x}:y={y}:alpha='{fade}':enable='between(t,{s:.3f},{e:.3f})'")
	bw = p["border"]
	filters.append(f"drawbox=x=0:y=0:w=iw:h=ih:t={bw}:color=0x3b4046")
	filters.append(f"drawbox=x=0:y=0:w=iw:h=ih:t=1:color=0x15171a")
	filters.append(f"fade=t=out:st={max(0.0, dur - FADE_S):.3f}:d={FADE_S}")
	return ",".join(filters)

def encode_video(rec, work, out_mp4, video_end_e):
	rec.flash_vt = find_flash(rec.raw, work)
	log(f"sync flash at video t={rec.flash_vt:.3f}s")
	check_drift(rec, video_end_e)
	trim = rec.flash_vt + (rec.t0_e - rec.flash_e)
	dur = video_end_e - rec.t0_e
	vf = vf_chain(rec, work, trim, dur)
	rng = random.Random(1)
	audio = build_audio(rec, work, dur, rng)
	run(["ffmpeg", "-v", "error", "-y",
		"-ss", f"{trim:.3f}", "-i", str(rec.raw), "-i", str(audio),
		"-t", f"{dur:.3f}", "-vf", vf,
		"-c:v", "libx265", "-preset", "slow", "-crf", "20", "-pix_fmt", "yuv420p",
		"-tag:v", "hvc1", "-x265-params", "log-level=error",
		"-r", str(rec.fps), "-c:a", "aac", "-b:a", "160k",
		"-af", f"afade=t=out:st={max(0.0, dur - FADE_S):.3f}:d={FADE_S}",
		"-movflags", "+faststart", str(out_mp4)])
	return out_mp4

# a full-length 50fps 540p gif of nonstop smooth scrolling runs ~1 MB/s - far
# past what a README should carry (and what GitHub will render) - so the full
# gif goes to private/ and a same-fps highlight window is cut for the README
GIF_HL_START = 1.0
GIF_HL_DUR   = 20.0

def gif_pass(rec, work, out_gif, trim, dur):
	vf = vf_chain(rec, work, trim, dur)
	pal = work / "pal.png"
	cut = ["-ss", f"{trim:.3f}", "-t", f"{dur:.3f}"]
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(rec.raw),
		"-vf", f"{vf},fps={rec.fps},palettegen=stats_mode=diff:max_colors=128", str(pal)])
	run(["ffmpeg", "-v", "error", "-y", *cut, "-i", str(rec.raw), "-i", str(pal),
		"-lavfi",
		f"{vf},fps={rec.fps}[x];[x][1:v]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle",
		str(out_gif)])
	return out_gif

def encode_gif(rec, work, out_gif, video_end_e):
	# native 540p@50 - gif's ceiling - with a palette optimized on the actual
	# frames; bayer dither + rectangle diff keep the size sane
	rec.flash_vt = find_flash(rec.raw, work)
	log(f"sync flash at video t={rec.flash_vt:.3f}s")
	check_drift(rec, video_end_e)
	trim = rec.flash_vt + (rec.t0_e - rec.flash_e)
	dur = video_end_e - rec.t0_e
	gif_pass(rec, work, out_gif, trim, dur)
	hl = work / "demo-hl.gif"
	gif_pass(rec, work, hl, trim + GIF_HL_START, min(GIF_HL_DUR, dur - GIF_HL_START))
	return out_gif, hl


##•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
##	Output placement + rotation

def place_video(mp4, out_dir, no_rotate):
	out_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	dst = out_dir / f"silkterm-demo_{stamp}.mp4"
	shutil.copy2(mp4, dst)
	mb = dst.stat().st_size / (1 << 20)          # before rotation renames it
	if not no_rotate:
		inc = REPO / "cicd/utility/include/gfs-rotate.bash"
		subprocess.run(["bash", "-c",
			f'source "{inc}" && gfs_rotate "{out_dir}" silkterm-demo mp4'],
			check=False)
	log(f"video: {dst} ({mb:.1f} MiB)")

# above this, GitHub stops inlining the image and the repo carries dead weight
GIF_ASSET_MAX_MB = 12

def place_gif(full, hl, out_dir, no_rotate):
	# full recording -> private (rotated); highlight -> the README asset
	out_dir.mkdir(parents=True, exist_ok=True)
	stamp = time.strftime("%Y%m%d-%H%M%S")
	dst = out_dir / f"silkterm-demo_{stamp}.gif"
	shutil.copy2(full, dst)
	full_mb = dst.stat().st_size / (1 << 20)
	if not no_rotate:
		inc = REPO / "cicd/utility/include/gfs-rotate.bash"
		subprocess.run(["bash", "-c",
			f'source "{inc}" && gfs_rotate "{out_dir}" silkterm-demo gif'],
			check=False)
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
		rec.launch_app("/bin/bash --noprofile --norc -i")
		time.sleep(2.0)
		rec.t0_e = time.time() - LEAD_S

		t = Typist(rec, rng)
		m = Mouse(rec, rng)
		want = [s.strip() for s in args.segments.split(",") if s.strip()]
		for seg, fn in SEGMENTS[name]:
			if want and seg not in want:
				continue
			log(f"[{name}] segment: {seg}")
			fn(rec, t, m)
		time.sleep(1.5)
		video_end_e = time.time()

		rec.stop_capture()
		rec.kill_app()

		if name == "video":
			out = rec.work / "demo.mp4"
			encode_video(rec, rec.work, out, video_end_e)
			place_video(out, Path(args.out_dir), args.no_rotate)
		else:
			out = rec.work / "demo.gif"
			full, hl = encode_gif(rec, rec.work, out, video_end_e)
			place_gif(full, hl, Path(args.out_dir), args.no_rotate)
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
##		- 20260712 JC: Two recordings (1080p60 h265 + native 540p50 gif), window
##		  frame, generated see-through desktop via config+socket reload, new
##		  scene list (ls/settings/wallpaper/nano), Lato narration, tighter foley.
##		- 20260711 JC: Created.
