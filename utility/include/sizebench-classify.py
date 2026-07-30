#!/usr/bin/env python3
"""Install size and resident memory of a running terminal, with the graphics stack split out.

Reads /proc/<pid>/{maps,smaps} for a whole process tree and answers two questions:

  File+deps  the executable plus every shared library it needs beyond a base OS
  Mem        unique resident footprint - private pages summed, shared mappings counted once

Both leave out the GPU driver, which is not the terminal's cost: it is shared with the
compositor and every other accelerated program, and it dwarfs everything else (libLLVM
alone is over 120 MiB on this box). A terminal that draws on the CPU pays none of it, so
counting it would make the table measure the driver instead of the terminal.

Two things here are easy to get wrong and were got wrong the first time round:

  * ldd understates any program that dlopens. It lists three libraries for SilkTerm; the
    running process maps sixty-four. Always read a live process.

  * Classifying the driver by name misses what the driver pulls in - libstdc++ and libxml2
    exist in that map only because mesa wants them, and billing them to the terminal
    inflated a small one by a third. Classify by ldd CLOSURE from the graphics roots, and
    let the app closure win, since a library the app needs in its own right is the app's
    however many other things also happen to want it.
"""

import argparse
import json
import os
import re
import subprocess
import sys
from collections import defaultdict

MIB = 1048576.0

# Roots of the graphics stack. Everything these pull in is the driver's unless the app
# needs it independently.
#
# The trailing lookahead has to admit '_', because the vendor back ends are named
# libEGL_nvidia / libGLX_mesa / libdrm_intel and are dlopen'd rather than linked. A plain
# word boundary misses all of them, they then look like libraries nobody in the graphics
# stack needs, and seeding the app closure with one drags libLLVM in behind it - which is
# how the terminal ended up billed for 200 MiB of mesa.
GFX_RE = re.compile(
	r"""^(?:
		  libGL | libGLX | libGLdispatch | libGLESv\d | libEGL | libglapi
		| libgallium | libvulkan | libVkLayer | libgbm | libdrm
		| libnvidia | libcuda | libnvcuvid
		| .*_dri | swrast | iris | crocus | radeonsi | nouveau | zink | virtio_gpu | lvp
		| libxcb-dri\d | libxcb-present | libxcb-sync | libxcb-glx | libxcb-dri
		| libxshmfence | libpciaccess | libsensors
	)(?=[._-]|$)""",
	re.X,
)

# Present on any base install, so not something a terminal makes you install.
BASE_RE = re.compile(
	r"^(?:ld-linux.*|libc|libm|libdl|librt|libpthread|libgcc_s|libresolv|libnsl|libutil)\b"
)


def base_name(path):
	"""Strip the version tail so libX11.so.6.4.0 and libX11.so.6 compare equal."""
	name = os.path.basename(path)
	cut = name.find(".so")
	return name[:cut] if cut >= 0 else name


def is_library(path):
	return ".so" in os.path.basename(path)


def needed(path, cache={}):
	"""DT_NEEDED entries of one ELF file."""
	if path in cache:
		return cache[path]
	out = []
	try:
		raw = subprocess.run(
			["objdump", "-p", path], capture_output=True, text=True, timeout=30
		).stdout
		out = re.findall(r"^\s*NEEDED\s+(\S+)", raw, re.M)
	except Exception:
		pass
	cache[path] = out
	return out


def closure(roots, resolve):
	"""Everything reachable from roots through DT_NEEDED, as real paths."""
	seen, stack = set(), list(roots)
	while stack:
		path = stack.pop()
		if path in seen:
			continue
		seen.add(path)
		for soname in needed(path):
			target = resolve(soname)
			if target and target not in seen:
				stack.append(target)
	return seen


def read_maps(pid):
	"""Distinct real paths this process has mapped from disk."""
	paths = set()
	try:
		with open(f"/proc/{pid}/maps") as fh:
			for line in fh:
				parts = line.split(None, 5)
				if len(parts) < 6:
					continue
				path = parts[5].strip()
				if path.startswith("/") and not path.startswith(("/dev/", "/memfd", "/SYSV")):
					try:
						paths.add(os.path.realpath(path))
					except OSError:
						pass
	except OSError:
		pass
	return paths


def read_smaps(pid):
	"""Per-mapping private and shared resident bytes, keyed by backing file ('' = anonymous)."""
	rows = []
	path, priv, shared = "", 0, 0
	try:
		with open(f"/proc/{pid}/smaps") as fh:
			for line in fh:
				if re.match(r"^[0-9a-f]+-[0-9a-f]+ ", line):
					if path is not None:
						rows.append((path, priv, shared))
					parts = line.split(None, 5)
					path = parts[5].strip() if len(parts) >= 6 else ""
					priv = shared = 0
				elif line.startswith("Private_"):
					priv += int(line.split()[1]) * 1024
				elif line.startswith("Shared_"):
					shared += int(line.split()[1]) * 1024
		rows.append((path, priv, shared))
	except OSError:
		return []
	return rows


def measure(pids, exe_paths, verbose=False):
	# Every library any process in the tree has mapped.
	mapped = set()
	for pid in pids:
		mapped |= read_maps(pid)
	libs = {p for p in mapped if is_library(p)}

	by_name = {}
	for p in libs:
		by_name.setdefault(base_name(p), p)

	def resolve(soname):
		"""Prefer the copy this process actually mapped, so a bundled lib wins over a system one."""
		hit = by_name.get(base_name(soname))
		if hit:
			return hit
		for root in ("/usr/lib/x86_64-linux-gnu", "/lib/x86_64-linux-gnu", "/usr/lib", "/lib"):
			cand = os.path.join(root, soname)
			if os.path.exists(cand):
				return os.path.realpath(cand)
		return None

	gfx_named = {p for p in libs if GFX_RE.match(base_name(p))}
	driver_closure = closure(gfx_named, resolve) | gfx_named

	# Anything that is neither named like a graphics library nor pulled in by one is
	# unambiguously the app's, and seeds the app's own closure. Order matters: libX11 is
	# in the driver closure too (mesa needs it), but the app needs it in its own right, so
	# the app closure wins. Graphics libraries are excluded from the seed set by name as
	# well as by closure, or a dlopen'd back end nobody links against seeds it instead.
	app_roots = [p for p in exe_paths if os.path.exists(p)]
	app_roots += [p for p in libs if p not in driver_closure]
	app_closure = closure(app_roots, resolve)

	driver_libs = {p for p in driver_closure if p not in app_closure and is_library(p)}
	app_libs = {
		p for p in libs
		if p not in driver_libs and not BASE_RE.match(base_name(p))
	}

	def disk(paths):
		total = 0
		for p in paths:
			try:
				total += os.stat(p).st_size
			except OSError:
				pass
		return total

	exe_bytes = disk(exe_paths)
	deps_bytes = disk(app_libs - set(exe_paths))

	# Resident: private pages are per process, shared mappings are counted once across the
	# tree (per file, the largest any one process holds). Summing RSS instead would charge
	# a multi-process terminal several times over for the same pages.
	priv = defaultdict(int)
	shared_max = defaultdict(int)
	for pid in pids:
		per_file = defaultdict(int)
		for path, pv, sh in read_smaps(pid):
			key = os.path.realpath(path) if path.startswith("/") else ""
			priv[key] += pv
			per_file[key] += sh
		for key, val in per_file.items():
			shared_max[key] = max(shared_max[key], val)

	app_mem = driver_mem = 0
	for key in set(priv) | set(shared_max):
		total = priv.get(key, 0) + shared_max.get(key, 0)
		if key in driver_libs:
			driver_mem += total
		else:
			app_mem += total

	result = {
		"pids": sorted(pids),
		"exe_mib": exe_bytes / MIB,
		"deps_mib": deps_bytes / MIB,
		"file_deps_mib": (exe_bytes + deps_bytes) / MIB,
		"mem_mib": app_mem / MIB,
		"driver_mem_mib": driver_mem / MIB,
		"driver_disk_mib": disk(driver_libs) / MIB,
		"n_driver_libs": len(driver_libs),
		"n_app_libs": len(app_libs),
	}
	if verbose:
		result["app_libs"] = sorted(
			((os.stat(p).st_size / MIB, p) for p in app_libs - set(exe_paths)),
			reverse=True,
		)
		result["driver_libs"] = sorted(
			((os.stat(p).st_size / MIB, p) for p in driver_libs), reverse=True
		)
	return result


def main():
	ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
	ap.add_argument("pids", nargs="+", type=int, help="every pid in the terminal's tree")
	ap.add_argument("--exe", action="append", default=[], help="executable, repeatable")
	ap.add_argument("--payload", help="extracted bundle dir, counted instead of the executable")
	ap.add_argument("--verbose", action="store_true", help="itemize both library sets")
	ap.add_argument("--json", action="store_true")
	ap.add_argument("--summary", action="store_true",
	                help="also print one RESULT line, for the wrapper to read")
	args = ap.parse_args()

	exes = [os.path.realpath(e) for e in args.exe]
	if not exes:
		for pid in args.pids:
			try:
				exes.append(os.path.realpath(f"/proc/{pid}/exe"))
			except OSError:
				pass
		exes = list(dict.fromkeys(exes))[:1]

	res = measure(args.pids, exes, verbose=args.verbose)

	# A self-contained bundle has no meaningful "executable" - what you install is the
	# whole extracted payload, plus the system libraries it still borrows.
	if args.payload:
		total = 0
		for root, _, files in os.walk(args.payload):
			for f in files:
				try:
					total += os.lstat(os.path.join(root, f)).st_size
				except OSError:
					pass
		res["exe_mib"] = total / MIB
		res["file_deps_mib"] = res["exe_mib"] + res["deps_mib"]

	if args.json:
		print(json.dumps(res, indent=2))
		return 0

	print(f"  processes      {len(res['pids'])}  {res['pids']}")
	print(f"  file size      {res['exe_mib']:8.1f} MiB")
	print(f"  own deps       {res['deps_mib']:8.1f} MiB  ({res['n_app_libs']} libraries)")
	print(f"  File+deps      {res['file_deps_mib']:8.1f} MiB")
	print(f"  Mem            {res['mem_mib']:8.1f} MiB")
	print(f"  driver (excl)  {res['driver_mem_mib']:8.1f} MiB resident, "
	      f"{res['driver_disk_mib']:.1f} MiB on disk, {res['n_driver_libs']} libraries")
	if args.verbose:
		print("\n  billed to the terminal:")
		for size, path in res["app_libs"]:
			print(f"    {size:7.2f}  {path}")
		print("\n  billed to the driver:")
		for size, path in res["driver_libs"][:20]:
			print(f"    {size:7.2f}  {path}")
	if args.summary:
		print(f"RESULT file={res['exe_mib']:.1f} filedeps={res['file_deps_mib']:.1f} "
		      f"mem={res['mem_mib']:.1f} driver={res['driver_mem_mib']:.1f}")
	return 0


if __name__ == "__main__":
	sys.exit(main())
