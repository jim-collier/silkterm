// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! What gets set up in the shells this terminal starts: a directory-reporting
//! block in PowerShell profiles, and a git-aware prompt for both PowerShell and
//! bash.
//!
//! The PowerShell half:
//!
//! Every other shell moves its own process when it moves, so the operating
//! system can be asked where it is and nothing needs setting up (see `cwd.rs`).
//! PowerShell keeps its location to itself, so it has to say where it is - and
//! asking every user to paste a block into a file to make new tabs open in the
//! right place is a poor trade when the block can be put there for them.
//!
//! What that licenses is narrow, and the limits are the design:
//!
//! - Only a profile that reports NOTHING is touched. Our own marker, or any
//!   other OSC 7 / OSC 9;9 already in the file (a Windows Terminal setup, say),
//!   means somebody has this in hand and the file is left alone.
//! - Only ever APPENDED to, after a copy is kept beside it, and only once - the
//!   marker is what makes a second launch a no-op. The one exception is the
//!   block ITSELF, which is kept current in place: it gains things over time
//!   (the version prompt did), and an install that only ever appends would
//!   leave everyone who already had it on the first version forever. That edit
//!   is only safe because the region is delimited by our own two markers and
//!   was written by us - which is exactly the signal a stored shell entry
//!   lacks, and why THAT list may only ever be added to.
//! - Deleting the block is how it is switched off; nothing puts it back, since
//!   the block is gone and no marker is left to match. `shell.integration`
//!   switches the whole thing off before it starts.
//! - A shell that would refuse to load the profile is left alone. Measured on
//!   this box: Windows PowerShell 5.1 sits at a policy that blocks script
//!   files, so a profile written for it turned every launch into a red
//!   execution-policy error. Writing a file a shell cannot read is worse than
//!   doing nothing, and changing somebody's execution policy is not ours to do.
//! - It runs on the shell-scan thread, well after the window is up, because it
//!   asks PowerShell itself where its profile is - which means starting one.
//!
//! The block also carries the prompt, rather than pointing at a script written
//! beside the config the way the bash half does. A prompt is drawn after every
//! command, and on Windows starting a process that often is not free.
//!
//! The bash half is a much smaller thing, and deliberately so. bash picks up
//! `PROMPT_COMMAND` from its environment, and an rc file that sets one of its
//! own runs afterwards and wins - so a pane is OFFERED a prompt rather than
//! given one, and anybody who already has a prompt keeps it without knowing
//! this exists. Nothing is written into anyone's rc file, and switching it off
//! is a setting rather than an uninstall.

use std::path::{Path, PathBuf};

use crate::config;
use crate::shells::Found;

// The block, and the marker that says it is already there. Compiled in so the
// binary is the one source of it - `shell-integration.md` documents the same
// text for anyone adding it by hand, and a test holds the two together.
//
// It is written out as plain UTF-8 with no byte-order mark, and Windows
// PowerShell 5.1 reads such a file as ANSI - so the block itself has to stay
// ASCII, and the glyphs its prompt draws are spelled as code points.
pub const SNIPPET: &str = include_str!("shell_integration.ps1");
// Named rather than spelled at each use: this module compares and rewrites
// line endings constantly, and an escape is easy to get subtly wrong.
const LF: &str = "\n";
const CRLF: &str = "\r\n";
const NL: char = '\n';

pub const MARKER: &str = "# >>> SilkTerm shell integration >>>";
pub const END_MARKER: &str = "# <<< SilkTerm shell integration <<<";

// A file that already carries either sequence is reporting - by our block or by
// somebody else's setup - and is not ours to edit.
pub fn already_reports(profile: &str) -> bool {
	profile.contains(MARKER) || profile.contains("]9;9;") || profile.contains("]7;file:")
}

// The profile with the block on the end, separated by a blank line and starting
// on one of its own. Existing content is never rewritten, only followed.
pub fn with_block(profile: &str, newline: &str) -> String {
	let block = SNIPPET.replace("\r\n", "\n").replace('\n', newline);
	if profile.trim().is_empty() {
		return block;
	}
	let mut out = profile.to_string();
	if !out.ends_with('\n') {
		out.push_str(newline);
	}
	out.push_str(newline);
	out.push_str(&block);
	out
}

// The PowerShell programs among a scan's findings, one per program. A shell is
// named by its argv, so the program is the first word of it; the no-startup-file
// twins collapse into the same program, and their profile is the same file.
pub fn powershells(found: &[Found]) -> Vec<String> {
	let mut out: Vec<String> = Vec::new();
	for entry in found {
		let Ok(argv) = crate::cli::shell_split(&entry.command) else {
			continue;
		};
		let Some(program) = argv.first() else {
			continue;
		};
		if !is_powershell(program) {
			continue;
		}
		// Windows hands the same file back under more than one spelling of its
		// path (%SystemRoot% is C:\WINDOWS, PATH says C:\Windows), and each
		// one costs a shell launched to ask it the same question and a second
		// copy of the same diagnostic.
		let same = |seen: &String| {
			if cfg!(windows) {
				seen.eq_ignore_ascii_case(program)
			} else {
				seen == program
			}
		};
		if !out.iter().any(same) {
			out.push(program.clone());
		}
	}
	out
}

// Is this program a PowerShell? Matched on the base name the way the shell
// table is (lowercased, `.exe` dropped), so a full path answers the same as a
// bare name - and `pwsh-preview` and the like answer yes as well.
fn is_powershell(program: &str) -> bool {
	// split on both separators: a Windows path reaches this on any platform, and
	// Path would hand back the whole string for one on unix
	let base = program
		.rsplit(['/', '\\'])
		.next()
		.unwrap_or(program)
		.to_ascii_lowercase();
	let base = base.strip_suffix(".exe").unwrap_or(&base);
	base == "powershell" || base == "pwsh" || base.starts_with("pwsh-")
}

// Put the block in every PowerShell profile that reports nothing. Called on the
// shell-scan thread; every failure is a diagnostic, never a stop - a profile we
// cannot read or write is somebody else's business.
pub fn install(found: &[Found]) {
	if !config::settings().shell_integration {
		return;
	}
	let mut done: Vec<PathBuf> = Vec::new();
	for program in powershells(found) {
		let Some((profile, policy)) = ask_shell(&program) else {
			continue;
		};
		if !policy_runs_scripts(&policy) {
			let answer = if policy.is_empty() {
				"no answer"
			} else {
				&policy
			};
			eprintln!(
				"{}: {program} will not run profile scripts ({answer}), so its profile was left alone - see shell-integration.md",
				config::APP_NAME
			);
			continue;
		}
		// two PowerShells can share a profile; only look at each file once
		if done.contains(&profile) {
			continue;
		}
		done.push(profile.clone());
		install_into(&profile);
	}
}

// Ask a PowerShell where its own profile is. There is no way to work this out
// from outside - the Documents folder it sits under can be redirected, and per
// host and per version it differs - so the shell is asked, which means starting
// one. On Windows that starts a console with it, hence CREATE_NO_WINDOW: a
// console flashing over the terminal a few seconds after launch would be a
// mystery to anyone who saw it.
fn ask_shell(program: &str) -> Option<(PathBuf, String)> {
	let mut command = std::process::Command::new(program);
	// both facts in one launch: where the profile is, and whether this shell
	// would even run it
	command.args([
		"-NoProfile",
		"-NonInteractive",
		"-Command",
		"$PROFILE; Get-ExecutionPolicy",
	]);
	#[cfg(windows)]
	{
		use std::os::windows::process::CommandExt;
		const CREATE_NO_WINDOW: u32 = 0x0800_0000;
		command.creation_flags(CREATE_NO_WINDOW);
	}
	// The exit status is deliberately not consulted. On a locked-down box the
	// policy question ANSWERS ITSELF by failing: Get-ExecutionPolicy lives in a
	// module whose manifest is a script file, so Restricted stops it loading and
	// the shell exits non-zero - while the profile path still comes back on
	// stdout. Reading only the status would turn that into silence, which is
	// how this first went wrong.
	let output = command.output().ok()?;
	let answer = String::from_utf8_lossy(&output.stdout);
	let mut lines = answer
		.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty());
	let path = lines.next()?.to_string();
	let policy = lines.next().unwrap_or_default().to_string();
	(!path.is_empty()).then(|| (PathBuf::from(path), policy))
}

// Would this shell actually load a profile it found? A block written into a
// profile the shell then refuses to run is worse than none at all: measured
// here, Windows PowerShell 5.1 sits at Restricted, and a profile written for it
// turned every launch into a red execution-policy error. An answer we do not
// recognise - including no answer at all - is treated as "no": the cost of
// being wrong that way is a feature that does not switch itself on, against an
// error on somebody's every prompt.
fn policy_runs_scripts(policy: &str) -> bool {
	matches!(
		policy.trim().to_ascii_lowercase().as_str(),
		"remotesigned" | "unrestricted" | "bypass"
	)
}

// The profile with our block brought up to date, or None when there is nothing
// to do - no block of ours in the file, or the one there is already current.
// Only the text BETWEEN the two markers is touched; whatever the user wrote
// above or below it is carried through untouched. An opening marker with no
// closing one is not a block we finished writing, so it is not a region we may
// replace either.
pub fn refreshed_block(profile: &str, newline: &str) -> Option<String> {
	let start = profile.find(MARKER)?;
	let end_marker = profile[start..].find(END_MARKER)? + start;
	let end = profile[end_marker..]
		.find(NL)
		.map_or(profile.len(), |nl| end_marker + nl + 1);
	let block = SNIPPET.replace(CRLF, LF).replace(NL, newline);
	if profile[start..end].replace(CRLF, LF) == block.replace(CRLF, LF) {
		return None;
	}
	Some(format!("{}{block}{}", &profile[..start], &profile[end..]))
}

fn install_into(profile: &Path) {
	let existing = std::fs::read_to_string(profile).unwrap_or_default();
	// a profile is read by the platform's own shell, so it gets the platform's
	// line ending rather than whatever the compiled-in copy carries
	let newline = if cfg!(windows) { CRLF } else { LF };
	// already ours: the only thing left to do is bring it up to date
	if existing.contains(MARKER) {
		if let Some(updated) = refreshed_block(&existing, newline) {
			match std::fs::write(profile, updated) {
				Ok(()) => eprintln!(
					"{}: updated the shell integration block in {}",
					config::APP_NAME,
					profile.display()
				),
				Err(e) => eprintln!(
					"{}: could not write {}: {e}",
					config::APP_NAME,
					profile.display()
				),
			}
		}
		return;
	}
	if already_reports(&existing) {
		return;
	}
	// keep what is there, under a name that says where it came from - and never
	// over a backup already made, which would be the one thing worth keeping
	if !existing.trim().is_empty() {
		let backup = profile.with_extension("ps1.silkterm-backup");
		if !backup.exists() {
			if let Err(e) = std::fs::copy(profile, &backup) {
				eprintln!(
					"{}: could not back up {}: {e} - left it alone",
					config::APP_NAME,
					profile.display()
				);
				return;
			}
		}
	}
	if let Some(parent) = profile.parent() {
		if let Err(e) = std::fs::create_dir_all(parent) {
			eprintln!("{}: {}: {e}", config::APP_NAME, parent.display());
			return;
		}
	}
	match std::fs::write(profile, with_block(&existing, newline)) {
		Ok(()) => eprintln!(
			"{}: added shell integration to {} - new tabs and panes will open where the shell is (see shell-integration.md)",
			config::APP_NAME,
			profile.display()
		),
		Err(e) => eprintln!(
			"{}: could not write {}: {e}",
			config::APP_NAME,
			profile.display()
		),
	}
}

// The prompt script itself, compiled in so the binary is the one source of it.
// x9ps1-git is a separate MIT project of the same author; this is a copy of its
// `bin/x9ps1-git`, and the version it carries is in its own header.
const BASH_PROMPT: &str = include_str!("x9ps1-git.bash");

// What the script is called once it is on disk. No extension, because it is
// also perfectly usable by hand from a PATH directory.
const BASH_PROMPT_FILE: &str = "x9ps1-git";

// Is this program bash? Same base-name matching as `is_powershell`, so Git Bash
// and a full path both answer yes, while `sh` (which may well be dash) does not.
fn is_bash(program: &str) -> bool {
	let base = program
		.rsplit(['/', '\\'])
		.next()
		.unwrap_or(program)
		.to_ascii_lowercase();
	base.strip_suffix(".exe").unwrap_or(&base) == "bash"
}

// The PROMPT_COMMAND a bash pane is given, for a script sitting at `path`.
//
// `$BASH` is bash's own path, so the script runs under the same bash the pane
// does with no dependency on what is on PATH and no execute bit needed. The
// path is spelled with forward slashes and single-quoted, which every bash
// takes - including a Windows one, where a backslash inside quotes would
// otherwise arrive as an escape.
fn prompt_command(path: &Path) -> String {
	let quoted = path
		.display()
		.to_string()
		.replace('\\', "/")
		.replace('\'', "'\\''");
	format!("PS1=$(\"$BASH\" '{quoted}')")
}

// Put the script in the data directory, once per run, and say where it is.
// Rewritten whenever it differs, so an updated SilkTerm carries an updated
// prompt rather than leaving the first copy standing forever.
fn bash_prompt_path() -> Option<&'static Path> {
	static PATH: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
	PATH.get_or_init(|| {
		let dir = config::data_dir()?;
		let path = dir.join(BASH_PROMPT_FILE);
		if std::fs::read_to_string(&path).is_ok_and(|held| held == BASH_PROMPT) {
			return Some(path);
		}
		std::fs::create_dir_all(&dir).ok()?;
		match std::fs::write(&path, BASH_PROMPT) {
			Ok(()) => Some(path),
			Err(e) => {
				eprintln!(
					"{}: could not write {}: {e}",
					config::APP_NAME,
					path.display()
				);
				None
			}
		}
	})
	.as_deref()
}

// The environment a pane about to run `command` should start with, on top of
// what it inherits. Empty for anything that is not bash, and for a bash pane
// when the setting is off.
pub fn pane_env(command: Option<&[String]>) -> Vec<(String, String)> {
	if !config::settings().bash_prompt {
		return Vec::new();
	}
	let Some(program) = command.and_then(<[String]>::first) else {
		return Vec::new();
	};
	if !is_bash(program) {
		return Vec::new();
	}
	bash_prompt_path()
		.map(|path| vec![("PROMPT_COMMAND".to_string(), prompt_command(path))])
		.unwrap_or_default()
}

#[cfg(test)]
mod tests {
	use super::{
		BASH_PROMPT, END_MARKER, LF, MARKER, SNIPPET, already_reports, is_bash, is_powershell,
		powershells, prompt_command, refreshed_block, with_block,
	};
	use crate::shells::Found;

	// sh may well be dash, and a shell named by a full path is the ordinary case
	// on Windows - so both have to answer the way a bare `bash` does.
	#[test]
	fn only_bash_is_offered_the_bash_prompt() {
		assert!(is_bash("bash"));
		assert!(is_bash("/usr/bin/bash"));
		assert!(is_bash("C:\\Program Files\\Git\\bin\\bash.exe"));
		assert!(!is_bash("sh"));
		assert!(!is_bash("zsh"));
		assert!(!is_bash("wsl.exe"));
	}

	// The value is handed to bash as a command string, so a Windows path has to
	// arrive as something bash reads rather than as a run of escapes.
	#[test]
	fn a_prompt_command_survives_a_windows_path() {
		let win = prompt_command(std::path::Path::new("C:\\Users\\me\\x9ps1-git"));
		assert_eq!(win, "PS1=$(\"$BASH\" 'C:/Users/me/x9ps1-git')");
		let unix = prompt_command(std::path::Path::new("/home/me/.config/silkterm/x9ps1-git"));
		assert_eq!(
			unix,
			"PS1=$(\"$BASH\" '/home/me/.config/silkterm/x9ps1-git')"
		);
	}

	// The compiled-in copy is what gets written out and then run by bash, so a
	// truncated or mangled vendoring should not reach anybody's prompt.
	#[test]
	fn the_bash_prompt_script_is_a_whole_script() {
		assert!(BASH_PROMPT.starts_with("#!/bin/bash"));
		assert!(BASH_PROMPT.contains("x9ps1-git v"));
		assert!(BASH_PROMPT.contains("fMain"));
	}

	fn found(title: &str, command: &str) -> Found {
		Found::new(title, command.to_string(), "")
	}

	// A profile that already reports is not ours to edit, whoever set it up -
	// and this is also what makes a second launch a no-op rather than a second
	// copy of the block.
	#[test]
	fn a_profile_that_already_reports_is_left_alone() {
		assert!(already_reports(&with_block("", "\n")), "our own block");
		assert!(
			already_reports("Write-Host \"$e]9;9;`\"$p`\"$e\\\""),
			"somebody else's OSC 9;9"
		);
		assert!(
			already_reports("printf '\\033]7;file://%s%s' $h $p"),
			"somebody else's OSC 7"
		);
		assert!(!already_reports(
			"# just a profile\nSet-Alias ll Get-ChildItem"
		));
		assert!(!already_reports(""));
	}

	// What is there is followed, never rewritten - so a prompt the profile sets
	// up further down is the one this block wraps.
	#[test]
	fn the_block_lands_at_the_end_and_keeps_what_was_there() {
		let before = "Import-Module Cows\r\nSet-Alias ll Get-ChildItem\r\n";
		let after = with_block(before, "\r\n");
		assert!(after.starts_with(before), "existing content moved");
		assert!(after.contains(MARKER));
		assert!(after.ends_with(&SNIPPET.replace("\r\n", "\n").replace('\n', "\r\n")));
		// a file with no trailing newline still gets the block on its own line
		let joined = with_block("Set-Alias ll Get-ChildItem", "\n");
		assert!(joined.contains("Get-ChildItem\n\n# >>> SilkTerm"));
		// and an empty profile is just the block
		assert_eq!(with_block("   \n", "\n"), SNIPPET.replace("\r\n", "\n"));
	}

	// The whole of what a launch does to a file that is not ours: keep a copy,
	// follow what is there, and never do it twice.
	#[test]
	fn a_profile_is_backed_up_once_and_added_to_once() {
		let dir = std::env::temp_dir().join("silkterm-integration-test");
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("temp dir");
		let profile = dir.join("Microsoft.PowerShell_profile.ps1");
		let before = "Set-Alias ll Get-ChildItem\n";
		std::fs::write(&profile, before).expect("write profile");

		super::install_into(&profile);
		let after = std::fs::read_to_string(&profile).expect("read profile");
		assert!(after.starts_with(before), "what was there survived");
		assert!(after.contains(MARKER), "the block went in");
		let backup = dir.join("Microsoft.PowerShell_profile.ps1.silkterm-backup");
		assert_eq!(
			std::fs::read_to_string(&backup).ok().as_deref(),
			Some(before),
			"the copy beside it is the file as it was"
		);

		// a second launch is a no-op, and cannot overwrite the copy either
		std::fs::write(&profile, format!("{after}# a line the user added\n")).unwrap();
		super::install_into(&profile);
		let twice = std::fs::read_to_string(&profile).expect("read profile");
		assert_eq!(twice.matches(MARKER).count(), 1, "a second block went in");
		assert!(
			twice.ends_with("# a line the user added\n"),
			"their line went"
		);
		assert_eq!(
			std::fs::read_to_string(&backup).ok().as_deref(),
			Some(before),
			"the copy was overwritten"
		);

		// and a profile that never existed is created with just the block
		let fresh = dir.join("fresh").join("Microsoft.PowerShell_profile.ps1");
		super::install_into(&fresh);
		assert!(
			std::fs::read_to_string(&fresh).unwrap().contains(MARKER),
			"a missing profile was not created"
		);
		assert!(!fresh.with_extension("ps1.silkterm-backup").exists());
		let _ = std::fs::remove_dir_all(&dir);
	}

	// A block in a profile the shell will not run is worse than no block: it is
	// an execution-policy error on every launch, which is what a first cut of
	// this did to Windows PowerShell 5.1 on the box it was written on.
	#[test]
	fn a_shell_that_will_not_run_scripts_keeps_its_profile() {
		for allowed in ["RemoteSigned", "Unrestricted", "Bypass", " bypass "] {
			assert!(super::policy_runs_scripts(allowed), "{allowed}");
		}
		for blocked in ["Restricted", "AllSigned", "Undefined", "", "who knows"] {
			assert!(!super::policy_runs_scripts(blocked), "{blocked}");
		}
	}

	// Only PowerShell needs this: every other shell moves its own process, so
	// the OS can be asked. A full path answers the same as a bare name.
	#[test]
	fn only_the_powershells_are_offered_a_profile() {
		assert!(is_powershell("pwsh"));
		assert!(is_powershell("pwsh.exe"));
		assert!(is_powershell(r"C:\Program Files\PowerShell\7\pwsh.exe"));
		assert!(is_powershell("PowerShell.EXE"));
		assert!(is_powershell("/usr/bin/pwsh"));
		assert!(is_powershell("pwsh-preview"));
		assert!(!is_powershell("bash"));
		assert!(!is_powershell("cmd.exe"));
		assert!(!is_powershell("powershell-ise.exe"), "a different program");

		// one entry per PROGRAM: the no-startup-file twin is the same shell with
		// the same profile, and a scan lists both
		let scan = [
			found("Bash", "/bin/bash"),
			found("PowerShell 7", r"C:\pwsh.exe"),
			found("PowerShell 7 (no profile)", r"C:\pwsh.exe -NoProfile"),
			found("Windows PowerShell 5", "powershell.exe"),
		];
		assert_eq!(
			powershells(&scan),
			vec![r"C:\pwsh.exe".to_string(), "powershell.exe".to_string()]
		);
	}

	// The block gains things over time, so an install that only ever appended
	// would leave anyone who already has it on whatever version they first got.
	#[test]
	fn an_existing_block_is_brought_up_to_date_in_place() {
		let stale = format!(
			"# mine, above\n\n{MARKER}\nWrite-Host 'an older block'\n{END_MARKER}\n\n# mine, below\n"
		);
		let updated = refreshed_block(&stale, LF).expect("a stale block needs replacing");
		assert!(
			updated.starts_with("# mine, above\n"),
			"lost what was above"
		);
		assert!(updated.ends_with("# mine, below\n"), "lost what was below");
		assert!(
			!updated.contains("an older block"),
			"kept the old block: {updated}"
		);
		assert!(updated.contains(SNIPPET.replace("\r\n", LF).trim_end()));
		// ...and having done it once, there is nothing left to do.
		assert_eq!(refreshed_block(&updated, LF), None);
	}

	#[test]
	fn a_profile_with_no_block_of_ours_is_not_rewritten() {
		assert_eq!(refreshed_block("# just my own profile\n", LF), None);
		assert_eq!(
			refreshed_block(&format!("{MARKER}\nhalf a block\n"), LF),
			None
		);
	}

	// A profile with no byte-order mark is read as ANSI by Windows PowerShell
	// 5.1, so a single accented character or box-drawing glyph in here arrives
	// mangled on the one version that cannot be told otherwise.
	#[test]
	fn the_block_is_plain_ascii() {
		let stray: String = SNIPPET.chars().filter(|c| !c.is_ascii()).collect();
		assert!(stray.is_empty(), "non-ascii in the block: {stray}");
	}

	// The prompt is part of the block rather than a script beside it, and both
	// halves of the version split have to reach it.
	#[test]
	fn the_block_carries_the_prompt() {
		assert!(SNIPPET.contains("function global:__SilkTermPrompt"));
		assert!(SNIPPET.contains("git status --porcelain=v2 --branch"));
		// the 6+ hook branch, and the 5.1 wrap
		assert_eq!(SNIPPET.matches("__SilkTermPrompt }").count(), 2);
	}

	// The block people are told to paste in by hand has to be the block that
	// gets installed, or one of the two quietly stops being true.
	#[test]
	fn the_documented_snippet_is_the_one_that_is_installed() {
		let doc = include_str!("../../shell-integration.md").replace("\r\n", "\n");
		let snippet = SNIPPET.replace("\r\n", "\n");
		assert!(
			doc.contains(snippet.trim_end()),
			"shell-integration.md no longer carries the snippet verbatim"
		);
	}
}
