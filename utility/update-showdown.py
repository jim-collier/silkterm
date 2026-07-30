#!/usr/bin/env python3

##	Purpose:
##		Refresh the README "Terminal showdown" table. One entry point over everything
##		that feeds it, because the parts are easy to run inconsistently: they measure
##		different things, at different grid sizes, on different displays.
##
##		Two ways in:
##
##		THIS TERMINAL, any OS. Runs the throughput tool inside whatever terminal you
##		are sitting in and writes its speed row. Needs nothing but Python 3 and a tty,
##		so it is the only way to measure the terminals that exist solely on Windows or
##		macOS. This is what you get if you name no terminal.
##
##		THE RIGS, Linux only. Bring up a display, launch a named terminal into it, fit
##		it to a fixed grid, measure, tear down. Two of them:
##
##		  speed  include/termbench-run.bash   160x42, headless sway on the real GPU
##		  size   include/sizebench-run.bash   100x30, private Xvfb
##
##		The grids differ deliberately and must not be unified: the speed figure wants a
##		realistic working grid, while memory scales with the surface, so the size rows
##		are taken small and identical. The same SilkTerm binary reads 38 MiB heavier at
##		its default geometry than at 100x30.
##
##		Nothing is published unless it is comparable. Re-measure a terminal already in
##		the table before adding a new one: the rig reproduced SilkTerm's speed within
##		0.6% and its memory within 1.3%, and a figure taken any other way does not
##		belong beside the existing rows. Figures from a second machine are a separate
##		question again - see include/showdown-README.md on calibrating one.
##
##	Usage:
##		utility/update-showdown.py                      measure this terminal
##		utility/update-showdown.py --label 'name'       ... naming it yourself
##		utility/update-showdown.py --term alacritty     drive both rigs (Linux)
##		utility/update-showdown.py --all
##		utility/update-showdown.py --term kitty --size-only
##		utility/update-showdown.py --list
##
##	History:
##		20260730 Written, to drive both shootout rigs from one place.
##		20260730 Ported from shell so it runs on Windows too, and absorbed the
##		         measure-this-terminal path, which had no wrapper before.

import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
INCLUDE = os.path.join(HERE, "include")
REPO = os.path.dirname(HERE)
README = os.path.join(REPO, "README.md")

#	key: README row name, then which rigs can drive it here. A terminal the size rig has
#	no recipe for still gets its speed row; the size columns are left as they were.
TERMS = [
	("silkterm",   "SilkTerm +candy", "both"),
	("silkplain",  "SilkTerm plain",  "both"),
	("alacritty",  "Alacritty",       "both"),
	("kitty",      "kitty",           "both"),
	("xfce4",      "XFCE4 Terminal",  "both"),
	("terminator", "Terminator",      "both"),
	("xterm",      "XTerm",           "size"),
	("gnome",      "GNOME Terminal",  "speed"),
	("wezterm",    "WezTerm",         "speed"),
]

LETTERBOX = "-" * 78


#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
#	Output
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

#	Matches the house style of cicd.bash and the rigs: a bracketed status line, repeat
#	blanks collapsed, so the blank-line rhythm does the visual grouping.
_last_blank = [False]


def echo_clean(text=""):
	if text:
		print(text)
		_last_blank[0] = False
	elif not _last_blank[0]:
		print()
		_last_blank[0] = True


def echo(text=""):
	echo_clean("[ %s ]" % text if text else "")


def section(text):
	echo_clean()
	echo_clean(LETTERBOX)
	echo(text)


def die(text):
	echo_clean()
	sys.stdout.flush()                             ## or the reason lands above its own output
	print("[ FAILED: %s ]" % text, file=sys.stderr)
	sys.exit(1)


#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
#	Running the parts
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

def run_plain(cmd):
	"""Run a child on this terminal's own stdio, and say whether it worked.

	The throughput tool stops its clock on the terminal's reply, so its stdout has to
	stay on the tty. Capturing it would time a pipe instead.
	"""
	try:
		return subprocess.call(cmd) == 0
	except OSError as err:
		echo("WARNING: could not run %s: %s" % (os.path.basename(cmd[0]), err))
		return False


def run_capturing(cmd):
	"""Run a child, echoing its output as it arrives, and return the lines.

	The rig's own output is the record of what was measured, so it stays on screen; the
	summary line is picked out of the same stream afterwards.
	"""
	try:
		proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
		                        text=True, bufsize=1)
	except OSError as err:
		echo("WARNING: could not run %s: %s" % (os.path.basename(cmd[0]), err))
		return []
	lines = []
	for line in proc.stdout:
		sys.stdout.write(line)
		sys.stdout.flush()
		lines.append(line.rstrip("\n"))
	proc.wait()
	_last_blank[0] = False
	return lines


def measure_here(reps, quick, label, write_readme):
	"""Measure the terminal this is running inside."""
	cmd = [sys.executable, os.path.join(INCLUDE, "termbench.py")]
	if quick:
		cmd.append("--quick")
	else:
		cmd += ["--reps", str(reps)]
	if label:
		cmd += ["--label", label]
	if not write_readme:
		#	Same meaning as on the rig path: measure and print, record nothing anywhere.
		cmd.append("--no-save")
	return run_plain(cmd)


def measure_speed(key, row, reps, write_readme):
	cmd = [os.path.join(INCLUDE, "termbench-run.bash"),
	       "--term", key, "--reps", str(reps), "--label", row]
	if not write_readme:
		cmd.append("--no-save")
	if not run_plain(cmd):
		echo("WARNING: speed run failed for %s" % key)


def measure_size(key, row, write_readme):
	"""File+deps and Mem in MiB, or None if the rig could not say."""
	lines = run_capturing([os.path.join(INCLUDE, "sizebench-run.bash"), "--term", key])
	result = next((l for l in lines if l.startswith("RESULT ")), "")
	if not result:
		echo("WARNING: no result from the size rig for %s" % key)
		return None

	got = dict(re.findall(r"(\w+)=([0-9.]+)", result))
	if "filedeps" not in got or "mem" not in got:
		echo("WARNING: could not read the result line for %s" % key)
		return None

	file_deps, mem = float(got["filedeps"]), float(got["mem"])
	if write_readme:
		wrote = run_plain([sys.executable, os.path.join(INCLUDE, "showdown-readme.py"),
		                   "--readme", README, "--terminal", row,
		                   "--file-deps", "%.1f" % file_deps, "--mem", "%.1f" % mem])
		if not wrote:
			echo("WARNING: could not write the %s row" % row)
	return file_deps, mem


#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••
#	Entry
#•••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••••

def header_text():
	"""The comment header, which is also the help text.

	Read by prefix rather than searched for a first line: a pattern matching that line
	also matches itself, which is how the shell version printed its own source twice.
	"""
	out = []
	with open(os.path.abspath(__file__)) as fh:
		next(fh)                                   ## the interpreter line
		for line in fh:
			if line.startswith("##"):
				out.append(line[2:].rstrip())
			elif out:
				break
	return "\n".join(out).strip("\n")


def show_list():
	echo_clean("  key          README row                rigs")
	for key, row, rigs in TERMS:
		echo_clean("  %-12s %-25s %s" % (key, row, rigs))


def main(argv):
	ap = argparse.ArgumentParser(add_help=False)
	ap.add_argument("--term", action="append", default=[], metavar="KEY")
	ap.add_argument("--all", action="store_true")
	ap.add_argument("--here", action="store_true")
	ap.add_argument("--speed-only", action="store_true")
	ap.add_argument("--size-only", action="store_true")
	ap.add_argument("--reps", type=int, default=6, metavar="N")
	ap.add_argument("--quick", action="store_true")
	ap.add_argument("--label", default="", metavar="NAME")
	ap.add_argument("--no-readme", action="store_true")
	ap.add_argument("--list", action="store_true")
	ap.add_argument("-h", "--help", action="store_true")
	args = ap.parse_args(argv)

	if args.help:
		print(header_text())
		return 0
	if args.list:
		show_list()
		return 0

	keys = [k for k, _, _ in TERMS] if args.all else list(args.term)
	if args.here and keys:
		die("--here measures the terminal you are in, so it takes no --term")

	write_readme = not args.no_readme

	#	Naming no terminal means the one measurement that needs no rig, which is also the
	#	only one available off Linux.
	if not keys:
		if args.size_only:
			die("size and memory need a named terminal and a rig")
		if args.quick:
			echo("NOTE: a quick run is aggregated separately and never reaches the table")
		section("Speed: this terminal")
		ok = measure_here(args.reps, args.quick, args.label, write_readme)
		echo_clean()
		echo("README updated - check the diff before committing"
		     if write_readme and ok else "nothing written")
		echo_clean()
		return 0 if ok else 1

	if not sys.platform.startswith("linux"):
		die("the rigs need a Linux display; name no terminal to measure this one")

	known = {key: (row, rigs) for key, row, rigs in TERMS}
	for key in keys:
		if key not in known:
			show_list()
			die("unknown key '%s'" % key)

	measured = []
	for key in keys:
		row, rigs = known[key]
		if not args.size_only and rigs in ("both", "speed"):
			section("Speed: %s" % row)
			measure_speed(key, row, args.reps, write_readme)
		if not args.speed_only and rigs in ("both", "size"):
			section("Size and memory: %s" % row)
			got = measure_size(key, row, write_readme)
			if got:
				measured.append((row,) + got)

	if measured:
		section("Size and memory measured")
		echo_clean("  %-25s %10s %10s" % ("Terminal", "File+deps", "Mem"))
		for row, file_deps, mem in measured:
			echo_clean("  %-25s %10.1f %10.1f" % (row, file_deps, mem))

	echo_clean()
	echo("README updated - check the diff before committing" if write_readme
	     else "nothing written (--no-readme)")
	echo_clean()
	return 0


if __name__ == "__main__":
	sys.exit(main(sys.argv[1:]))
