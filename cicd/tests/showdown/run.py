#!/usr/bin/env python3

##	- Purpose:
##		The README's showdown table only holds figures measured the same way as the
##		rows beside them. A quick run, a scaled run and a run at another grid each
##		used to rewrite their row anyway. This drives both table writers with the
##		terminal and the measuring faked out, against a scratch README.
##	- History: At bottom of file.

##	Copyright © 2026 Jim Collier [ID: 2უNაɘ«҂թȹɤξπ๙¿ձϖ]
##	SPDX-License-Identifier: GPL-2.0-or-later

import importlib.util
import io
import shutil
import sys
import tempfile
from contextlib import redirect_stdout
from pathlib import Path
from types import SimpleNamespace

ME_DIR = Path(__file__).resolve().parent
REPO = ME_DIR.parents[2]
UTILITY = REPO / "utility"

failures = 0
def check(what, ok, detail=""):
	global failures
	if ok:
		print(f"  ok   {what}")
	else:
		print(f"  FAIL {what}{': ' + detail if detail else ''}")
		failures += 1

def load(name, path):
	spec = importlib.util.spec_from_file_location(name, path)
	mod = importlib.util.module_from_spec(spec)
	spec.loader.exec_module(mod)
	return mod

scratch = Path(tempfile.mkdtemp(prefix="silk-showdown-"))
readme = scratch / "README.md"
shutil.copyfile(REPO / "README.md", readme)
original = readme.read_text(encoding="utf-8")

## termbench.py, with everything that touches a terminal replaced.
tb = load("termbench", UTILITY / "include/termbench.py")
saved = []

class FakeConsole:
	def __enter__(self): return self
	def __exit__(self, *exc): return False
	def emit(self, text): pass

class Tty:
	@staticmethod
	def isatty(): return True

grid = [(160, 42)]
tb.Console = FakeConsole
tb.sys = SimpleNamespace(stdout=Tty, stdin=Tty, stderr=io.StringIO())
tb.time = SimpleNamespace(perf_counter=lambda: 0.0, sleep=lambda s: None)
tb.terminal_size = lambda: grid[0]
tb.build_payload = lambda scene, scale, cells: (b"x" * 1000, 1000, 1000, 10)
tb.harness_ceiling = lambda blob: 0.0
tb.run_scene = lambda console, scene, blob, reps, quiet: ([0.001, 0.001], True)
tb.save = lambda records: saved.extend(records) or "memory"
tb.load = lambda: list(saved)
tb.readme_path = lambda: str(readme)

def bench(*args):
	with redirect_stdout(io.StringIO()) as out:
		tb.main(["--scene", "ascii", "--label", "XTerm/999", *args])
	return out.getvalue()

def xterm_row():
	return next((ln for ln in readme.read_text(encoding="utf-8").splitlines()
	             if "| XTerm |" in ln), "")

before = xterm_row()
out = bench("--quick")
check("a quick run leaves the table alone", xterm_row() == before, xterm_row())
check("and says why", "not touched: a quick run" in out, out)
out = bench("--scale", "0.5")
check("a scaled run leaves the table alone", xterm_row() == before, xterm_row())
grid[0] = (100, 30)
out = bench()
check("a run at another grid leaves the table alone", xterm_row() == before, xterm_row())
check("and names the grid", "grid 100x30" in out, out)
grid[0] = (160, 42)
bench("--history", "--quick")
check("a quick history refresh leaves the table alone", xterm_row() == before, xterm_row())
bench()
check("a full run at the table's grid still writes its row", "| 999 |" in xterm_row(), xterm_row())

## The writer puts back a table it has nothing new for exactly as it was.
block = original.split(tb.README_BEGIN, 1)[1].split(tb.README_END, 1)[0]
again = tb.readme_table(block, [])
check("an unchanged table is rewritten byte for byte",
	again == tb.README_BEGIN + block + tb.README_END)

## update-showdown.py: a quick or --any-size run passes nothing on to be written.
us = load("update_showdown", UTILITY / "update-showdown.py")
calls = []
us.run_plain = lambda cmd: calls.append(cmd) or True
us.run_capturing = lambda cmd: calls.append(cmd) or ["RESULT filedeps=1.0 mem=2.0"]
us.terminal_grid = lambda: us.SPEED_GRID

def showdown(*args):
	calls.clear()
	with redirect_stdout(io.StringIO()):
		us.main(list(args))
	return calls

def termbench_cmd(cmds):
	return next((c for c in cmds if c[1].endswith("termbench.py")), [])

def wrote_size(cmds):
	return any(c[1].endswith("showdown-readme.py") for c in cmds)

cmd = termbench_cmd(showdown("--speed-only", "--quick", "--label", "XTerm"))
check("a quick run here tells termbench not to write", "--no-readme" in cmd, str(cmd))
cmd = termbench_cmd(showdown("--speed-only", "--any-size", "--label", "XTerm"))
check("an --any-size speed run here tells termbench not to write", "--no-readme" in cmd, str(cmd))
cmd = termbench_cmd(showdown("--speed-only", "--label", "XTerm"))
check("a full speed run here may write", cmd and "--no-readme" not in cmd, str(cmd))
check("an --any-size size run writes no size figure",
	not wrote_size(showdown("--size-only", "--any-size", "--label", "XTerm")))
us.terminal_grid = lambda: us.SIZE_GRID
check("a size run at the size grid still writes its figure",
	wrote_size(showdown("--size-only", "--label", "XTerm")))

shutil.rmtree(scratch, ignore_errors=True)
if failures:
	print(f"{failures} failed")
	sys.exit(1)
print("all passed")

##	History:
##		- 20260917 JC: Created.
