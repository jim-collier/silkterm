# Terminal size and memory rig

Repeatable rig behind the last two columns of the README's "Terminal showdown" table - **File+deps** and **Mem**. Companion to `../termbench/`, which measures the speed columns.

Like that one, this is deliberately **not** wired into cicd: it launches real terminals on a private display and would churn the README on every build. Run it by hand when a row needs adding.

```sh
cicd/tests/sizebench/run.bash --list
cicd/tests/sizebench/run.bash --term alacritty
cicd/tests/sizebench/run.bash --term silkterm --verbose    # itemize both library sets
cicd/tests/sizebench/run.bash --help
```

## What the two numbers mean

**File+deps** is the executable plus every shared library the running process actually maps, minus the base OS runtime (libc, libm, libgcc, the loader) and minus the graphics stack. Self-contained bundles count their extracted payload plus the system libraries they still borrow - a different basis to a packaged terminal's, deliberately, because it is the honest answer to "what does installing this cost" for each packaging style.

**Mem** is the unique resident footprint of the whole process tree: private pages summed per process, plus each shared mapping counted once. It is neighbour-independent and free of intra-tree double counting, which the obvious alternatives are not - summed RSS charges a multi-process terminal several times for the same pages, and PSS divides shared pages by however many unrelated programs happen to be running.

## Three ways to get this wrong

**`ldd` understates any program that dlopens.** It lists three libraries for SilkTerm; the running process maps sixty-four. Always read a live process, never the binary on disk.

**Name-matching the driver misses what the driver drags in.** libstdc++ and libxml2 appear in that map only because mesa wants them; billing them to the terminal inflated SilkTerm's install size by a third. Classify by ldd *closure* from the graphics roots instead, and let the app closure win - a library the app needs in its own right is the app's however many other things also want it.

The name match still has to find the graphics *roots*, and the vendor back ends are called `libEGL_nvidia`, `libGLX_mesa`, `libdrm_intel`. A pattern anchored with a word boundary misses every one of them; they then look like libraries nothing in the graphics stack needs, and seeding the app closure with one drags libLLVM in behind it. That single character put 200 MiB of mesa on the terminal's bill.

**Window size moves the memory number a long way.** The same SilkTerm binary reads 159 MiB at its default geometry and 119 at the table's 100x30 grid, because the wallpaper and scrim buffers scale with the surface. Every row must be measured at the same grid or the column is meaningless.

## Reproduce before publishing

The rows in the table were measured across two sessions, so a new row is only comparable if the rig reproduces an old one. Both controls were re-run when this rig was written:

| Check | Published | This rig |
| :--- | ---: | ---: |
| xterm File+deps | 6.0 | 6.0 |
| xterm driver excluded | 0.0 | 0.0 |
| SilkTerm driver excluded | 108 | 108.3 |
| SilkTerm Mem | 121.4 | 119.4 |
| SilkTerm File+deps | 17.4 | 16.8 |

xterm is the useful control precisely because it draws on the CPU and should classify *nothing* as driver. SilkTerm is the other end - a third of a gigabyte of mapped graphics libraries that all has to land on the right side of the line.

The published figures are left as they were rather than refreshed to this pass. They sit inside the drift the table already warns about, and rewriting one row's numbers from a later session while the other nine keep their originals would make the column less consistent, not more. Expect a few MiB either way: libraries load on demand, and GTK terminals are the worst for it - gnome-terminal moved 28.5 to 53.6 between two runs of the earlier pass, because the first one measured the thin client wrapper rather than `gnome-terminal-server`.

## Notes

Only processes this script launched are ever signalled, and only by pid. A pattern kill would match the harness's own command line, and has taken out a live session before now.

Terminals are found on `PATH` first, then under `cicd/artifacts/sizebench/`, which keeps the downloaded comparison artifacts so a re-measure needs no re-download. That directory is gitignored and excluded from the backup archive.

Electron terminals (Hyper, Tabby) need their `AppRun` given `APPDIR`, or run the inner binary directly, and must not be run under a fake `HOME`. They exit with SIGTRAP or SIGILL after measurement under Xvfb, which is harmless - the pids have already been sampled.

| File | What it is |
| :--- | :--- |
| `run.bash` | display bring-up, terminal launch, grid sizing, process-tree collection |
| `classify.py` | the closure classifier and the smaps accounting |
