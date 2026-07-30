# Terminal shootout rig

Repeatable rig behind the README's "Terminal showdown" table. It brings up a private headless Wayland compositor on the real GPU, launches one terminal as its only client, fits every terminal to the same grid, and runs `utility/termbench.py` inside it.

This is deliberately **not** wired into cicd. It takes minutes, needs a GPU, and republishing numbers on every build would churn the README. Run it by hand when a row needs adding or refreshing.

```sh
cicd/tests/termbench/run.bash --list                        # known terminal keys
cicd/tests/termbench/run.bash --term alacritty              # measure and publish one row
cicd/tests/termbench/run.bash --term silkterm --label "SilkTerm +candy"
cicd/tests/termbench/run.bash --term kitty --no-save        # measure without touching anything
cicd/tests/termbench/run.bash --help
```

## What the number means

Ingest and keep-up throughput - "why does it bog down when something dumps text" - **not** rasterization. At 160x42 only ~3400 cells are ever visible, so most of a 100 MB stream is parsed, stored and scrolled past without being drawn. The clock stops when the terminal answers a Primary DA query (`ESC[c`), which it can only answer once it has worked through everything queued.

A terminal that never answers cannot be timed this way. `termbench.py` records that as `synced: false` and refuses to publish the row, so an unbarriered figure can never reach the table - it would be timing a timeout. Hyper is the standing example.

## The rig is not neutral - this is the whole reason the harness exists

Measured 20260730, same terminal, same grid, three rigs:

| Rig | SilkTerm ascii | xterm ascii |
| :--- | ---: | ---: |
| Software GL (Xvfb + llvmpipe) | 45 MB/s | ~29 MB/s |
| VirtualGL on Xvfb | 76 MB/s | ~29 MB/s |
| Headless sway on the discrete GPU | 88 MB/s | 28.3 MB/s |

GPU-accelerated terminals swing by a factor of two; CPU-rendered ones do not move at all. So a table assembled from mixed rigs can rank the wrong terminal first - a VirtualGL run would have published a GTK terminal as faster than SilkTerm. **Every published row must come from one rig.** Do not assume a rig is neutral because one terminal reproduces on it.

Headless sway is used rather than the live desktop so the measurement neither disturbs nor is disturbed by whatever the machine is actually doing.

## Shortening a run

Use `--reps`. Fewer repetitions of the same payloads leaves the measured rate directly comparable and only widens the confidence interval - Tabby sits in the published table at 4 reps.

Never use `--scale`. Shrinking the payload changes what is measured: Hyper reads 32 MB/s at `--scale 0.05` but roughly 3 MB/s at full size, a tenfold difference. `--quick` is also unsuitable for publishing: `mode` is part of the comparability key, so quick runs are aggregated separately and never reach the table.

Watch the `CV%` column. On a machine that is also being used for other work, a run that got stepped on shows up there. Published rows have run 1-4%; re-run anything much worse.

## Per-terminal notes

Most terminals need nothing but their key. The awkward ones, and why:

- **gnome-terminal** never resizes with the compositor output, so it is the one terminal told its geometry directly (`--geometry`). Its first measured run silently came out at 180x45 and had to be redone - always confirm the fitted grid in the output.
- **xterm** is X11-only. Xwayland fails on this rig (`/tmp/.X11-unix` ownership), so xterm was measured on X11 instead, and its cross-rig agreement (28.8 vs 29.2, 1.3%) is what justifies publishing it beside Wayland rows.
- **WezTerm** 20240203 silently falls back to X11 under sway 1.10 despite `enable_wayland`. It is parser-bound and agreed within 1.7% across rigs, so its figure holds anywhere.
- **Hyper** rewrites `~/.hyper.js` on every launch, so it has to be written fresh per run. It never answers the barrier, so it cannot be timed at all.
- **Tabby** ignores `SHELL` and offers no profile hook that takes. Dismiss its Welcome tab once by clicking "Close and never show again", then hook the run through the login shell's `.bashrc`.
- **Electron terminals** must not be run under a fake `HOME`: the results store lives under `~/.local/share/silkterm-bench`, and redirecting `HOME` sends the results there too.

Binaries are resolved from `PATH` first, then `cicd/artifacts/sizebench/terms/`. That directory is gitignored and excluded from the backup archive, and deliberately keeps the downloaded comparison artifacts so a re-measure needs no re-download. Alacritty is kept there as an extracted `.deb`:

```sh
apt-get download alacritty                                   # no sudo needed
dpkg -x alacritty_*.deb cicd/artifacts/sizebench/terms/..    # gives terms/usr/bin/alacritty
```

## Measuring on Windows

There is no compositor rig on Windows. Run `utility/termbench.py` directly inside each terminal under test, sized to the same grid:

```sh
python utility\termbench.py --reps 6 --label "Windows Terminal"
```

Windows figures are **not** directly comparable with the Linux rows, because the Windows host is a virtual machine on the same box with half the cores, less memory, virtualization overhead and a lower-specification passed-through GPU.

Do not correct that with a guessed multiplier. Calibrate it: SilkTerm, Alacritty, WezTerm and kitty all run on both platforms, so measuring those four on the VM gives a measured host-to-guest ratio, separately for parser-bound and GPU-sensitive terminals. A guessed factor cannot be validated against Windows-only terminals like conhost or MobaXterm, because those will only ever have been run on one rig.

## Files

The table's last two columns - File+deps and Mem - come from a separate rig at `../sizebench/`, measured at a different grid and not refreshed by this one.

| File | What it is |
| :--- | :--- |
| `run.bash` | the rig: compositor bring-up, terminal launch, grid fit, teardown |
| `scene.sh` | runs inside the terminal; reports its grid, then runs the benchmark |
| `plain.toml` | SilkTerm with every optional effect off, for the "plain" row |
