// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

//! Command-line parsing -> a window/tab/pane layout plan. See
//! project/design.md "Command-line options". Startup-only (not a hot path).
//!
//! Model: window-level options come first, then a hierarchy of tabs and panes
//! built with the create/select verbs (`--new-tab`/`--tab=`, `--new-pane`/`--pane=`).
//! Style options (shell, colors, font, ...) attach to the current scope and
//! cascade window -> tab -> pane (resolved at apply time).

use std::path::PathBuf;

use crate::config::{self, Fit};

// Direction a new pane goes relative to the pane it splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir4 {
	Down,
	Up,
	Left,
	Right,
}

// New-pane size within the split, in the split direction.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
	Cells(u32),
	Percent(f32),
}

// Cascading look/behaviour options; each level fills what it sets, the rest
// inherit. `bg_image: Some(None)` means "explicitly no image".
#[derive(Debug, Default, Clone)]
pub struct Style {
	pub shell: Option<Vec<String>>, // argv (already shell-word-split)
	pub keep_open: Option<bool>,
	pub font_name: Option<String>,
	pub font_size: Option<f32>,
	pub bg_color: Option<[u8; 3]>,
	pub fg_color: Option<[u8; 3]>,
	pub bg_image: Option<Option<String>>,
	pub bg_fit: Option<Fit>,
	pub bg_opacity: Option<f32>,
}

// Options that apply to the whole window (only valid before any tab/pane marker).
#[derive(Debug, Default, Clone)]
pub struct WindowOpts {
	pub columns: Option<usize>,
	pub rows: Option<usize>,
	pub pixel_width: Option<u32>,
	pub pixel_height: Option<u32>,
	pub opacity: Option<f32>,
	pub hide_frame: Option<bool>,
	pub hide_menu: Option<bool>,
	pub fullscreen: Option<bool>,
	pub title: Option<String>,
	pub style: Style,
}

#[derive(Debug, Clone)]
pub struct PaneSpec {
	pub id: Option<String>,     // handle; the first pane is "main"
	pub splits: Option<String>, // which pane to split (None -> previous/current)
	pub dir: Option<Dir4>,
	pub size: Option<Size>,
	pub title: Option<String>,
	pub style: Style,
	first: bool, // the implicit first pane; can't take splits/dir/size
}

impl PaneSpec {
	fn new(id: Option<String>, first: bool) -> Self {
		Self {
			id,
			splits: None,
			dir: None,
			size: None,
			title: None,
			style: Style::default(),
			first,
		}
	}
}

#[derive(Debug, Clone)]
pub struct TabSpec {
	pub id: Option<String>,
	pub title: Option<String>,
	pub style: Style,
	pub panes: Vec<PaneSpec>,
}

impl TabSpec {
	fn new(id: Option<String>) -> Self {
		// every tab starts with an implicit first pane (id "main")
		Self {
			id,
			title: None,
			style: Style::default(),
			panes: vec![PaneSpec::new(None, true)],
		}
	}
}

#[derive(Debug, Default)]
pub struct Cli {
	pub help: bool,
	pub version: bool,
	pub syntax: bool,
	pub config: Option<PathBuf>,
	pub win: WindowOpts,
	pub tabs: Vec<TabSpec>, // empty -> no hierarchical options given (use defaults)
	pub hierarchical: bool, // any tab/pane/structure flag was seen
}

// An id refers to the implicit first tab/pane.
fn is_first_id(id: &str) -> bool {
	matches!(id, "0" | "main")
}

fn parse_bool(s: &str) -> Option<bool> {
	match s.to_ascii_lowercase().as_str() {
		"true" | "t" | "yes" | "y" | "1" => Some(true),
		"false" | "f" | "no" | "n" | "0" => Some(false),
		_ => None,
	}
}

// Minimal POSIX-ish word split honouring single/double quotes and backslash, so
// `git log --oneline`, `bash --norc`, and `sh -c "a | b"` all argv-split right.
pub fn shell_split(s: &str) -> Result<Vec<String>, String> {
	let mut out = Vec::new();
	let mut cur = String::new();
	let mut chars = s.chars().peekable();
	let mut in_word = false;
	while let Some(c) = chars.next() {
		match c {
			' ' | '\t' => {
				if in_word {
					out.push(std::mem::take(&mut cur));
					in_word = false;
				}
			}
			'\'' => {
				in_word = true;
				for q in chars.by_ref() {
					if q == '\'' {
						break;
					}
					cur.push(q);
				}
			}
			'"' => {
				in_word = true;
				while let Some(q) = chars.next() {
					match q {
						'"' => break,
						'\\' => {
							if let Some(&n) = chars.peek() {
								if n == '"' || n == '\\' || n == '$' || n == '`' {
									cur.push(chars.next().unwrap());
									continue;
								}
							}
							cur.push('\\');
						}
						_ => cur.push(q),
					}
				}
			}
			'\\' => {
				in_word = true;
				if let Some(n) = chars.next() {
					cur.push(n);
				}
			}
			_ => {
				in_word = true;
				cur.push(c);
			}
		}
	}
	if in_word {
		out.push(cur);
	}
	if out.is_empty() {
		return Err("empty command".into());
	}
	Ok(out)
}

// Where a value flag's value comes from: `--opt=v`, `--opt v`, or `-o v`.
struct Args {
	items: Vec<String>,
	i: usize,
}
impl Args {
	fn next_token(&mut self) -> Option<String> {
		let t = self.items.get(self.i).cloned();
		if t.is_some() {
			self.i += 1;
		}
		t
	}
	// value for a flag whose `=value` (if any) is `inline`; else the next token.
	fn value(&mut self, flag: &str, inline: Option<String>) -> Result<String, String> {
		if let Some(v) = inline {
			return Ok(v);
		}
		self.next_token()
			.ok_or_else(|| format!("{flag} needs a value"))
	}
	// optional-bool flag: inline, else a following bool literal, else true.
	fn bool_value(&mut self, flag: &str, inline: Option<String>) -> Result<bool, String> {
		if let Some(v) = inline {
			return parse_bool(&v).ok_or_else(|| format!("{flag}: not a bool: {v}"));
		}
		if let Some(t) = self.items.get(self.i) {
			if let Some(b) = parse_bool(t) {
				self.i += 1;
				return Ok(b);
			}
		}
		Ok(true)
	}
}

fn parse_hex(flag: &str, v: &str) -> Result<[u8; 3], String> {
	config::parse_hex(v).ok_or_else(|| format!("{flag}: not a #rrggbb color: {v}"))
}

fn parse_f32(flag: &str, v: &str) -> Result<f32, String> {
	v.parse().map_err(|_| format!("{flag}: not a number: {v}"))
}

fn parse_size(v: &str) -> Result<Size, String> {
	if let Some(p) = v.strip_suffix('%') {
		Ok(Size::Percent(
			p.trim()
				.parse()
				.map_err(|_| format!("--size: bad percent: {v}"))?,
		))
	} else {
		Ok(Size::Cells(
			v.trim()
				.parse()
				.map_err(|_| format!("--size: bad cell count: {v}"))?,
		))
	}
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
	let mut a = Args {
		items: args.into_iter().collect(),
		i: 0,
	};
	let mut cli = Cli::default();
	// current scope: which tab / pane subsequent options attach to. None -> window.
	let mut cur_tab: Option<usize> = None;
	let mut cur_pane: usize = 0;

	while let Some(tok) = a.next_token() {
		if tok == "-h" {
			cli.help = true;
			continue;
		}
		let Some(body) = tok.strip_prefix("--") else {
			return Err(format!("unexpected argument: {tok}"));
		};
		let (name, inline) = match body.split_once('=') {
			Some((n, v)) => (n, Some(v.to_string())),
			None => (body, None),
		};

		// markers (enter/select a scope)
		match name {
			"new-tab" => {
				// optional handle comes only from `=value` (never eats the next flag)
				ensure_first_tab(&mut cli); // implicit first tab always exists
				let id = inline.filter(|s| !s.is_empty());
				cli.tabs.push(TabSpec::new(id));
				cur_tab = Some(cli.tabs.len() - 1);
				cur_pane = 0;
				cli.hierarchical = true;
				continue;
			}
			"tab" => {
				ensure_first_tab(&mut cli);
				let id = a.value("--tab", inline)?;
				let idx = find_tab(&cli, &id).ok_or_else(|| format!("--tab: no such tab: {id}"))?;
				cur_tab = Some(idx);
				cur_pane = 0;
				cli.hierarchical = true;
				continue;
			}
			"new-pane" => {
				ensure_first_tab(&mut cli);
				let t = cur_tab.unwrap_or(0);
				// optional handle comes only from `=value` (never eats the next flag)
				let id = inline.filter(|s| !s.is_empty());
				cli.tabs[t].panes.push(PaneSpec::new(id, false));
				cur_pane = cli.tabs[t].panes.len() - 1;
				cur_tab = Some(t);
				cli.hierarchical = true;
				continue;
			}
			"pane" => {
				ensure_first_tab(&mut cli);
				let t = cur_tab.unwrap_or(0);
				let id = a.value("--pane", inline)?;
				let p = find_pane(&cli.tabs[t], &id)
					.ok_or_else(|| format!("--pane: no such pane: {id}"))?;
				cur_pane = p;
				cur_tab = Some(t);
				cli.hierarchical = true;
				continue;
			}
			_ => {}
		}

		// window-level options (illegal once a tab/pane marker was seen)
		let window_only = matches!(
			name,
			"columns"
				| "rows" | "pixel-width"
				| "pixel-height"
				| "background-opacity"
				| "hide-windowframe"
				| "hide-menu"
				| "fullscreen"
				| "config" | "help"
				| "version" | "syntax"
		);
		if window_only {
			if cur_tab.is_some() {
				return Err(format!(
					"--{name} is a window option; put it before --new-tab/--tab/--new-pane/--pane"
				));
			}
			match name {
				"columns" => {
					cli.win.columns = Some(
						a.value(name, inline)?
							.parse()
							.map_err(|_| "bad --columns")?,
					)
				}
				"rows" => {
					cli.win.rows = Some(a.value(name, inline)?.parse().map_err(|_| "bad --rows")?)
				}
				"pixel-width" => {
					cli.win.pixel_width = Some(
						a.value(name, inline)?
							.parse()
							.map_err(|_| "bad --pixel-width")?,
					)
				}
				"pixel-height" => {
					cli.win.pixel_height = Some(
						a.value(name, inline)?
							.parse()
							.map_err(|_| "bad --pixel-height")?,
					)
				}
				"background-opacity" => {
					cli.win.opacity = Some(parse_f32(name, &a.value(name, inline)?)?)
				}
				"hide-windowframe" => cli.win.hide_frame = Some(a.bool_value(name, inline)?),
				"hide-menu" => cli.win.hide_menu = Some(a.bool_value(name, inline)?),
				"fullscreen" => cli.win.fullscreen = Some(a.bool_value(name, inline)?),
				"config" => cli.config = Some(PathBuf::from(a.value(name, inline)?)),
				"help" => cli.help = true,
				"version" => cli.version = true,
				"syntax" => cli.syntax = true,
				_ => unreachable!(),
			}
			continue;
		}

		// structural pane options
		if matches!(
			name,
			"splits" | "splits-pane" | "down" | "up" | "left" | "right" | "size"
		) {
			let t = cur_tab.ok_or_else(|| format!("--{name} only applies to a --new-pane"))?;
			let pane = &mut cli.tabs[t].panes[cur_pane];
			if pane.first {
				return Err(format!(
					"--{name} can't apply to the first pane (main); use --new-pane"
				));
			}
			match name {
				"splits" | "splits-pane" => pane.splits = Some(a.value(name, inline)?),
				"down" => set_dir(pane, Dir4::Down, a.bool_value(name, inline)?, name)?,
				"up" => set_dir(pane, Dir4::Up, a.bool_value(name, inline)?, name)?,
				"left" => set_dir(pane, Dir4::Left, a.bool_value(name, inline)?, name)?,
				"right" => set_dir(pane, Dir4::Right, a.bool_value(name, inline)?, name)?,
				"size" => pane.size = Some(parse_size(&a.value(name, inline)?)?),
				_ => unreachable!(),
			}
			continue;
		}

		// title (window / tab / pane by scope)
		if name == "title" {
			let v = a.value(name, inline)?;
			match cur_tab {
				None => cli.win.title = Some(v),
				Some(t) => {
					if cur_pane == 0 {
						cli.tabs[t].title = Some(v);
					} else {
						cli.tabs[t].panes[cur_pane].title = Some(v);
					}
				}
			}
			continue;
		}

		// cascading style options (route to the current scope)
		let style = match cur_tab {
			None => &mut cli.win.style,
			Some(t) => {
				if cur_pane == 0 {
					&mut cli.tabs[t].style
				} else {
					&mut cli.tabs[t].panes[cur_pane].style
				}
			}
		};
		match name {
			"shell" => style.shell = Some(shell_split(&a.value(name, inline)?)?),
			"keep-open" => style.keep_open = Some(a.bool_value(name, inline)?),
			"font-name" => style.font_name = Some(a.value(name, inline)?),
			"font-size" => style.font_size = Some(parse_f32(name, &a.value(name, inline)?)?),
			"background-color" => style.bg_color = Some(parse_hex(name, &a.value(name, inline)?)?),
			"foreground-color" => style.fg_color = Some(parse_hex(name, &a.value(name, inline)?)?),
			"background-image" => {
				// value present -> that path; no value -> explicitly none
				let v = a.value(name, inline).ok().filter(|s| !s.is_empty());
				style.bg_image = Some(v);
			}
			"background-image-stretch" => {
				if a.bool_value(name, inline)? {
					style.bg_fit = Some(Fit::Stretch);
				}
			}
			"background-image-zoom" => {
				if a.bool_value(name, inline)? {
					style.bg_fit = Some(Fit::Zoom);
				}
			}
			"background-image-opacity" => {
				style.bg_opacity = Some(parse_f32(name, &a.value(name, inline)?)?)
			}
			_ => return Err(format!("unknown option: --{name}")),
		}
	}

	Ok(cli)
}

fn set_dir(pane: &mut PaneSpec, dir: Dir4, on: bool, flag: &str) -> Result<(), String> {
	if !on {
		return Ok(()); // --right=false etc. is a no-op (leaves default/inherit)
	}
	if let Some(prev) = pane.dir {
		if prev != dir {
			return Err(format!(
				"--{flag} conflicts with an earlier direction on this pane"
			));
		}
	}
	pane.dir = Some(dir);
	Ok(())
}

// Fold window-level CLI style options into `s` (pure). Window-scoped only:
// per-pane visual style is deferred (it needs a per-pane renderer the single
// shared TextCtx doesn't have). `--shell` is handled separately (build_layout).
pub fn fold_window_style(s: &mut config::Settings, st: &Style) {
	if let Some(f) = &st.font_name {
		s.font_family = Some(f.clone());
	}
	if let Some(sz) = st.font_size {
		s.font_size = sz;
	}
	if let Some(c) = st.bg_color {
		s.bg = c;
	}
	if let Some(c) = st.fg_color {
		s.fg = c;
	}
	if let Some(img) = &st.bg_image {
		s.background_image = img.as_ref().map(PathBuf::from);
	}
	if let Some(fit) = st.bg_fit {
		s.background_fit = fit;
	}
	if let Some(o) = st.bg_opacity {
		s.background_opacity = o;
	}
}

impl WindowOpts {
	// Apply this window's CLI style to the live settings at startup (no-op if none
	// set). Call after the theme/OS palette settles so colours aren't clobbered.
	pub fn apply_style(&self) {
		let st = &self.style;
		let any = st.font_name.is_some()
			|| st.font_size.is_some()
			|| st.bg_color.is_some()
			|| st.fg_color.is_some()
			|| st.bg_image.is_some()
			|| st.bg_fit.is_some()
			|| st.bg_opacity.is_some();
		if !any {
			return;
		}
		let mut s = config::settings().as_ref().clone();
		fold_window_style(&mut s, st);
		config::update(s);
	}
}

fn ensure_first_tab(cli: &mut Cli) {
	if cli.tabs.is_empty() {
		cli.tabs.push(TabSpec::new(None));
	}
}

fn find_tab(cli: &Cli, id: &str) -> Option<usize> {
	if is_first_id(id) {
		return (!cli.tabs.is_empty()).then_some(0);
	}
	cli.tabs.iter().position(|t| t.id.as_deref() == Some(id))
}

fn find_pane(tab: &TabSpec, id: &str) -> Option<usize> {
	if is_first_id(id) {
		return Some(0);
	}
	tab.panes.iter().position(|p| p.id.as_deref() == Some(id))
}

// One-line-per-option usage text (shared by --help and --syntax).
pub fn usage() -> &'static str {
	"\
Usage: silkterm [WINDOW OPTIONS] [--new-tab|--tab=ID [TAB OPTIONS]] [--new-pane|--pane=ID [PANE OPTIONS]] ...

Window options (must precede any tab/pane):
  --columns N                 initial width in cells
  --rows N                    initial height in cells
  --pixel-width N             initial width in pixels (alternate)
  --pixel-height N            initial height in pixels (alternate)
  --background-opacity F      window see-through opacity 0..1
  --hide-windowframe[=BOOL]   start without WM decorations
  --hide-menu[=BOOL]          start with the menu bar hidden
  --fullscreen[=BOOL]         start fullscreen
  --config PATH               use an alternate config file
  --help, -h                  this help
  --syntax                    options list only
  --version                   program name + version + build

Layout:
  --new-tab[=HANDLE]          create a tab (becomes current)
  --tab=ID                    select an existing tab (0/main or a handle)
  --new-pane[=HANDLE]         create a pane by splitting the current/--splits pane
  --pane=ID                   select an existing pane (0/main or a handle)
  --splits=ID                 (with --new-pane) which pane to split
  --down|--up|--left|--right  where the new pane goes
  --size=N | --size=N%        new pane size in the split direction

Per-scope (window/tab/pane; cascades, most-specific wins):
  --title \"...\"               window/tab title (pane-level: not wired up yet)
  --shell \"...\"               command to run (argv; e.g. fish, 'bash --norc')
  --keep-open[=BOOL]          keep the pane after the command exits (not implemented yet)
  --font-name \"...\"           font family
  --font-size N               font size
  --background-color #rrggbb
  --foreground-color #rrggbb
  --background-image \"path\"   (no value = none)
  --background-image-stretch[=BOOL]
  --background-image-zoom[=BOOL]
  --background-image-opacity F
"
}

#[cfg(test)]
mod tests {
	use super::*;
	fn p(s: &str) -> Cli {
		parse(s.split_whitespace().map(String::from)).unwrap()
	}

	#[test]
	fn window_opts() {
		let c = p("--columns 100 --rows 40 --fullscreen --hide-menu=no");
		assert_eq!(c.win.columns, Some(100));
		assert_eq!(c.win.rows, Some(40));
		assert_eq!(c.win.fullscreen, Some(true));
		assert_eq!(c.win.hide_menu, Some(false));
		assert!(!c.hierarchical);
	}

	#[test]
	fn window_opt_after_tab_errors() {
		assert!(
			parse(
				"--new-tab --columns 80"
					.split_whitespace()
					.map(String::from)
			)
			.is_err()
		);
	}

	#[test]
	fn tabs_and_panes() {
		let c = p("--new-tab --new-pane --right --new-pane --down --splits=main");
		// implicit tab0 + one --new-tab = 2 tabs
		assert_eq!(c.tabs.len(), 2);
		let t = &c.tabs[1];
		assert_eq!(t.panes.len(), 3); // main + 2 new
		assert_eq!(t.panes[1].dir, Some(Dir4::Right));
		assert_eq!(t.panes[2].dir, Some(Dir4::Down));
		assert_eq!(t.panes[2].splits.as_deref(), Some("main"));
	}

	#[test]
	fn first_pane_rejects_split() {
		assert!(parse("--pane=main --right".split_whitespace().map(String::from)).is_err());
	}

	#[test]
	fn select_unknown_tab_errors() {
		assert!(parse("--tab=nope".split_whitespace().map(String::from)).is_err());
	}

	#[test]
	fn shell_splitting() {
		let c = parse(
			["--new-pane", "--shell=git log --oneline"]
				.into_iter()
				.map(String::from),
		)
		.unwrap();
		let sh = c.tabs[0].panes[1].style.shell.as_ref().unwrap();
		assert_eq!(sh, &["git", "log", "--oneline"]);
	}

	#[test]
	fn shell_quotes() {
		assert_eq!(
			shell_split(r#"bash -c "a | b""#).unwrap(),
			["bash", "-c", "a | b"]
		);
		assert_eq!(shell_split("'a b' c").unwrap(), ["a b", "c"]);
	}

	#[test]
	fn style_cascade_scope() {
		let c = p("--shell=fish --new-tab --shell=zsh --new-pane --shell=htop");
		assert_eq!(
			c.win.style.shell.as_deref(),
			Some(&["fish".to_string()][..])
		);
		assert_eq!(
			c.tabs[1].style.shell.as_deref(),
			Some(&["zsh".to_string()][..])
		);
		assert_eq!(
			c.tabs[1].panes[1].style.shell.as_deref(),
			Some(&["htop".to_string()][..])
		);
	}

	#[test]
	fn size_and_colors() {
		let c = p("--new-pane --size=30% --background-color=#102030");
		assert_eq!(c.tabs[0].panes[1].size, Some(Size::Percent(30.0)));
		assert_eq!(c.tabs[0].panes[1].style.bg_color, Some([0x10, 0x20, 0x30]));
	}

	#[test]
	fn window_style_folds_into_settings() {
		let c = p(
			"--font-name=Iosevka --font-size=20 --background-color=#102030 \
			--foreground-color=#abcdef --background-image=/x.png --background-image-zoom \
			--background-image-opacity=0.5",
		);
		let mut s = config::Settings::default();
		fold_window_style(&mut s, &c.win.style);
		assert_eq!(s.font_family.as_deref(), Some("Iosevka"));
		assert_eq!(s.font_size, 20.0);
		assert_eq!(s.bg, [0x10, 0x20, 0x30]);
		assert_eq!(s.fg, [0xab, 0xcd, 0xef]);
		assert_eq!(s.background_image, Some(PathBuf::from("/x.png")));
		assert_eq!(s.background_fit, config::Fit::Zoom);
		assert_eq!(s.background_opacity, 0.5);
	}

	#[test]
	fn window_style_noop_leaves_defaults() {
		// no style flags -> settings untouched
		let c = p("--columns 80");
		let mut s = config::Settings::default();
		let before = (s.font_size, s.bg, s.fg);
		fold_window_style(&mut s, &c.win.style);
		assert_eq!((s.font_size, s.bg, s.fg), before);
	}
}
