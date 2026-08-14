# Terminal showdown rigs

Everything behind the README's "Terminal showdown" table. One entry point, `utility/update-showdown.py`, over two rigs that measure different things:

```sh
utility/update-showdown.py                               # measure this terminal, any OS
utility/update-showdown.py --list
utility/update-showdown.py --term alacritty              # both rigs, writes the table
utility/update-showdown.py --term kitty --size-only
utility/update-showdown.py --all --no-readme             # measure everything, write nothing
```

| | Rig | Grid | Display | Columns it owns |
| :--- | :--- | :--- | :--- | :--- |
| speed | `termbench-run.bash` | 160x42 | headless sway on the real GPU | the width classes and the score |
| size | `sizebench-run.bash` | 100x30 | private Xvfb | File+deps and Mem |

Naming no terminal takes the third path, which needs no rig at all: measure whatever terminal you are sitting in, from inside it. That is the only way to measure a terminal that exists solely on Windows or macOS, and the only mode that works off Linux. Both halves are available that way, but only one at a time, since the two want different window sizes.

**The two grids differ deliberately and must not be unified.** Speed wants a realistic working grid. Memory scales with the surface - the same SilkTerm binary reads 38 MiB heavier at its default geometry than at 100x30 - so the size rows are taken small and identical.

Neither rig is wired into cicd. They take minutes, need a GPU, and republishing numbers on every build would churn the README. Run them by hand when a row needs adding or refreshing.

The two writers own disjoint columns and key on the terminal name, so they never fight: `termbench.py` refreshes the speed cells, `showdown-readme.py` the size cells, and everything else in the table stays exactly as written by hand.

## Speed: what the number means

Ingest and keep-up throughput - "why does it bog down when something dumps text" - **not** rasterization. At 160x42 only ~3400 cells are ever visible, so most of a 100 MB stream is parsed, stored and scrolled past without being drawn. The clock stops when the terminal answers a Primary DA query (`ESC[c`), which it can only answer once it has worked through everything queued.

A terminal that never answers cannot be timed this way. `termbench.py` records that as `synced: false` and refuses to publish the row, so an unbarriered figure can never reach the table - it would be timing a timeout. Hyper is the standing example.

### The rig is not neutral - this is the whole reason it exists

Measured 20260730, same terminal, same grid, three rigs:

| Rig | SilkTerm ascii | xterm ascii |
| :--- | ---: | ---: |
| Software GL (Xvfb + llvmpipe) | 45 MB/s | ~29 MB/s |
| VirtualGL on Xvfb | 76 MB/s | ~29 MB/s |
| Headless sway on the discrete GPU | 88 MB/s | 28.3 MB/s |

GPU-accelerated terminals swing by a factor of two; CPU-rendered ones do not move at all. So a table assembled from mixed rigs can rank the wrong terminal first - a VirtualGL run would have published a GTK terminal as faster than SilkTerm. **Every published row must come from one rig.** Do not assume a rig is neutral because one terminal reproduces on it.

Headless sway is used rather than the live desktop so the measurement neither disturbs nor is disturbed by whatever the machine is actually doing.

### Shortening a run

Use `--reps`. Fewer repetitions of the same payloads leaves the measured rate directly comparable and only widens the confidence interval - Tabby sits in the published table at 4 reps.

Never use `--scale`. Shrinking the payload changes what is measured: Hyper reads 32 MB/s at `--scale 0.05` but roughly 3 MB/s at full size, a tenfold difference. `--quick` is also unsuitable for publishing: `mode` is part of the comparability key, so quick runs are aggregated separately and never reach the table.

Watch the `CV%` column. On a machine that is also being used for other work, a run that got stepped on shows up there. Published rows have run 1-4%; re-run anything much worse.

## Size and memory: what the numbers mean

**File+deps** is the executable plus every shared library the running process actually maps, minus the base OS runtime (libc, libm, libgcc, the loader) and minus the graphics stack. Self-contained bundles count their extracted payload plus the system libraries they still borrow - a different basis to a packaged terminal's, deliberately, because it is the honest answer to "what does installing this cost" for each packaging style.

**Mem** is the unique resident footprint of the whole process tree: private pages summed per process, plus each shared mapping counted once. It is neighbor-independent and free of intra-tree double counting, which the obvious alternatives are not - summed RSS charges a multi-process terminal several times for the same pages, and PSS divides shared pages by however many unrelated programs happen to be running.

### Three ways to get this wrong

**`ldd` understates any program that dlopens.** It lists three libraries for SilkTerm; the running process maps sixty-four. Always read a live process, never the binary on disk.

**Name-matching the driver misses what the driver drags in.** libstdc++ and libxml2 appear in that map only because mesa wants them; billing them to the terminal inflated SilkTerm's install size by a third. Classify by ldd *closure* from the graphics roots instead, and let the app closure win - a library the app needs in its own right is the app's however many other things also want it.

The name match still has to find the graphics *roots*, and the vendor back ends are called `libEGL_nvidia`, `libGLX_mesa`, `libdrm_intel`. A pattern anchored with a word boundary misses every one of them; they then look like libraries nothing in the graphics stack needs, and seeding the app closure with one drags libLLVM in behind it. That single character put 200 MiB of mesa on the terminal's bill.

**Window size moves the memory number a long way**, for the same reason the two rigs use different grids. Every row must be measured at the same grid or the column is meaningless.

### Reproduce before publishing

The rows in the table were measured across several sessions, so a new row is only comparable if the rig reproduces an old one. Controls re-run when the size rig was written:

| Check | Published | This rig |
| :--- | ---: | ---: |
| xterm File+deps | 6.0 | 6.0 |
| xterm driver excluded | 0.0 | 0.0 |
| SilkTerm driver excluded | 108 | 108.3 |
| SilkTerm Mem | 121.4 | 119.4 |
| SilkTerm File+deps | 17.4 | 16.8 |
| SilkTerm ascii throughput | 93.23 | 92.64 |

xterm is the useful control precisely because it draws on the CPU and should classify *nothing* as driver. SilkTerm is the other end - a third of a gigabyte of mapped graphics libraries that all has to land on the right side of the line.

The published figures are left as they were rather than refreshed to a later pass. They sit inside the drift the table already warns about, and rewriting one row's numbers while the others keep their originals would make the column less consistent, not more. Expect a few MiB either way: libraries load on demand, and GTK terminals are the worst for it - gnome-terminal moved 28.5 to 53.6 between two runs of one pass, because the first measured the thin client wrapper rather than `gnome-terminal-server`.

## Per-terminal notes

Most terminals need nothing but their key. The awkward ones, and why:

- **gnome-terminal** never resizes with the compositor output, so it is the one terminal told its geometry directly (`--geometry`). Its first measured run silently came out at 180x45 and had to be redone - always confirm the fitted grid in the output.
- **xterm** is X11-only. Xwayland fails on this rig (`/tmp/.X11-unix` ownership), so its speed figure was taken on X11, and its cross-rig agreement (28.8 vs 29.2, 1.3%) is what justifies publishing it beside Wayland rows. The size rig runs on Xvfb anyway, so it needs nothing special there.
- **WezTerm** 20240203 silently falls back to X11 under sway 1.10 despite `enable_wayland`. It is parser-bound and agreed within 1.7% across rigs, so its figure holds anywhere.
- **Hyper** rewrites `~/.hyper.js` on every launch, so it has to be written fresh per run. It never answers the barrier, so it cannot be timed at all.
- **Tabby** ignores `SHELL` and offers no profile hook that takes. Dismiss its Welcome tab once by clicking "Close and never show again", then hook the run through the login shell's `.bashrc`.
- **Electron terminals** must not be run under a fake `HOME`: the results store lives under `~/.local/share/silkterm-bench`, and redirecting `HOME` sends the results there too. Give `AppRun` an `APPDIR` or run the inner binary directly. They exit with SIGTRAP or SIGILL after measurement under Xvfb, which is harmless - the pids have already been sampled.

Only processes these scripts launched are ever signaled, and only by pid. A pattern kill would match the harness's own command line, and has taken out a live session before now.

Binaries are resolved from `PATH` first, then `cicd/artifacts/sizebench/terms/`. That directory is gitignored and excluded from the backup archive, and deliberately keeps the downloaded comparison artifacts so a re-measure needs no re-download. Alacritty is kept there as an extracted `.deb`:

```sh
apt-get download alacritty                                   # no sudo needed
dpkg -x alacritty_*.deb cicd/artifacts/sizebench/terms/..    # gives terms/usr/bin/alacritty
```

## Measuring on Windows

There is no compositor rig on Windows, so no terminal can be named on the command line and nothing resizes the window for you. Both halves are measured from inside the terminal under test, and **each has its own grid**, so it takes two passes with a resize between them. Set the window size by hand, then:

```sh
rem  speed: size the window to 160 x 42 first
python utility\update-showdown.py --speed-only --reps 6 --label "Windows Terminal"

rem  size and memory: now size it to 100 x 30
python utility\update-showdown.py --size-only --label "Windows Terminal"
```

The grid is checked rather than trusted: at the wrong size each half says so and does nothing, because memory scales with the surface and throughput scales with how much of the stream is drawn. `--any-size` overrides that and marks nothing, so only use it to explore. Naming no terminal at all runs whichever half the current window is already sized for.

`--label` is the table's row name, and both halves need it - one writes the speed cells, the other File+deps and Mem. Without it the numbers are printed and nothing is written, since putting a figure in the wrong row is worse than leaving the row empty.

The three Windows-only rows have to be measured this way or not at all. Two of them need care:

- **conhost** is not an ancestor of the shell - it is attached to it as a child - so the console window is asked which process owns it. That also picks the right thing under Windows Terminal, where a console host sits between the shell and the window process that is the real terminal.
- **MobaXterm** runs its local shell under Cygwin, so the ancestor walk finds it normally - but neither half can be run inside it as things stand. No Windows program gets a tty through that shell: `isatty` is false on both streams and the grid call fails, even though `stty` reports the size correctly. Its `python3` is the Windows one on `PATH`, so there is no Cygwin interpreter to fall back to, and both halves need a real terminal on stdin and stdout. Installing a Cygwin python into the plugin environment would be the way in.

A first pass ran on a laptop and published nothing; the figures and the reasons are in `ancillary-notes.fods`. Three things from it are worth knowing before the next attempt:

- **A terminal measured on non-comparable hardware cannot be rescued by calibrating it.** The three terminals that run on both platforms all landed within 1.5% of each other there, while spanning 77.4 to 86.9 MB/s on the reference rig - which says they were all pinned at the console pipe rather than by the machine, so the ratio measures the platform, not the hardware. There is no correction to derive from that.
- **No single factor serves the whole table anyway.** On one machine the console host read 13.6 MB/s of ASCII and Windows Terminal 93.1. They are limited by different things, so a multiplier fitted to one is wrong for the other.
- **Windows Terminal's 2-byte scene does not settle.** 12.67 to 31.08 MB/s inside a single run on an idle machine, and no better over four runs, against the 1 to 4% the published rows hold to.

**Copy `termbench-plain.shcl` somewhere temporary before pointing SilkTerm at it.** `--config` is a file the loader maintains, not just reads: it backfills every missing key and rewrites the layout, so one plain-row run turned the 13-line override list into 461 lines with the header comment moved inside a block. It still measures correctly, and the file no longer means what it says.

Two traps in driving a terminal from outside, both of which produce a plausible wrong answer rather than an error. Windows Terminal keeps every window in one long-lived process, quite possibly one someone is sitting in, and `wt.exe` joins an existing window unless given `-w new` - so a run can end up measuring inside somebody's session, and a name-based kill would take it out. And the size collector asks stdout, then stderr, then stdin for the grid, of which only a screen-buffer handle answers on Windows: capture both streams and it sees no terminal at all and refuses.

### What the Windows accounting does differently

The two collectors take the same measurement by different routes: the module list stands in for the mapped library set, mapped regions plus their resident pages for `smaps`, and PE imports for `DT_NEEDED`. The self-check (`sizebench-classify.py --selftest`) measures the running process both ways and compares, which stands in for the reproduce gate - there is no published Windows row to reproduce yet.

One difference is real and deliberate. "Beyond a base OS" means only the C runtime on Linux, because a desktop library there is something you installed; on Windows everything under System32 ships with the machine, so it is excluded by path. A Win32 terminal therefore shows a much smaller File+deps than a GTK one, and that is true rather than an artifact - but it means the two platforms' File+deps answer slightly different questions, on top of everything below.

A managed assembly contributes no closure edges, because the runtime resolves its references rather than the loader. That can only ever leave a driver-side library billed to the terminal, never the reverse, so it is the safe direction to be wrong in.

Windows figures are **not** directly comparable with the Linux rows, because the Windows host is a virtual machine on the same box with half the cores, less memory, virtualization overhead and a lower-specification passed-through GPU.

Do not correct that with a guessed multiplier. Calibrate it: SilkTerm, Alacritty, WezTerm and kitty all run on both platforms, so measuring those four on the VM gives a measured host-to-guest ratio, separately for parser-bound and GPU-sensitive terminals. A guessed factor cannot be validated against Windows-only terminals like conhost or MobaXterm, because those will only ever have been run on one rig.

## Files

| File | What it is |
| :--- | :--- |
| `../update-showdown.py` | the entry point: measures here or drives the rigs, writes the table |
| `termbench.py` | the throughput tool itself; runs standalone on any terminal, any OS |
| `bench-common.bash` | output helpers and pid-safe teardown, sourced by both rigs |
| `termbench-run.bash` | speed rig: compositor bring-up, terminal launch, grid fit, teardown |
| `termbench-scene.sh` | runs inside the terminal; reports its grid, then runs the benchmark |
| `termbench-plain.shcl` | SilkTerm with every optional effect off, for the "plain" rows |
| `sizebench-run.bash` | size rig: display bring-up, launch, grid sizing, process-tree collection |
| `sizebench-classify.py` | the closure classifier and the accounting, plus a collector for each platform |
| `showdown-readme.py` | writes the File+deps and Mem cells for one row |
| `ancillary-notes.fods` | measurements taken but not published, and why they could not be |

The entry point is Python and the rigs are shell, which is the right split: the rigs drive a Linux display and are Linux-only by nature, while the entry point has to run wherever a terminal does. It is deliberately one file rather than a shell copy plus a PowerShell copy. Two copies of one program drift, and a fix then lands in whichever copy was to hand rather than the one being run - which has happened here before, to `n8git_backup-and-publish`.
