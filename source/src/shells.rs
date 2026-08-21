// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

// Shell discovery, off the winit thread.
//
// The list the Tabs menu offers lives in the config (`shells.*`), so a title,
// an order or a disabled entry survives a launch and stays the user's. What
// this module adds is the part nobody wants to type: after the window is up and
// settled, look around for the shells that are actually installed and fold the
// new ones in.
//
// None of it may sit between launch and the first frame. A PATH scan stats every
// directory on the user's PATH - any of which can be a mount that answers slowly
// or never - and the Windows side reads the registry as well. So the window
// starts with whatever the config already holds, and a scan lands later as
// UserEvent::ShellsReady. Same shape as the wallpaper pipeline, deliberately:
// a thread per request, and the result is folded in on the winit thread.
//
// Two rules decide what a scan is allowed to do to a stored list, and they are
// deliberately lopsided (see `merge`): it may ADD a shell it found, and it may
// switch OFF one whose program has gone. It never switches one on and never
// rewrites a command line - those are the user's, and a scan has no way to tell
// a deliberate "no thanks" from a program that happened to be missing.

use std::path::{Path, PathBuf};

use winit::event_loop::EventLoopProxy;

use crate::config;
use crate::term::UserEvent;

// One shell the Tabs menu can offer, as stored under `shells.<slug>` in the
// config. `slug` is the config key and never changes once written; `title` is
// what the menu shows and the user may rename freely. `active` is the user's
// switch - an inactive entry stays in the file and stays out of the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellEntry {
	pub slug: String,
	pub title: String,
	pub command: String,
	pub active: bool,
	pub comment: String,
	// The last date a scan found this shell's program installed, YYYY-MM-DD.
	// Empty means no scan has ever seen it - a hand-written entry, or one that
	// was already switched off when the field was added.
	pub last_seen: String,
}

// Where a find sits in the order the list is offered in. A scan sorts by this
// and then by `seq`/title, so the order is stated once, in one place, instead of
// falling out of the sequence the detection happens to run in.
//
// It only ever decides an INITIAL population and where a newly-found shell is
// offered - `merge` keeps a stored list's own order whole, because that order is
// the user's (the Shell tab exists to set it). So changing anything here reaches
// a fresh config and nobody's existing one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
	// The user's own login shell, and directly under it the twin that skips its
	// startup files. Unix only: Windows has no user shell, and the order there
	// is stated outright rather than led by whatever ComSpec names.
	Login,
	LoginNoRc,
	// Windows leads with PowerShell 7 where it is installed.
	Pwsh7,
	// The modern cross-platform shells, alphabetically among themselves.
	Modern,
	// WSL distributions, each version alphabetically among its own. Both are
	// offered when both exist - a WSL1 distribution is installed and usable, and
	// hiding one because a newer-generation one exists beside it is not something
	// a scan gets to decide.
	Wsl2,
	Wsl1,
	// The POSIX environments' bashes (MSYS2, Git for Windows, Cygwin) - one
	// curated order, since they are three builds of the same shell.
	PosixEnv,
	PyCmd,
	// Language REPLs people do use as a shell, alphabetically.
	Language,
	// Everything else installed: Windows Cmd, and on unix the rest of the POSIX
	// family. Curated order (the table's own), which is a rough preference order
	// and more use than an alphabetical one here.
	Legacy,
	// The Windows 5.1 shell, and below it the variant that relaxes the execution
	// policy for its own session. Last because they are what you reach for when
	// something needs them, not what you open a terminal to get.
	WinPs5,
	WinPs5Relaxed,
}

// Whether the group's members sort alphabetically among themselves. The rest use
// `seq`, which is the position the table gave them.
impl Group {
	fn alphabetical(self) -> bool {
		matches!(
			self,
			Self::Modern | Self::Wsl2 | Self::Wsl1 | Self::Language
		)
	}
}

// One shell a scan turned up. It becomes a `ShellEntry` only if the stored list
// has nothing already running the same program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
	pub title: String,
	pub command: String,
	pub comment: String,
	// Almost everything found is offered switched ON - it is installed, so it is
	// presumably wanted. A variant that only exists as an alternative to another
	// entry arrives OFF instead, so the list gains a row rather than a surprise.
	pub active: bool,
	// Where this sits in the offered order, and its position inside its own group
	// where that group is curated rather than alphabetical.
	group: Group,
	seq: u32,
}

impl Found {
	pub(crate) fn new(title: &str, command: String, comment: &str) -> Self {
		Self {
			title: title.to_string(),
			command,
			comment: comment.to_string(),
			active: true,
			group: Group::Legacy,
			seq: 0,
		}
	}

	// The same, offered switched off: there for the user to turn on, never
	// something they have to notice and turn back off.
	fn dormant(title: &str, command: String, comment: &str) -> Self {
		Self {
			active: false,
			..Self::new(title, command, comment)
		}
	}

	fn in_group(self, group: Group, seq: u32) -> Self {
		Self { group, seq, ..self }
	}

	// What a find sorts on. An alphabetical group ignores `seq` outright, so a
	// table position cannot quietly override the order that group is meant to be
	// in; the title is folded to lowercase so "YSH" does not sort above "elvish".
	fn order(&self) -> (Group, u32, String) {
		let seq = if self.group.alphabetical() {
			0
		} else {
			self.seq
		};
		(self.group, seq, self.title.to_lowercase())
	}
}

// Run a scan on its own thread and post the merged list back to the event loop.
// A thread per scan rather than a long-lived worker: a PATH entry on a dead
// mount blocks its own thread forever, and there is nothing queued behind it.
pub fn spawn(proxy: &EventLoopProxy<UserEvent>) {
	let proxy = proxy.clone();
	let spawned = std::thread::Builder::new()
		.name("shells".into())
		.spawn(move || {
			let found = detect();
			// PowerShell cannot be asked where it is, so it is offered a way to
			// say - on this thread, since it means starting one (integration.rs)
			crate::integration::install(&found);
			let _ = proxy.send_event(UserEvent::ShellsReady(found));
		});
	if let Err(e) = spawned {
		eprintln!("{}: could not start shell scan: {e}", config::APP_NAME);
	}
}

// Fold a scan's findings into the stored list, in place and conservatively.
//
// The stored order is kept whole (it is the menu's order, and the future Shells
// tab lets the user set it); anything new lands at the end. `active` only ever
// falls: an entry whose program cannot be found is switched off rather than
// deleted, so a shell that is merely uninstalled keeps its title, its flags and
// its place. It is NOT switched back on if the program returns - a scan cannot
// tell that from a switch the user turned off on purpose.
pub fn merge(stored: &[ShellEntry], found: &[Found]) -> Vec<ShellEntry> {
	merge_with(stored, found, &which, &today())
}

fn merge_with(
	stored: &[ShellEntry],
	found: &[Found],
	resolve: &dyn Fn(&str) -> Option<PathBuf>,
	today: &str,
) -> Vec<ShellEntry> {
	let mut out: Vec<ShellEntry> = stored.to_vec();
	// One identity per stored entry, resolved once: where its program actually
	// is (None = not installed), its bare name, and its arguments.
	let ids: Vec<Option<Ident>> = out
		.iter()
		.map(|entry| Ident::of(&entry.command, resolve))
		.collect();
	for (entry, id) in out.iter_mut().zip(&ids) {
		if id.as_ref().is_none_or(|id| id.exe.is_none()) {
			entry.active = false;
		} else {
			// Stamped on the way past rather than only when something changed:
			// "last seen" is the one field a scan that found nothing new still
			// has news about.
			entry.last_seen = today.to_string();
		}
	}
	for hit in found {
		let Some(id) = Ident::of(&hit.command, resolve) else {
			continue;
		};
		if ids.iter().flatten().any(|stored| stored.same(&id)) {
			continue;
		}
		let slug = unique_slug(&hit.title, &out);
		out.push(ShellEntry {
			slug,
			title: hit.title.clone(),
			command: hit.command.clone(),
			active: hit.active,
			comment: hit.comment.clone(),
			last_seen: today.to_string(),
		});
	}
	out
}

// Today's date as YYYY-MM-DD, UTC. A "last seen" only ever has to be readable
// and comparable by eye, so a plain date is the whole of it - no clock, no zone,
// and nothing worth pulling a calendar crate in for.
pub fn today() -> String {
	let secs = std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |since| since.as_secs());
	let (year, month, day) = civil_from_days((secs / 86_400) as i64);
	format!("{year:04}-{month:02}-{day:02}")
}

// Days since 1970-01-01 -> (year, month, day). Hinnant's civil_from_days: the
// era arithmetic makes the 400-year leap cycle exact, so there is no table and
// no special case for February.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
	let shifted = days + 719_468; // re-base on 0000-03-01, so leap day lands last
	let era = shifted.div_euclid(146_097); // 400 years
	let day_of_era = shifted.rem_euclid(146_097);
	let year_of_era =
		(day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
	let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
	let month_index = (5 * day_of_year + 2) / 153; // 0 = March
	let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
	let month = if month_index < 10 {
		month_index + 3
	} else {
		month_index - 9
	} as u32;
	let year = year_of_era + era * 400;
	(if month <= 2 { year + 1 } else { year }, month, day)
}

// What makes two command lines "the same shell". `exe` is where the program
// resolved to right now (absolute, lowercased on Windows), `base` its bare name.
struct Ident {
	exe: Option<PathBuf>,
	base: String,
	args: Vec<String>,
}

// Flags that change how a shell LOOKS and nothing about what it is, so two
// commands differing only by one of these are the same shell. Everything else
// stays part of the identity - that is what keeps a `--norc` twin a separate
// entry from the shell it twins. Without this, adding `-NoLogo` to the table
// would land a second PowerShell beside every stored one on the next scan,
// which is the exact duplicate-entry mess this list already had once.
const COSMETIC_FLAGS: &[&str] = &["-nologo"];

impl Ident {
	fn of(command: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Self> {
		let argv = crate::cli::shell_split(command).ok()?;
		let (prog, args) = argv.split_first()?;
		Some(Self {
			exe: resolve(prog).map(|p| norm(&p)),
			base: base_name(prog),
			args: args
				.iter()
				.filter(|arg| !COSMETIC_FLAGS.contains(&arg.to_ascii_lowercase().as_str()))
				.cloned()
				.collect(),
		})
	}
	// Same arguments, and either both programs resolve to the same file or the
	// stored one resolves nowhere and merely shares a name. That second arm is
	// what keeps a reinstall from adding a duplicate beside the disabled entry
	// it belongs to; two shells that are BOTH installed stay distinct on their
	// paths, so Git Bash, MSYS2 bash and Cygwin bash never collapse together.
	fn same(&self, other: &Self) -> bool {
		if self.args != other.args {
			return false;
		}
		match (&self.exe, &other.exe) {
			(Some(a), Some(b)) => a == b,
			(None, _) => self.base == other.base,
			_ => false,
		}
	}
}

// Do two command lines name the same shell? Same rule the scan folds on, so a
// caller outside this module gets the same answer: a bare name is looked up
// before comparing, and a stored entry whose program has gone falls back to its
// bare name. Asked in both directions because that fallback is one-sided.
//
// `adopt_default_shell` is what needs it. The retired `shell.default` was
// routinely a bare name (`pwsh`) where the scanned list already carried the full
// path to the same file, and comparing those as STRINGS put a second copy of one
// shell at the top of the list - which is then the default shell, twice over.
pub fn same_command(a: &str, b: &str) -> bool {
	same_command_with(a, b, &which)
}

fn same_command_with(a: &str, b: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> bool {
	match (Ident::of(a, resolve), Ident::of(b, resolve)) {
		(Some(a), Some(b)) => a.same(&b) || b.same(&a),
		_ => false,
	}
}

// An argv back as one command line, quoted the way this module's own tables
// write one - so `friendly` can be asked about a pane launched from the CLI,
// where the shell arrives already split.
pub fn command_line(argv: &[String]) -> String {
	argv.iter()
		.map(|arg| {
			if arg.contains(' ') {
				format!("\"{arg}\"")
			} else {
				arg.clone()
			}
		})
		.collect::<Vec<_>>()
		.join(" ")
}

// What a tab calls the shell it is running. The stored list is the authority -
// the user renamed those titles, so "Windows PowerShell 5 (relaxed)" is what
// their tab should say - and only where nothing matches does the program's own
// name stand in. Matching goes through `same_command`, not string equality, for
// the reason that rule exists at all: the pane may have been launched with a
// bare `pwsh` where the list carries the full path to the same file.
pub fn friendly(command: &str, stored: &[ShellEntry]) -> String {
	if let Some(entry) = stored
		.iter()
		.find(|entry| same_command(&entry.command, command))
	{
		if !entry.title.trim().is_empty() {
			return entry.title.clone();
		}
	}
	crate::cli::shell_split(command)
		.ok()
		.and_then(|argv| argv.first().map(|prog| pretty(&base_name(prog))))
		.filter(|title| !title.is_empty())
		.unwrap_or_else(|| "Shell".to_string())
}

// A list entry for a command line the list does not carry yet: the Settings
// dialog's "Add", and the one-time adoption of the old `shell.default`. The
// title is the program's own name, tidied - the user renames it if they want
// something else - and the key is made unique against what is already stored.
// `last_seen` stays empty: no scan has vouched for this one.
pub fn adopted(command: &str, existing: &[ShellEntry]) -> ShellEntry {
	let title = crate::cli::shell_split(command)
		.ok()
		.and_then(|argv| argv.first().map(|prog| pretty(&base_name(prog))))
		.filter(|title| !title.is_empty())
		.unwrap_or_else(|| "New shell".to_string());
	ShellEntry {
		slug: unique_slug(&title, existing),
		title,
		command: command.trim().to_string(),
		active: true,
		comment: String::new(),
		last_seen: String::new(),
	}
}

// Config key for a new entry: the title, folded to something a config file can
// hold, made unique against what is already in the list. It never changes after
// this - a retitle rewrites one line, the way a theme rename does.
fn unique_slug(title: &str, existing: &[ShellEntry]) -> String {
	let mut base: String = title
		.chars()
		.map(|c| {
			if c.is_ascii_alphanumeric() {
				c.to_ascii_lowercase()
			} else {
				'_'
			}
		})
		.collect();
	while base.contains("__") {
		base = base.replace("__", "_");
	}
	let base = base.trim_matches('_').to_string();
	let base = if base.is_empty() { "shell" } else { &base }.to_string();
	let taken = |s: &str| existing.iter().any(|e| e.slug == s);
	if !taken(&base) {
		return base;
	}
	(2..=u32::from(u16::MAX))
		.map(|n| format!("{base}_{n}"))
		.find(|slug| !taken(slug))
		.unwrap_or(base)
}

// Case-fold a resolved path on Windows, where two spellings name one file.
fn norm(path: &Path) -> PathBuf {
	if cfg!(windows) {
		PathBuf::from(path.to_string_lossy().to_lowercase())
	} else {
		path.to_path_buf()
	}
}

fn base_name(prog: &str) -> String {
	let name = Path::new(prog)
		.file_name()
		.map_or_else(|| prog.to_string(), |n| n.to_string_lossy().into_owned());
	let name = if cfg!(windows) {
		name.to_lowercase()
	} else {
		name
	};
	name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

// Where `prog` would run from, or None if it is not installed. A name with a
// separator in it is taken literally (that is the user saying where); a bare
// name is looked up on PATH, honouring PATHEXT on Windows.
fn which(prog: &str) -> Option<PathBuf> {
	if prog.contains('/') || (cfg!(windows) && prog.contains('\\')) {
		let path = Path::new(prog);
		return path.is_file().then(|| path.to_path_buf());
	}
	let exts: Vec<String> = if cfg!(windows) {
		std::env::var("PATHEXT")
			.unwrap_or_else(|_| ".EXE;.COM;.BAT;.CMD".into())
			.split(';')
			.filter(|e| !e.is_empty())
			.map(str::to_lowercase)
			.collect()
	} else {
		Vec::new()
	};
	let path = std::env::var_os("PATH")?;
	for dir in std::env::split_paths(&path) {
		let direct = dir.join(prog);
		if direct.is_file() {
			return Some(direct);
		}
		for ext in &exts {
			let with_ext = dir.join(format!("{prog}{ext}"));
			if with_ext.is_file() {
				return Some(with_ext);
			}
		}
	}
	None
}

// Wrap a path for a command line only when it has to be. Inside double quotes a
// backslash before an ordinary character stays put (see cli::shell_split), so a
// Windows path survives this unharmed.
fn quoted(path: &Path) -> String {
	let text = path.to_string_lossy();
	if text.contains(' ') {
		format!("\"{text}\"")
	} else {
		text.into_owned()
	}
}

// The flag that starts a shell without reading its startup files, and the words
// to put in its title. Only the default shell gets this twin - it is the one a
// user reaches for when their own rc file is what they are debugging.
fn no_startup_file(base: &str) -> Option<(&'static str, &'static str)> {
	Some(match base {
		"bash" => ("--norc", "no rc"),
		"zsh" => ("--no-rcs", "no rc"),
		"fish" => ("--no-config", "no config"),
		"csh" | "tcsh" => ("-f", "no rc"),
		"nu" => ("--no-config-file", "no config"),
		"xonsh" => ("--no-rc", "no rc"),
		"pwsh" | "powershell" => ("-NoProfile", "no profile"),
		_ => return None,
	})
}

// A friendly name for a program we found by its bare name.
fn pretty(base: &str) -> String {
	for (exe, title, _, _) in KNOWN {
		if *exe == base {
			return (*title).to_string();
		}
	}
	let mut chars = base.chars();
	chars.next().map_or_else(
		|| base.to_string(),
		|first| first.to_uppercase().collect::<String>() + chars.as_str(),
	)
}

// Where a program found by its bare name belongs in the order, and its position
// inside a curated group. Anything the table does not name is an ordinary system
// shell - that is what /etc/shells turns up.
fn known_group(base: &str) -> (Group, u32) {
	for (seq, (exe, _, _, group)) in KNOWN.iter().enumerate() {
		if *exe == base {
			return (*group, seq as u32);
		}
	}
	(Group::Legacy, KNOWN.len() as u32)
}

// The shells worth looking for by name. The fourth field is where each belongs
// in the offered order (see `Group`); within a curated group the table's own
// position decides, so this list is also the preference order for the POSIX
// family. A name not on this list is still found when it is the user's login
// shell, or when /etc/shells names it.
#[cfg(unix)]
const KNOWN: &[(&str, &str, &str, Group)] = &[
	("bash", "Bash", "", Group::Legacy),
	("zsh", "Zsh", "", Group::Legacy),
	("fish", "Fish", "", Group::Legacy),
	("dash", "Dash", "", Group::Legacy),
	("ash", "Ash", "", Group::Legacy),
	("ksh", "Korn shell", "", Group::Legacy),
	("mksh", "MirBSD Korn shell", "", Group::Legacy),
	("yash", "Yash", "", Group::Legacy),
	("tcsh", "Tcsh", "", Group::Legacy),
	("csh", "C shell", "", Group::Legacy),
	("sh", "POSIX shell", "", Group::Legacy),
	("es", "Es", "", Group::Legacy),
	("rc", "rc", "the Plan 9 shell", Group::Legacy),
	(
		"nu",
		"Nushell",
		"structured data through the pipeline",
		Group::Modern,
	),
	("elvish", "Elvish", "", Group::Modern),
	(
		"xonsh",
		"Xonsh",
		"Python syntax with shell primitives",
		Group::Modern,
	),
	("ysh", "YSH", "the Oils shell", Group::Modern),
	(
		"osh",
		"OSH",
		"the Oils shell, bash-compatible",
		Group::Modern,
	),
	("murex", "Murex", "", Group::Modern),
	("ion", "Ion", "", Group::Modern),
	("pwsh", "PowerShell 7", "", Group::Modern),
	("python3", "Python 3", "", Group::Language),
	("ipython", "IPython", "", Group::Language),
	("node", "Node.js", "", Group::Language),
];

#[cfg(not(unix))]
const KNOWN: &[(&str, &str, &str, Group)] = &[
	("pwsh", "PowerShell 7", "", Group::Pwsh7),
	(
		"nu",
		"Nushell",
		"structured data through the pipeline",
		Group::Modern,
	),
	("elvish", "Elvish", "", Group::Modern),
	(
		"xonsh",
		"Xonsh",
		"Python syntax with shell primitives",
		Group::Modern,
	),
	("ysh", "YSH", "the Oils shell", Group::Modern),
	(
		"osh",
		"OSH",
		"the Oils shell, bash-compatible",
		Group::Modern,
	),
	("murex", "Murex", "", Group::Modern),
	(
		"pycmd",
		"PyCmd",
		"cmd.exe with completion and history",
		Group::PyCmd,
	),
	("python", "Python 3", "", Group::Language),
	("node", "Node.js", "", Group::Language),
	("cmd", "Windows Cmd", "", Group::Legacy),
	(
		"powershell",
		"Windows PowerShell 5",
		"the 5.1 shell that ships with Windows",
		Group::WinPs5,
	),
];

// A shell offered with the flags it reads better with. Only presentation: the
// PowerShells print a copyright banner (and 7 an occasional update notice)
// before their first prompt, which is noise in a terminal that opens a new tab
// per thought. Nothing here may change how a shell BEHAVES - every such flag
// belongs in COSMETIC_FLAGS too, or the next scan lands a duplicate.
fn launch(command: &str, program: &str) -> String {
	let base = base_name(program);
	let base = base.strip_suffix(".exe").unwrap_or(&base);
	match base {
		"pwsh" | "powershell" => format!("{command} -NoLogo"),
		_ => command.to_string(),
	}
}

// Everything installed that looks like a shell, in the order it should be
// offered in (see `Group`).
pub fn detect() -> Vec<Found> {
	let mut out: Vec<Found> = Vec::new();
	let mut seen: Vec<Ident> = Vec::new();
	let add = |hit: Found, out: &mut Vec<Found>, seen: &mut Vec<Ident>| {
		let Some(id) = Ident::of(&hit.command, &which) else {
			return;
		};
		if id.exe.is_none() || seen.iter().any(|s| s.same(&id)) {
			return;
		}
		seen.push(id);
		out.push(hit);
	};

	// On unix the user's own shell leads - and that is load-bearing rather than
	// merely tidy, because an initial population becomes the list verbatim and the
	// top of the list IS the default shell (config::default_shell_argv). So the
	// terminal opens on the shell the user logs in with, without their having to
	// say so. It is also the one, and the only one, that gets the twin that skips
	// its startup files. Windows has no user shell, so ComSpec takes its ordinary
	// place in the order instead of the top of it.
	if let Some(login) = login_shell() {
		let base = base_name(&login);
		let title = pretty(&base);
		let login_cmd = launch(&login, &base);
		let (at, twin_at) = login_groups(&base);
		add(
			Found::new(&title, login_cmd.clone(), "").in_group(at.0, at.1),
			&mut out,
			&mut seen,
		);
		if let Some((flag, note)) = no_startup_file(&base) {
			// Switched OFF: it is what you reach for when your own rc file is the
			// thing you are debugging, not what you want a second copy of in the
			// menu every day.
			add(
				Found::dormant(
					&format!("{title} ({note})"),
					format!("{login_cmd} {flag}"),
					"starts without reading the shell's startup files",
				)
				.in_group(twin_at.0, twin_at.1),
				&mut out,
				&mut seen,
			);
		}
	}
	for (seq, (exe, title, comment, group)) in KNOWN.iter().enumerate() {
		if let Some(path) = which(exe) {
			add(
				Found::new(title, launch(&quoted(&path), exe), comment)
					.in_group(*group, seq as u32),
				&mut out,
				&mut seen,
			);
		}
	}
	for hit in platform_extras() {
		add(hit, &mut out, &mut seen);
	}
	// One sort, at the end: the order is a property of the list, not of the
	// sequence the looking happened to run in.
	out.sort_by_key(Found::order);
	out
}

// Where the login shell and its startup-file-free twin belong. Both arms compile
// on both platforms - a `cfg` here would make Login/LoginNoRc look unconstructed
// on Windows, and it is worth being able to read the whole rule from either box.
fn login_groups(base: &str) -> ((Group, u32), (Group, u32)) {
	if cfg!(unix) {
		((Group::Login, 0), (Group::LoginNoRc, 0))
	} else {
		let at = known_group(base);
		(at, at)
	}
}

// The shell the user logs in with. $SHELL is what a person means by "my shell"
// and every desktop session sets it. Windows has no user shell at all, so the
// nearest discoverable thing is what it calls the command processor (ComSpec,
// i.e. cmd.exe); if even that is unset, the table below leads instead.
#[cfg(unix)]
fn login_shell() -> Option<String> {
	let shell = std::env::var("SHELL").ok()?;
	(!shell.trim().is_empty()).then_some(shell)
}

#[cfg(not(unix))]
fn login_shell() -> Option<String> {
	// ComSpec names the command processor, and its startup-file twin is worth
	// having for the same reason a login shell's is.
	let comspec = std::env::var("ComSpec").ok()?;
	(!comspec.trim().is_empty()).then_some(quoted(Path::new(&comspec)))
}

// Shells that live at a known place rather than on PATH, plus anything that
// needs asking the system rather than the filesystem.
#[cfg(unix)]
fn platform_extras() -> Vec<Found> {
	// /etc/shells is the system's own list of login shells, so it turns up
	// anything installed outside PATH (and anything too obscure for the table).
	let Ok(text) = std::fs::read_to_string("/etc/shells") else {
		return Vec::new();
	};
	text.lines()
		.map(str::trim)
		.filter(|line| line.starts_with('/'))
		.map(|line| {
			let base = base_name(line);
			let (group, seq) = known_group(&base);
			Found::new(&pretty(&base), line.to_string(), "").in_group(group, seq)
		})
		.collect()
}

#[cfg(windows)]
fn platform_extras() -> Vec<Found> {
	let mut out = Vec::new();
	// Windows PowerShell 5.1 and cmd.exe are always at a fixed place under the
	// system root, whether or not the user has them on PATH.
	if let Ok(root) = std::env::var("SystemRoot") {
		let root = Path::new(&root);
		for (rel, title, comment, group) in [
			(
				r"System32\WindowsPowerShell\v1.0\powershell.exe",
				"Windows PowerShell 5",
				"the 5.1 shell that ships with Windows",
				Group::WinPs5,
			),
			(r"System32\cmd.exe", "Windows Cmd", "", Group::Legacy),
		] {
			let path = root.join(rel);
			if path.is_file() {
				out.push(
					Found::new(title, launch(&quoted(&path), rel), comment).in_group(group, 0),
				);
			}
		}
		// Windows PowerShell 5.1 ships at a policy that refuses to run script
		// files, so it loads no profile - which is why it cannot report where it
		// is (see integration.rs). This entry relaxes that for its own session
		// only, nothing written anywhere. It arrives switched OFF because it is
		// a security setting: an alternative to reach for, not a default.
		let ps51 = root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
		if ps51.is_file() {
			out.push(
				Found::dormant(
					"Windows PowerShell 5 (relaxed)",
					format!(
						"{} -ExecutionPolicy RemoteSigned",
						launch(&quoted(&ps51), "powershell")
					),
					"runs profile scripts; per-session, nothing is written",
				)
				.in_group(Group::WinPs5Relaxed, 0),
			);
		}
	}
	// The POSIX environments each ship their own bash. They share a name and
	// nothing else, so each is offered under the environment it belongs to - and
	// they are named for it, since "Bash" alone would be three identical rows.
	//
	// Git for Windows is the exception that has to be handled: it installs the
	// same shell under two names (bin\bash.exe wraps usr\bin\bash.exe) and a
	// 64-bit box reports one Program Files directory under more than one
	// variable, so both spellings are real files and nothing downstream can tell
	// they are one shell. Take the first hit and stop.
	let git_bash = program_files()
		.iter()
		.flat_map(|base| [r"Git\bin\bash.exe", r"Git\usr\bin\bash.exe"].map(|rel| base.join(rel)))
		.find(|path| path.is_file());
	if let Some(path) = git_bash {
		out.push(
			Found::new(
				"Bash (Git's mini)",
				quoted(&path),
				"MSYS2-based, from Git for Windows",
			)
			.in_group(Group::PosixEnv, 1),
		);
	}
	// PyCmd ships as a zip that is extracted wherever the user likes, so it is
	// normally nowhere near PATH - the table above finds it only for someone who
	// put it there deliberately. Program Files is where it usually lands.
	let pycmd = program_files()
		.iter()
		.map(|base| base.join(r"PyCmd\PyCmd.exe"))
		.find(|path| path.is_file());
	if let Some(path) = pycmd {
		out.push(
			Found::new(
				"PyCmd",
				quoted(&path),
				"cmd.exe with completion and history",
			)
			.in_group(Group::PyCmd, 0),
		);
	}
	for (path, title, comment, seq) in [
		(r"C:\msys64\usr\bin\bash.exe", "Bash (MSYS2's full)", "", 0),
		(r"C:\msys32\usr\bin\bash.exe", "Bash (MSYS2's full)", "", 0),
		(r"C:\cygwin64\bin\bash.exe", "Bash (Cygwin)", "", 2),
		(r"C:\cygwin\bin\bash.exe", "Bash (Cygwin)", "", 2),
	] {
		let path = Path::new(path);
		if path.is_file() {
			out.push(Found::new(title, quoted(path), comment).in_group(Group::PosixEnv, seq));
		}
	}
	// WSL distributions, read from the registry rather than by asking wsl.exe:
	// a WSL2 distribution lives in a virtual disk, and listing them must not be
	// the thing that boots the virtual machine. What is offered is the whole
	// distribution, with no shell named - its own default runs, and the user can
	// add flags to the entry if they want a particular one. The generation is
	// part of the title because it is the whole difference between two rows that
	// would otherwise read identically, and it decides which sorts first.
	for (name, wsl2) in wsl_distributions() {
		let title = format!("WSL{}; {name}", if wsl2 { 2 } else { 1 });
		let command = format!("wsl.exe -d {}", quoted(Path::new(&name)));
		out.push(
			Found::new(&title, command, "the distribution's own shell")
				.in_group(if wsl2 { Group::Wsl2 } else { Group::Wsl1 }, 0),
		);
	}
	out
}

#[cfg(windows)]
fn program_files() -> Vec<PathBuf> {
	["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
		.iter()
		.filter_map(|var| std::env::var(var).ok())
		.map(PathBuf::from)
		.collect()
}

// Installed WSL distributions, by name, with whether each runs on WSL2. Nothing
// is launched: the keys are written when a distribution is registered.
//
// The generation is bit 3 of the distribution's `Flags` value. Note the `Version`
// value beside it is NOT it - that is the registration format's version, and it
// reads 2 for a WSL1 distribution as happily as for a WSL2 one (measured). A
// distribution whose flags cannot be read is reported as WSL1, which is the
// older behaviour and puts it lower in the list rather than claiming something
// about it that was never established.
#[cfg(windows)]
fn wsl_distributions() -> Vec<(String, bool)> {
	use windows_sys::Win32::Foundation::ERROR_SUCCESS;
	use windows_sys::Win32::System::Registry::{
		HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD, REG_SZ, RegCloseKey, RegEnumKeyExW,
		RegOpenKeyExW, RegQueryValueExW,
	};

	// Bit 3 of a distribution's Flags: set = WSL2.
	const FLAG_WSL2: u32 = 0x8;

	fn wide(s: &str) -> Vec<u16> {
		s.encode_utf16().chain(std::iter::once(0)).collect()
	}

	let mut out = Vec::new();
	let root = wide(r"Software\Microsoft\Windows\CurrentVersion\Lxss");
	let mut lxss: HKEY = std::ptr::null_mut();
	// SAFETY: a read-only open of a fixed key path; the handle is closed below.
	let opened =
		unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, root.as_ptr(), 0, KEY_READ, &raw mut lxss) };
	if opened != ERROR_SUCCESS {
		return out;
	}
	let value = wide("DistributionName");
	let flags_value = wide("Flags");
	for index in 0.. {
		// Key names are bounded at 255 characters by the registry itself.
		let mut name = [0u16; 256];
		let mut len = name.len() as u32;
		// SAFETY: `len` is the buffer's length in characters, as the call wants.
		let more = unsafe {
			RegEnumKeyExW(
				lxss,
				index,
				name.as_mut_ptr(),
				&raw mut len,
				std::ptr::null(),
				std::ptr::null_mut(),
				std::ptr::null_mut(),
				std::ptr::null_mut(),
			)
		};
		if more != ERROR_SUCCESS {
			break;
		}
		let sub = wide(&String::from_utf16_lossy(&name[..len as usize]));
		let mut distro: HKEY = std::ptr::null_mut();
		// SAFETY: read-only open of a subkey just enumerated; closed below.
		let opened = unsafe { RegOpenKeyExW(lxss, sub.as_ptr(), 0, KEY_READ, &raw mut distro) };
		if opened != ERROR_SUCCESS {
			continue;
		}
		let mut kind = 0u32;
		let mut buf = [0u16; 256];
		let mut bytes = std::mem::size_of_val(&buf) as u32;
		// SAFETY: `bytes` is the buffer's size in BYTES, which is what the
		// registry wants here - unlike RegEnumKeyExW's count of characters.
		let read = unsafe {
			RegQueryValueExW(
				distro,
				value.as_ptr(),
				std::ptr::null(),
				&raw mut kind,
				buf.as_mut_ptr().cast::<u8>(),
				&raw mut bytes,
			)
		};
		let mut flags = 0u32;
		let mut flags_kind = 0u32;
		let mut flags_bytes = std::mem::size_of::<u32>() as u32;
		// SAFETY: a DWORD-sized read into a DWORD, with the size in bytes.
		let flags_read = unsafe {
			RegQueryValueExW(
				distro,
				flags_value.as_ptr(),
				std::ptr::null(),
				&raw mut flags_kind,
				(&raw mut flags).cast::<u8>(),
				&raw mut flags_bytes,
			)
		};
		// SAFETY: the handle came from a successful open above.
		unsafe { RegCloseKey(distro) };
		if read != ERROR_SUCCESS || kind != REG_SZ {
			continue;
		}
		let wsl2 = flags_read == ERROR_SUCCESS
			&& flags_kind == REG_DWORD
			&& (flags & FLAG_WSL2) == FLAG_WSL2;
		let chars = (bytes as usize / 2).min(buf.len());
		let name = String::from_utf16_lossy(&buf[..chars]);
		let name = name.trim_end_matches('\0').trim().to_string();
		if !name.is_empty() {
			out.push((name, wsl2));
		}
	}
	// SAFETY: the handle came from the successful open at the top.
	unsafe { RegCloseKey(lxss) };
	out
}

#[cfg(not(any(unix, windows)))]
fn platform_extras() -> Vec<Found> {
	Vec::new()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn hit(title: &str, group: Group, seq: u32) -> Found {
		Found::new(title, format!("/x/{title}"), "").in_group(group, seq)
	}

	fn ordered(mut found: Vec<Found>) -> Vec<String> {
		found.sort_by_key(Found::order);
		found.into_iter().map(|hit| hit.title).collect()
	}

	// The whole offered order, in one assertion: this is what the Tabs menu looks
	// like on a fresh install, so a group that moved - or one that quietly stopped
	// sorting alphabetically - is a visible change and not an implementation
	// detail. Elvish carries a LATER table position than YSH on purpose: inside an
	// alphabetical group the title decides and the table position must not leak in.
	#[test]
	fn the_offered_order_groups_first_and_sorts_inside_a_group() {
		let titles = ordered(vec![
			hit("Windows Cmd", Group::Legacy, 9),
			hit("YSH", Group::Modern, 3),
			hit("PowerShell 7", Group::Pwsh7, 0),
			hit("WSL1; Debian", Group::Wsl1, 0),
			hit("Nushell", Group::Modern, 0),
			hit("Python 3", Group::Language, 1),
			hit("WSL2; Ubuntu", Group::Wsl2, 0),
			hit("Windows PowerShell 5 (relaxed)", Group::WinPs5Relaxed, 0),
			hit("Bash (Git's mini)", Group::PosixEnv, 1),
			hit("Node.js", Group::Language, 0),
			hit("Windows PowerShell 5", Group::WinPs5, 0),
			hit("PyCmd", Group::PyCmd, 0),
			hit("WSL2; Fedora", Group::Wsl2, 0),
			hit("Bash (MSYS2's full)", Group::PosixEnv, 0),
			hit("Elvish", Group::Modern, 9),
		]);
		assert_eq!(
			titles,
			[
				"PowerShell 7",
				"Elvish",
				"Nushell",
				"YSH",
				"WSL2; Fedora",
				"WSL2; Ubuntu",
				"WSL1; Debian",
				"Bash (MSYS2's full)",
				"Bash (Git's mini)",
				"PyCmd",
				"Node.js",
				"Python 3",
				"Windows Cmd",
				"Windows PowerShell 5",
				"Windows PowerShell 5 (relaxed)",
			]
		);
	}

	// The top of the list is the default shell (config::default_shell_argv), so on
	// unix nothing may sort above the user's own - and its startup-file-free twin
	// has to stay directly under it rather than sorting off among the others.
	#[test]
	fn the_login_shell_leads_and_its_twin_stays_under_it() {
		let titles = ordered(vec![
			hit("Bash", Group::Legacy, 0),
			hit("Zsh (no rc)", Group::LoginNoRc, 0),
			hit("Nushell", Group::Modern, 0),
			hit("Zsh", Group::Login, 0),
			hit("Python 3", Group::Language, 0),
		]);
		assert_eq!(
			titles,
			["Zsh", "Zsh (no rc)", "Nushell", "Python 3", "Bash"]
		);
	}

	// A curated group keeps its table order even when that disagrees with the
	// alphabet - the three POSIX-environment bashes are one shell built three
	// ways, and MSYS2's full one is the one to reach for first.
	#[test]
	fn a_curated_group_keeps_its_table_order() {
		let titles = ordered(vec![
			hit("Bash (Cygwin)", Group::PosixEnv, 2),
			hit("Bash (MSYS2's full)", Group::PosixEnv, 0),
			hit("Bash (Git's mini)", Group::PosixEnv, 1),
		]);
		assert_eq!(
			titles,
			["Bash (MSYS2's full)", "Bash (Git's mini)", "Bash (Cygwin)"]
		);
	}

	// The table is where each shell's place is declared, so the places the order
	// names outright are worth holding to it.
	#[test]
	fn the_table_puts_each_named_shell_where_the_order_says() {
		if cfg!(unix) {
			// no shell leads on merit here - the user's own does, whatever it is
			assert_eq!(known_group("pwsh").0, Group::Modern);
			assert_eq!(known_group("nu").0, Group::Modern);
			assert_eq!(known_group("bash").0, Group::Legacy);
			assert_eq!(known_group("fish").0, Group::Legacy);
			assert_eq!(known_group("python3").0, Group::Language);
		} else {
			assert_eq!(known_group("pwsh").0, Group::Pwsh7);
			assert_eq!(known_group("nu").0, Group::Modern);
			assert_eq!(known_group("pycmd").0, Group::PyCmd);
			assert_eq!(known_group("node").0, Group::Language);
			assert_eq!(known_group("cmd").0, Group::Legacy);
			assert_eq!(known_group("powershell").0, Group::WinPs5);
		}
		// anything the table does not name is an ordinary system shell
		assert_eq!(known_group("some-new-shell").0, Group::Legacy);
	}

	fn entry(slug: &str, command: &str, active: bool) -> ShellEntry {
		ShellEntry {
			slug: slug.into(),
			title: slug.into(),
			command: command.into(),
			active,
			comment: String::new(),
			last_seen: String::new(),
		}
	}

	// A fixed "today", so a stamped date is something to assert on rather than
	// whatever the clock says while the suite runs.
	const NOW: &str = "2026-08-19";

	fn merged(
		stored: &[ShellEntry],
		found: &[Found],
		resolve: &dyn Fn(&str) -> Option<PathBuf>,
	) -> Vec<ShellEntry> {
		merge_with(stored, found, resolve, NOW)
	}

	// Pretend every listed program is installed at /opt/<name> and nothing else is.
	fn installed(names: &'static [&'static str]) -> impl Fn(&str) -> Option<PathBuf> {
		move |prog: &str| {
			let base = base_name(prog);
			names
				.contains(&base.as_str())
				.then(|| PathBuf::from(format!("/opt/{base}")))
		}
	}

	#[test]
	fn a_shell_that_is_gone_is_switched_off_and_kept() {
		let stored = vec![entry("bash", "bash", true), entry("fish", "fish", true)];
		let out = merged(&stored, &[], &installed(&["bash"]));
		assert_eq!(out.len(), 2, "nothing is ever deleted");
		assert!(out[0].active);
		assert!(!out[1].active, "fish is not installed any more");
	}

	// The lopsided half: a scan adds, and it switches off. It must never switch
	// one back on - it cannot tell a returning program from a deliberate "no".
	#[test]
	fn a_scan_never_switches_a_shell_back_on() {
		let stored = vec![entry("fish", "fish", false)];
		let found = vec![Found::new("Fish", "fish".into(), "")];
		let out = merged(&stored, &found, &installed(&["fish"]));
		assert_eq!(out.len(), 1, "it is already stored, so nothing is added");
		assert!(!out[0].active);
	}

	#[test]
	fn a_shell_already_stored_is_not_added_twice() {
		let stored = vec![entry("bash", "/bin/bash", true)];
		let found = vec![Found::new("Bash", "bash".into(), "")];
		let out = merged(&stored, &found, &installed(&["bash"]));
		assert_eq!(out.len(), 1, "the bare name resolves to the stored path");
	}

	// Adding a banner flag to the table must not land a SECOND PowerShell beside
	// everyone's stored one, so a flag that changes only how a shell looks is
	// left out of what makes it that shell. The stored command is not rewritten
	// either - a scan never touches one - so an existing entry keeps its banner
	// until the user says otherwise.
	#[test]
	fn a_cosmetic_flag_does_not_make_it_a_different_shell() {
		let stored = vec![entry("pwsh", "/opt/ps/pwsh", true)];
		let found = vec![Found::new(
			"PowerShell 7",
			"/opt/ps/pwsh -NoLogo".into(),
			"",
		)];
		let out = merged(&stored, &found, &installed(&["pwsh", "/opt/ps/pwsh"]));
		assert_eq!(out.len(), 1, "a duplicate was appended");
		assert_eq!(
			out[0].command, "/opt/ps/pwsh",
			"the stored command was rewritten"
		);
		// spelled either way round, and case does not matter
		let stored = vec![entry("pwsh", "/opt/ps/pwsh -nologo", true)];
		let found = vec![Found::new("PowerShell 7", "/opt/ps/pwsh".into(), "")];
		assert_eq!(
			merged(&stored, &found, &installed(&["/opt/ps/pwsh"])).len(),
			1
		);
		// and a flag that changes BEHAVIOUR still makes a separate entry
		let stored = vec![entry("pwsh", "/opt/ps/pwsh -NoLogo", true)];
		let found = vec![Found::new(
			"relaxed",
			"/opt/ps/pwsh -NoLogo -ExecutionPolicy RemoteSigned".into(),
			"",
		)];
		assert_eq!(
			merged(&stored, &found, &installed(&["/opt/ps/pwsh"])).len(),
			2
		);
	}

	// Something offered as an alternative to another entry arrives switched OFF:
	// a list that grows a row is fine, one that grows a row the user has to
	// notice and turn back off is not.
	#[test]
	fn a_dormant_find_arrives_switched_off() {
		let found = vec![Found::dormant("Relaxed", "/opt/ps/pwsh -x".into(), "")];
		let out = merged(&[], &found, &installed(&["/opt/ps/pwsh"]));
		assert_eq!(out.len(), 1);
		assert!(!out[0].active, "it should arrive switched off");
		// ...and everything else still arrives switched on
		let found = vec![Found::new("Bash", "/bin/bash".into(), "")];
		assert!(merged(&[], &found, &installed(&["/bin/bash"]))[0].active);
	}

	// The arguments are part of what makes a shell one entry or two: the twin
	// that skips the startup files is the same program and a different shell.
	#[test]
	fn the_same_program_with_different_flags_is_a_different_shell() {
		let stored = vec![entry("bash", "bash", true)];
		let found = vec![Found::new("Bash (no rc)", "bash --norc".into(), "")];
		let out = merged(&stored, &found, &installed(&["bash"]));
		assert_eq!(out.len(), 2);
		assert_eq!(out[1].command, "bash --norc");
	}

	// Three environments ship a program called bash and they are not the same
	// shell. Matching on the resolved path is what keeps them apart.
	#[test]
	fn two_installed_shells_that_share_a_name_stay_apart() {
		let resolve = |prog: &str| {
			matches!(prog, r"C:\msys64\usr\bin\bash.exe" | r"C:\Git\bin\bash.exe")
				.then(|| PathBuf::from(prog))
		};
		let stored = vec![entry("msys2_bash", r"C:\msys64\usr\bin\bash.exe", true)];
		let found = vec![Found::new("Git Bash", r"C:\Git\bin\bash.exe".into(), "")];
		let out = merged(&stored, &found, &resolve);
		assert_eq!(out.len(), 2, "same name, different program");
	}

	// A shell that was uninstalled and put back must re-arm the entry it belongs
	// to rather than landing beside it as a second copy - which is what a strict
	// path match would do, since the disabled entry resolves nowhere.
	#[test]
	fn a_reinstalled_shell_rejoins_its_own_disabled_entry() {
		let stored = vec![entry("fish", "/usr/local/bin/fish", false)];
		let found = vec![Found::new("Fish", "/usr/bin/fish".into(), "")];
		let resolve = |prog: &str| (prog == "/usr/bin/fish").then(|| PathBuf::from(prog));
		let out = merged(&stored, &found, &resolve);
		assert_eq!(out.len(), 1, "no duplicate beside the disabled entry");
	}

	// Initial population IS the scan's order, and detect() leads with the login
	// shell - which is what puts the user's own shell at the top, where the top
	// means "the default". Nothing may quietly sort or group the findings.
	#[test]
	fn an_empty_list_takes_the_scan_in_the_order_it_found_them() {
		let found = vec![
			Found::new("Fish", "fish".into(), ""),
			Found::new("Bash", "bash".into(), ""),
			Found::new("Zsh", "zsh".into(), ""),
		];
		let out = merged(&[], &found, &installed(&["bash", "fish", "zsh"]));
		let titles: Vec<&str> = out.iter().map(|e| e.title.as_str()).collect();
		assert_eq!(titles, vec!["Fish", "Bash", "Zsh"]);
		assert!(out[0].active, "and the one at the top is usable");
	}

	#[test]
	fn an_adopted_command_is_titled_after_its_program() {
		let entry = adopted("/usr/bin/fish --login", &[]);
		assert_eq!(entry.title, "Fish");
		assert_eq!(entry.slug, "fish");
		assert!(entry.active);
		assert!(entry.last_seen.is_empty(), "no scan has vouched for it");
	}

	#[test]
	fn a_new_shell_lands_at_the_end_with_its_own_key() {
		let stored = vec![entry("bash", "bash", true)];
		let found = vec![Found::new("PowerShell 7", "pwsh".into(), "note")];
		let out = merged(&stored, &found, &installed(&["bash", "pwsh"]));
		assert_eq!(out.len(), 2);
		assert_eq!(out[1].slug, "powershell_7");
		assert_eq!(out[1].comment, "note");
		assert!(out[1].active);
	}

	#[test]
	fn a_key_that_is_taken_gets_a_number() {
		let stored = vec![entry("git_bash", "/a/bash", true)];
		let found = vec![Found::new("Git Bash", "/b/bash".into(), "")];
		let resolve = |prog: &str| Some(PathBuf::from(prog));
		let out = merged(&stored, &found, &resolve);
		assert_eq!(out[1].slug, "git_bash_2");
	}

	#[test]
	fn a_command_that_does_not_split_is_ignored_rather_than_stored() {
		let out = merged(&[], &[Found::new("Empty", String::new(), "")], &|_| {
			Some(PathBuf::from("/x"))
		});
		assert!(out.is_empty());
	}

	// A table key is looked up under the name the CODE will have by then, which
	// is a base name - lowercased on Windows, `.exe` stripped. A mixed-case key
	// therefore can never be found, and the shell it names silently loses its
	// friendly title. PyCmd shipped spelled "PyCmd" and did exactly that.
	#[test]
	fn a_known_shell_can_be_found_under_its_own_name() {
		for (exe, title, _, _) in KNOWN {
			assert_eq!(
				&pretty(&base_name(exe)),
				title,
				"{exe} cannot be looked up under the name it will be found by"
			);
		}
	}

	// The identity `adopt_default_shell` needs: a bare name and the full path to
	// the same file are one shell, and the question has to answer the same way
	// round either way, because the resolves-nowhere fallback is one-sided.
	#[test]
	fn a_bare_name_and_its_full_path_are_one_shell() {
		let real = |prog: &str| match prog {
			"pwsh" | "/opt/ps/pwsh" => Some(PathBuf::from("/opt/ps/pwsh")),
			"bash" | "/bin/bash" => Some(PathBuf::from("/bin/bash")),
			_ => None,
		};
		assert!(same_command_with("pwsh", "/opt/ps/pwsh", &real));
		assert!(same_command_with("/opt/ps/pwsh", "pwsh", &real));
		assert!(!same_command_with("pwsh", "/bin/bash", &real));
		// arguments are part of the identity, so a twin stays its own entry
		assert!(!same_command_with("pwsh", "/opt/ps/pwsh -NoProfile", &real));
		// and nothing that resolves nowhere is claimed to be anything
		assert!(!same_command_with("ghost", "/opt/ps/pwsh", &real));
	}

	// The twin exists for the shell the user actually logs in with, so the flag
	// has to be the right one per shell rather than bash's spelling for all.
	#[test]
	fn each_shell_skips_its_startup_files_its_own_way() {
		assert_eq!(no_startup_file("bash").map(|f| f.0), Some("--norc"));
		assert_eq!(no_startup_file("zsh").map(|f| f.0), Some("--no-rcs"));
		assert_eq!(no_startup_file("pwsh").map(|f| f.0), Some("-NoProfile"));
		assert_eq!(no_startup_file("dash"), None, "dash has no such flag");
		// cmd.exe is deliberately not on the list. It is the Windows login shell
		// (ComSpec), so it would get a twin on every Windows box - and an AutoRun
		// is rare enough that a second "Command Prompt" in everyone's Tabs menu
		// costs more than it is worth.
		assert_eq!(no_startup_file("cmd"), None, "cmd.exe gets no twin");
	}

	// The stamp is what makes the "Active" column trustworthy: a shell switched
	// off carries the date it was last there, so the switch is explicable.
	#[test]
	fn a_scan_dates_what_it_found_and_leaves_what_it_did_not() {
		let mut gone = entry("fish", "fish", true);
		gone.last_seen = "2026-01-02".into();
		let stored = vec![entry("bash", "bash", true), gone];
		let out = merged(&stored, &[], &installed(&["bash"]));
		assert_eq!(out[0].last_seen, NOW, "bash is installed today");
		assert_eq!(
			out[1].last_seen, "2026-01-02",
			"a shell that is gone keeps the date it was last seen"
		);
		assert!(!out[1].active);
	}

	#[test]
	fn a_newly_found_shell_is_dated_the_day_it_turned_up() {
		let found = vec![Found::new("Fish", "fish".into(), "")];
		let out = merged(&[], &found, &installed(&["fish"]));
		assert_eq!(out[0].last_seen, NOW);
	}

	// The epoch, a leap day, and a century that is not a leap year - the three
	// places the era arithmetic can go wrong.
	#[test]
	fn a_day_count_reads_as_the_date_it_is() {
		assert_eq!(civil_from_days(0), (1970, 1, 1));
		assert_eq!(civil_from_days(-1), (1969, 12, 31));
		assert_eq!(civil_from_days(19_417), (2023, 3, 1));
		assert_eq!(
			civil_from_days(19_416),
			(2023, 2, 28),
			"2023 is not a leap year"
		);
		assert_eq!(civil_from_days(18_321), (2020, 2, 29), "2020 is");
		assert_eq!(
			civil_from_days(11_016),
			(2000, 2, 29),
			"2000 is, despite the century"
		);
		assert_eq!(civil_from_days(20_684), (2026, 8, 19));
	}

	#[test]
	fn a_path_with_a_space_is_quoted_and_survives_the_split() {
		let quoted = quoted(Path::new(r"C:\Program Files\Git\bin\bash.exe"));
		let argv = crate::cli::shell_split(&quoted).expect("splits");
		assert_eq!(argv, vec![r"C:\Program Files\Git\bin\bash.exe"]);
	}
}
