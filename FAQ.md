<!-- markdownlint-disable MD010 -- No hard tabs -->

# SilkTerm FAQ

## The binary is 10+ MiB. Isn't that huge for a terminal? `xfce4-terminal` is a few hundred KiB.

Short version: you're comparing a whole self-contained program against a thin launcher that leans on 10+ MiB of shared libraries already installed on your desktop. Count what actually has to be present for each one to run, and SilkTerm lands in the same ballpark - often smaller.

### Static vs dynamic linking

Most GTK terminals (`xfce4-terminal`, `gnome-terminal`, `terminator`, ...) are small *executables* but not small *programs*. The binary on disk is mostly glue; the real work lives in shared libraries that ship with your desktop and don't count toward the binary's size:

- **VTE** - the terminal widget (this *is* the emulator).
- **GTK** + **GLib/GObject** - the toolkit.
- **Pango** + **HarfBuzz** + **FreeType** + **fontconfig** - text shaping and glyph rendering.
- **Cairo** + **pixman** - drawing.

`terminator` goes a step further: it's Python, so its "binary" is basically a launcher script and the real cost is the Python interpreter plus everything above.

SilkTerm instead statically links nearly its entire stack into one file - and that stack is heavier by design, because it's a *GPU* terminal, not a GTK-widget terminal:

- **wgpu / naga** - the GPU abstraction and WGSL shader compiler. This is the single biggest chunk.
- **cosmic-text / swash / rustybuzz / ttf-parser** - SilkTerm's own text shaping and rasterization (its answer to Pango + HarfBuzz + FreeType).
- **alacritty_terminal** - the grid, PTY, and escape-sequence engine.
- **winit / glutin** - windowing and GL/X11 plumbing.

None of that is handed to us by the OS the way GTK and VTE are, so it all rides inside the one binary. In exchange you get a single file that runs with no toolkit to install and nothing to keep in version-sync.

### An "effective" comparison

The fair question isn't "how big is the file on disk" - it's "how much code has to be present for this thing to run at all." You can measure it yourself:

```bash
# The binary itself
ls -l "$(command -v xfce4-terminal)"

# Plus every shared library it drags in
ldd "$(command -v xfce4-terminal)" | awk '/=>/ {print $3}' | sort -u | xargs -r du -bc | tail -1
```

On a typical desktop the shared-library side of a GTK terminal adds up to well over 10 MiB (VTE + GTK + Pango + Cairo + GLib together), and that's *before* the Python interpreter for `terminator`. So the effective footprint of a GTK terminal is comparable to - frequently larger than - SilkTerm's, except SilkTerm carries all of it in one file with nothing to satisfy first.

For reference, SilkTerm's release binary is about **10.3 MiB** (Linux x86_64), already built with fat LTO, `panic = "abort"`, and symbol stripping. A default `cargo build --release` of the same code comes out noticeably larger; the size-tuned release profile is what keeps it there.

### Why go static at all?

- One file. No runtime, no "install these 40 packages first."
- Nothing to break when your distro bumps GTK or VTE out from under you.
- Not even tied to one display server: the same Linux binary runs native on both X11 and Wayland (chosen at runtime), so it keeps working across the slow industry migration between them.
- The same story ships on Windows - the build imports only core OS DLLs, no redistributable - where there's no system VTE/GTK to lean on in the first place.

That's the whole trade: the file carries everything, so it's bigger on disk than a glue binary. The program as a whole is not.
