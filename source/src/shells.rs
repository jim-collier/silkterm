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
}

// One shell a scan turned up. It becomes a `ShellEntry` only if the stored list
// has nothing already running the same program.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
	pub title: String,
	pub command: String,
	pub comment: String,
}

impl Found {
	fn new(title: &str, command: String, comment: &str) -> Self {
		Self {
			title: title.to_string(),
			command,
			comment: comment.to_string(),
		}
	}
}

// Run a scan on its own thread and post the merged list back to the event loop.
// A thread per scan rather than a long-lived worker: a PATH entry on a dead
// mount blocks its own thread forever, and there is nothing queued behind it.
pub fn spawn(proxy: &EventLoopProxy<UserEvent>, stored: Vec<ShellEntry>) {
	let proxy = proxy.clone();
	let spawned = std::thread::Builder::new()
		.name("shells".into())
		.spawn(move || {
			let merged = merge(&stored, &detect());
			let _ = proxy.send_event(UserEvent::ShellsReady(merged));
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
	merge_with(stored, found, &which)
}

fn merge_with(
	stored: &[ShellEntry],
	found: &[Found],
	resolve: &dyn Fn(&str) -> Option<PathBuf>,
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
			active: true,
			comment: hit.comment.clone(),
		});
	}
	out
}

// What makes two command lines "the same shell". `exe` is where the program
// resolved to right now (absolute, lowercased on Windows), `base` its bare name.
struct Ident {
	exe: Option<PathBuf>,
	base: String,
	args: Vec<String>,
}

impl Ident {
	fn of(command: &str, resolve: &dyn Fn(&str) -> Option<PathBuf>) -> Option<Self> {
		let argv = crate::cli::shell_split(command).ok()?;
		let (prog, args) = argv.split_first()?;
		Some(Self {
			exe: resolve(prog).map(|p| norm(&p)),
			base: base_name(prog),
			args: args.to_vec(),
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
		"cmd" => ("/d", "no AutoRun"),
		_ => return None,
	})
}

// A friendly name for a program we found by its bare name.
fn pretty(base: &str) -> String {
	for (exe, title, _) in KNOWN {
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

// The shells worth looking for by name, in the order they should be offered.
// Interactive shells first, then the language REPLs that people do use as one.
// A name not on this list is still found when it is the user's login shell.
#[cfg(unix)]
const KNOWN: &[(&str, &str, &str)] = &[
	("bash", "Bash", ""),
	("zsh", "Zsh", ""),
	("fish", "Fish", ""),
	("dash", "Dash", ""),
	("ash", "Ash", ""),
	("ksh", "Korn shell", ""),
	("mksh", "MirBSD Korn shell", ""),
	("yash", "Yash", ""),
	("tcsh", "Tcsh", ""),
	("csh", "C shell", ""),
	("sh", "POSIX shell", ""),
	("nu", "Nushell", "structured data through the pipeline"),
	("elvish", "Elvish", ""),
	("xonsh", "Xonsh", "Python syntax with shell primitives"),
	("ysh", "YSH", "the Oils shell"),
	("osh", "OSH", "the Oils shell, bash-compatible"),
	("murex", "Murex", ""),
	("ion", "Ion", ""),
	("es", "Es", ""),
	("rc", "rc", "the Plan 9 shell"),
	("pwsh", "PowerShell 7", ""),
	("python3", "Python 3", ""),
	("ipython", "IPython", ""),
	("node", "Node.js", ""),
];

#[cfg(not(unix))]
const KNOWN: &[(&str, &str, &str)] = &[
	("pwsh", "PowerShell 7", ""),
	(
		"powershell",
		"Windows PowerShell",
		"the 5.1 shell that ships with Windows",
	),
	("cmd", "Command Prompt", ""),
	("nu", "Nushell", "structured data through the pipeline"),
	("PyCmd", "PyCmd", "cmd.exe with completion and history"),
	("elvish", "Elvish", ""),
	("xonsh", "Xonsh", "Python syntax with shell primitives"),
	("python", "Python 3", ""),
	("node", "Node.js", ""),
];

// Everything installed that looks like a shell, best first.
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

	// The user's own shell leads, and it is the one that gets the twin that
	// skips its startup files.
	if let Some(login) = login_shell() {
		let base = base_name(&login);
		let title = pretty(&base);
		add(Found::new(&title, login.clone(), ""), &mut out, &mut seen);
		if let Some((flag, note)) = no_startup_file(&base) {
			add(
				Found::new(&format!("{title} ({note})"), format!("{login} {flag}"), ""),
				&mut out,
				&mut seen,
			);
		}
	}
	for (exe, title, comment) in KNOWN {
		if let Some(path) = which(exe) {
			add(
				Found::new(title, quoted(&path), comment),
				&mut out,
				&mut seen,
			);
		}
	}
	for hit in platform_extras() {
		add(hit, &mut out, &mut seen);
	}
	out
}

// The shell the user logs in with. $SHELL is what a person means by "my shell"
// and every desktop session sets it; on Windows there is no such thing, and the
// table below leads instead.
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
			Found::new(&pretty(&base), line.to_string(), "")
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
		for (rel, title, comment) in [
			(
				r"System32\WindowsPowerShell\v1.0\powershell.exe",
				"Windows PowerShell",
				"the 5.1 shell that ships with Windows",
			),
			(r"System32\cmd.exe", "Command Prompt", ""),
		] {
			let path = root.join(rel);
			if path.is_file() {
				out.push(Found::new(title, quoted(&path), comment));
			}
		}
	}
	// The POSIX environments each ship their own bash. They share a name and
	// nothing else, so each is offered under the environment it belongs to.
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
		out.push(Found::new(
			"Git Bash",
			quoted(&path),
			"MSYS2-based, from Git for Windows",
		));
	}
	for (path, title, comment) in [
		(r"C:\msys64\usr\bin\bash.exe", "MSYS2 Bash", ""),
		(r"C:\msys32\usr\bin\bash.exe", "MSYS2 Bash", ""),
		(r"C:\cygwin64\bin\bash.exe", "Cygwin Bash", ""),
		(r"C:\cygwin\bin\bash.exe", "Cygwin Bash", ""),
	] {
		let path = Path::new(path);
		if path.is_file() {
			out.push(Found::new(title, quoted(path), comment));
		}
	}
	// WSL distributions, read from the registry rather than by asking wsl.exe:
	// a WSL2 distribution lives in a virtual disk, and listing them must not be
	// the thing that boots the virtual machine. What is offered is the whole
	// distribution, with no shell named - its own default runs, and the user can
	// add flags to the entry if they want a particular one.
	for name in wsl_distributions() {
		let title = format!("WSL: {name}");
		let command = format!("wsl.exe -d {}", quoted(Path::new(&name)));
		out.push(Found::new(&title, command, "the distribution's own shell"));
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

// Installed WSL distributions, by name, straight out of the registry. Nothing
// is launched: the key is written when a distribution is registered.
#[cfg(windows)]
fn wsl_distributions() -> Vec<String> {
	use windows_sys::Win32::Foundation::ERROR_SUCCESS;
	use windows_sys::Win32::System::Registry::{
		HKEY, HKEY_CURRENT_USER, KEY_READ, REG_SZ, RegCloseKey, RegEnumKeyExW, RegOpenKeyExW,
		RegQueryValueExW,
	};

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
		// SAFETY: the handle came from a successful open above.
		unsafe { RegCloseKey(distro) };
		if read != ERROR_SUCCESS || kind != REG_SZ {
			continue;
		}
		let chars = (bytes as usize / 2).min(buf.len());
		let name = String::from_utf16_lossy(&buf[..chars]);
		let name = name.trim_end_matches('\0').trim().to_string();
		if !name.is_empty() {
			out.push(name);
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

	fn entry(slug: &str, command: &str, active: bool) -> ShellEntry {
		ShellEntry {
			slug: slug.into(),
			title: slug.into(),
			command: command.into(),
			active,
			comment: String::new(),
		}
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
		let out = merge_with(&stored, &[], &installed(&["bash"]));
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
		let out = merge_with(&stored, &found, &installed(&["fish"]));
		assert_eq!(out.len(), 1, "it is already stored, so nothing is added");
		assert!(!out[0].active);
	}

	#[test]
	fn a_shell_already_stored_is_not_added_twice() {
		let stored = vec![entry("bash", "/bin/bash", true)];
		let found = vec![Found::new("Bash", "bash".into(), "")];
		let out = merge_with(&stored, &found, &installed(&["bash"]));
		assert_eq!(out.len(), 1, "the bare name resolves to the stored path");
	}

	// The arguments are part of what makes a shell one entry or two: the twin
	// that skips the startup files is the same program and a different shell.
	#[test]
	fn the_same_program_with_different_flags_is_a_different_shell() {
		let stored = vec![entry("bash", "bash", true)];
		let found = vec![Found::new("Bash (no rc)", "bash --norc".into(), "")];
		let out = merge_with(&stored, &found, &installed(&["bash"]));
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
		let out = merge_with(&stored, &found, &resolve);
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
		let out = merge_with(&stored, &found, &resolve);
		assert_eq!(out.len(), 1, "no duplicate beside the disabled entry");
	}

	#[test]
	fn a_new_shell_lands_at_the_end_with_its_own_key() {
		let stored = vec![entry("bash", "bash", true)];
		let found = vec![Found::new("PowerShell 7", "pwsh".into(), "note")];
		let out = merge_with(&stored, &found, &installed(&["bash", "pwsh"]));
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
		let out = merge_with(&stored, &found, &resolve);
		assert_eq!(out[1].slug, "git_bash_2");
	}

	#[test]
	fn a_command_that_does_not_split_is_ignored_rather_than_stored() {
		let out = merge_with(&[], &[Found::new("Empty", String::new(), "")], &|_| {
			Some(PathBuf::from("/x"))
		});
		assert!(out.is_empty());
	}

	// The twin exists for the shell the user actually logs in with, so the flag
	// has to be the right one per shell rather than bash's spelling for all.
	#[test]
	fn each_shell_skips_its_startup_files_its_own_way() {
		assert_eq!(no_startup_file("bash").map(|f| f.0), Some("--norc"));
		assert_eq!(no_startup_file("zsh").map(|f| f.0), Some("--no-rcs"));
		assert_eq!(no_startup_file("pwsh").map(|f| f.0), Some("-NoProfile"));
		assert_eq!(no_startup_file("cmd").map(|f| f.0), Some("/d"));
		assert_eq!(no_startup_file("dash"), None, "dash has no such flag");
	}

	#[test]
	fn a_path_with_a_space_is_quoted_and_survives_the_split() {
		let quoted = quoted(Path::new(r"C:\Program Files\Git\bin\bash.exe"));
		let argv = crate::cli::shell_split(&quoted).expect("splits");
		assert_eq!(argv, vec![r"C:\Program Files\Git\bin\bash.exe"]);
	}
}
