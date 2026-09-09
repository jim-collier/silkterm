//! What a tab says, and how it is shortened to fit.
//!
//! A tab reads "<shell> [<task>] <path>": the shell's friendly name, the command
//! it is running (or `[last: cmd]` for the one it just finished), and where it
//! is. A shell that has never run anything drops the brackets and reads
//! "<shell> - <path>" instead.
//!
//! When that does not fit, the parts give way in a fixed order, each rung
//! strictly narrower than the one above it: the shell's name shortens first,
//! then the task's name is truncated, then the path abbreviates, then the task
//! goes altogether, then the path does, and the last rung is the shortest form
//! of the shell's name on its own. The caller measures the rungs in order and
//! takes the first that fits (`label_forms`).
//!
//! The path is shortened the way `PyCmd`'s prompt does it: every directory above
//! the current one drops to its first character, and only if that is still too
//! wide does an ellipsis eat the middle. Two things survive every step, because
//! they are what make the text read as a location rather than as a command: the
//! anchor it starts from (the drive on Windows, `/` or `~` elsewhere) and the
//! separator it ends with.
//!
//! Everything here is pure, and the path style is PASSED IN rather than read off
//! `cfg!` - which is the only reason the Windows forms and the posix ones are
//! both covered by tests from whichever box happens to be running them.
//!
//! The window title is decided here too (`window_suffix`), because it is built
//! from the same parts and falls back to the tab's own label.

// Three dots rather than U+2026: a tab is drawn in the desktop interface font,
// and not every one of those carries the single-glyph ellipsis.
const ELLIPSIS: &str = "...";

/// Which spelling of a path we are shortening.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
	/// Backslashes, anchored on a drive (`C:\`) or a UNC share. No `~`: neither
	/// cmd nor PowerShell writes one, so a tab showing one would be inventing a
	/// spelling the shell itself does not use.
	Windows,
	/// Forward slashes, anchored on `/` - or on `~` when the path is inside the
	/// home directory, which is how every shell there prints it.
	Posix,
}

impl Style {
	/// The style of the platform this build runs on.
	pub fn native() -> Self {
		if cfg!(windows) {
			Self::Windows
		} else {
			Self::Posix
		}
	}

	fn sep(self) -> char {
		match self {
			Self::Windows => '\\',
			Self::Posix => '/',
		}
	}

	fn is_sep(self, c: char) -> bool {
		// A Windows path may arrive with either separator (a shell that reports
		// through OSC 7 sends a URL, which is all forward slashes).
		match self {
			Self::Windows => c == '\\' || c == '/',
			Self::Posix => c == '/',
		}
	}
}

/// What the tab has to say about the pane's command, if anything.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Task<'a> {
	/// A command is running right now.
	Running(&'a str),
	/// The shell is back at its prompt; this is the last thing it ran.
	Last(&'a str),
}

// Hand-picked short forms for the shell names we ship (the titles in shells.rs
// KNOWN, and the Windows extras beside it). A name the user has renamed is not
// on this list and takes the derived forms instead, which is the point of
// having both: "Cmd" is what a person writes for the Command Prompt, and
// nothing mechanical gets there from "Windows Cmd".
#[rustfmt::skip]
const SHORT_SHELLS: &[(&str, &str, &str)] = &[
	// full name              shorter      shortest
	("Windows Cmd",           "Cmd",       "C"),
	("Windows PowerShell 5",  "WinPS 5",   "P5"),
	("PowerShell 7",          "PS 7",      "P7"),
	("Nushell",               "Nu",        "Nu"),
	("Python 3",              "Py 3",      "P3"),
	("Node.js",               "Node",      "N"),
	("PyCmd",                 "PyCmd",     "PC"),
	("Bash (Git's mini)",     "Git Bash",  "GB"),
	("Bash (MSYS2's full)",   "MSYS2",     "M2"),
	("Bash (Cygwin)",         "Cygwin",    "Cy"),
	("Korn shell",            "Ksh",       "K"),
	("MirBSD Korn shell",     "Mksh",      "Mk"),
	("C shell",               "Csh",       "C"),
	("POSIX shell",           "Sh",        "S"),
	("IPython",               "IPy",       "IP"),
];

/// The shell's name, longest form first: as it reads, then shortened, then cut
/// to the least that still names it. A shipped name has its forms written out
/// (`SHORT_SHELLS`); anything else is derived from the words in the name.
pub fn shell_forms(name: &str) -> Vec<String> {
	let name = name.trim();
	if name.is_empty() {
		return Vec::new();
	}
	let mut forms = vec![name.to_string()];
	if let Some((_, short, tiny)) = SHORT_SHELLS
		.iter()
		.find(|(full, _, _)| full.eq_ignore_ascii_case(name))
	{
		push_shorter(&mut forms, (*short).to_string());
		push_shorter(&mut forms, (*tiny).to_string());
	} else {
		push_shorter(&mut forms, derived_short(name));
		push_shorter(&mut forms, derived_tiny(name));
	}
	forms
}

// The part of a name worth keeping. A WSL entry is named for its distribution
// ("WSL2; Ubuntu"), and a variant is named for what it varies from ("Zsh (no
// rc)") - the star is all that is left of that qualifier, so two tabs reading
// "Zsh" and "Zsh*" at least say that one of them is not the ordinary one.
fn core_name(name: &str) -> (&str, bool) {
	let head = name.split_once('(').map_or(name, |(head, _)| head).trim();
	let varied = head.len() != name.len();
	let core = head.rsplit_once(';').map_or(head, |(_, tail)| tail).trim();
	(if core.is_empty() { head } else { core }, varied)
}

// Short but still recognizable: kept whole while it is short enough to be worth
// keeping whole, else cut back to its initials.
fn derived_short(name: &str) -> String {
	let (core, varied) = core_name(name);
	let star = if varied { "*" } else { "" };
	if core.chars().count() <= 6 {
		format!("{core}{star}")
	} else {
		format!("{}{star}", initials(core, usize::MAX))
	}
}

fn derived_tiny(name: &str) -> String {
	let (core, _) = core_name(name);
	initials(core, 2)
}

// One letter per word, plus whatever digits the name ends in - a version number
// is most of what tells two shells of the same family apart.
fn initials(core: &str, words: usize) -> String {
	let digits: Vec<char> = core
		.chars()
		.rev()
		.take_while(char::is_ascii_digit)
		.collect();
	let letters: String = core
		.split(|c: char| !c.is_alphanumeric())
		.filter(|word| !word.is_empty() && !word.chars().all(|c| c.is_ascii_digit()))
		.take(words)
		.filter_map(|word| word.chars().next())
		.collect();
	let out: String = letters.chars().chain(digits.into_iter().rev()).collect();
	if out.is_empty() {
		core.chars().take(1).collect()
	} else {
		out
	}
}

/// What the tab says about the command, longest first: the program's name, then
/// truncations of it. WHICH command it is matters more than the tail of its
/// name, so `last:` stays put and the name is what gets cut.
pub fn task_forms(task: Option<Task>) -> Vec<String> {
	let (marker, program) = match task {
		Some(Task::Running(program)) => ("", program.trim()),
		Some(Task::Last(program)) => ("last: ", program.trim()),
		None => return Vec::new(),
	};
	if program.is_empty() {
		return Vec::new();
	}
	let mut forms = vec![format!("[{marker}{program}]")];
	let mut keep = program.chars().count();
	// Halve until what is kept says nothing. An ellipsis costs three columns, so
	// the first cuts of a short name are no shorter than the name itself -
	// push_shorter drops those, and a name too short to cut yields no rung.
	while keep > 3 {
		keep /= 2;
		let head: String = program.chars().take(keep).collect();
		push_shorter(&mut forms, format!("[{marker}{head}{ELLIPSIS}]"));
	}
	forms
}

/// What the terminal's own rights mean for its title.
#[derive(Clone, Copy, Default)]
pub struct Rights {
	/// The word the window title starts with, while the terminal holds them.
	pub say: Option<&'static str>,
	/// Whether a console here writes that same word into the first title it
	/// sends. Windows does; nothing on unix decorates a title.
	pub decorated: bool,
}

impl Rights {
	/// The word a console has already written into a title, so it is not said
	/// twice. Kept derived rather than stored: a word to take off that nothing
	/// would put back could only destroy somebody's own text.
	fn console_marker(self) -> Option<&'static str> {
		self.say.filter(|_| self.decorated)
	}
}

/// What a title set by the running program is worth showing, if anything.
///
/// A Windows console names a new window after the program it starts, so a shell
/// that sets no title of its own arrives carrying its own image path - measured
/// for cmd and for pwsh on two machines. That says less than the tab's own
/// label, so it is passed over.
fn program_says<'a>(title: &'a str, marker: Option<&str>) -> Option<&'a str> {
	let mut title = title.trim_start();
	// While elevated, a Windows console writes its own rights in front of the
	// first title it sends, which is the one naming the program it started -
	// measured over a pseudoconsole on two machines. Both halves say nothing, so
	// the rights come off here and the name is dropped below. Only the exact word
	// about to be put back is taken off, so a title that merely reads like one
	// survives. See design.md for what that leaves on a non-English Windows.
	if let Some(word) = marker {
		if let Some(rest) = title.strip_prefix(word).and_then(|r| r.strip_prefix(": ")) {
			title = rest;
		}
	}
	let title = title.trim();
	if title.is_empty() {
		return None;
	}
	// A console that names the program and then the command it is running: the
	// command is the half that says something. The name has to be a full path,
	// which is what a console writes there - a bare one would eat the file name
	// in vim's "build.bat - VIM".
	if let Some((head, rest)) = title.split_once(" - ") {
		let head = head.trim();
		if is_absolute(head) && is_program_name(head) {
			// The title was trimmed, so what follows the separator cannot be blank.
			return Some(rest.trim());
		}
	}
	(!is_program_name(title)).then_some(title)
}

// Nothing but the name of a program, or a path to one. Only a Windows console
// writes such a title, but the test is the same everywhere: an executable
// extension is the only handle there is, and a posix path does not carry one.
fn is_program_name(title: &str) -> bool {
	let Some(dot) = title.rfind('.') else {
		return false;
	};
	// `.com` is left out. Far more titles end in a hostname or a directory than
	// in one of the three DOS-era programs that still use that extension.
	if !["exe", "bat", "cmd"]
		.iter()
		.any(|known| title[dot + 1..].eq_ignore_ascii_case(known))
	{
		return false;
	}
	// A file name with a space in it could be anything, and so could a sentence
	// ending in one. Only a path may carry a space, and only above its last part.
	let last = &title[title.rfind(['\\', '/']).map_or(0, |at| at + 1)..];
	!last.contains(char::is_whitespace)
		&& (is_absolute(title) || !title.contains(char::is_whitespace))
}

// Rooted the way either platform spells it: a drive, a UNC share, or `/`.
fn is_absolute(path: &str) -> bool {
	path.starts_with('/')
		|| path.starts_with("\\\\")
		|| matches!(path.as_bytes(), [drive, b':', b'\\' | b'/', ..] if drive.is_ascii_alphabetic())
}

/// What the window title says after the application name. A title typed on the
/// tab wins; blanking that one on purpose lets the running program's own title
/// through, and with neither the tab's own label stands in.
///
/// A typed title is shown as typed - the tab shows it that way too - while a
/// program's is trimmed, since nobody chose its spacing, and has any marker a
/// console wrote into it taken off. `tab` is only asked for when it is needed,
/// since working it out is not free.
pub fn window_suffix(
	rights: Rights,
	typed: Option<&str>,
	program: Option<&str>,
	tab: impl FnOnce() -> String,
) -> Option<String> {
	let program = program.and_then(|title| program_says(title, rights.console_marker()));
	if let Some(typed) = typed {
		if !typed.trim().is_empty() {
			return Some(typed.to_string());
		}
		return program.map(str::to_string);
	}
	Some(program.map_or_else(tab, str::to_string))
}

/// The whole window title. A `--title` given on the command line is the answer
/// apart from the rights, since it is a request for exactly that string.
///
/// The rights the terminal is running with come first and no flag turns them
/// off, since the absence of the word has to mean something.
pub fn window_title(
	rights: Rights,
	custom: Option<&str>,
	prefix: &str,
	suffix: Option<&str>,
) -> String {
	let said = match (custom, suffix) {
		(Some(custom), _) => custom.to_string(),
		(None, Some(suffix)) => format!("{prefix} - {suffix}"),
		(None, None) => prefix.to_string(),
	};
	let Some(word) = rights.say else {
		return said;
	};
	match said.as_str() {
		// Nothing to say, so no dangling colon in front of it either.
		said if said.trim().is_empty() => word.to_string(),
		// A title typed by hand can already start with the word.
		said if said.strip_prefix(word).is_some_and(|r| r.starts_with(": ")) => said.to_string(),
		said => format!("{word}: {said}"),
	}
}

/// The tab's text, longest form first. The caller measures each against the
/// space it has and takes the first that fits; the last rung is the least that
/// still names the pane, so there is always something to draw.
pub fn label_forms(
	friendly: &str,
	task: Option<Task>,
	cwd: Option<&str>,
	home: Option<&str>,
	style: Style,
) -> Vec<String> {
	let shells = shell_forms(friendly);
	let tasks = task_forms(task);
	let paths = cwd
		.filter(|dir| !dir.trim().is_empty())
		.map(|dir| path_forms(dir, home, style))
		.unwrap_or_default();
	let full_name = shells.first().map_or("", String::as_str);
	// The last form is always the shortest; the middle rung only exists when
	// there are three of them, and a name with no middle rung keeps its own.
	let tiny_name = (shells.len() > 1).then(|| shells[shells.len() - 1].clone());
	let short_name = if shells.len() >= 3 {
		shells[1].as_str()
	} else {
		full_name
	};
	let first_task = tasks.first().map(String::as_str);
	let last_task = tasks.last().map(String::as_str);
	let first_path = paths.first().map(String::as_str);
	let last_path = paths.last().map(String::as_str);

	// The order the parts give way in. Each is only a candidate: push_shorter
	// keeps the ones that actually buy width, so a part with nothing left to
	// give (a one-directory path, a short program name) costs no rungs at all.
	let mut forms = vec![join(full_name, first_task, first_path)];
	push_shorter(&mut forms, join(short_name, first_task, first_path));
	for form in tasks.iter().skip(1) {
		push_shorter(&mut forms, join(short_name, Some(form), first_path));
	}
	for form in paths.iter().skip(1) {
		push_shorter(&mut forms, join(short_name, last_task, Some(form)));
	}
	push_shorter(&mut forms, join(short_name, None, last_path));
	push_shorter(&mut forms, join(short_name, None, None));
	if let Some(tiny) = tiny_name {
		push_shorter(&mut forms, tiny);
	}
	forms.retain(|form| !form.is_empty());
	if forms.is_empty() {
		forms.push(friendly.trim().to_string());
	}
	forms
}

// The parts, spelled the one way. The brackets already set a task apart from
// the path after it, so only a tab with nothing running needs the dash.
fn join(shell: &str, task: Option<&str>, path: Option<&str>) -> String {
	let mut out = shell.to_string();
	if let Some(task) = task {
		if !out.is_empty() {
			out.push(' ');
		}
		out.push_str(task);
	}
	if let Some(path) = path {
		if out.is_empty() {
			out.push_str(path);
		} else if task.is_some() {
			out.push(' ');
			out.push_str(path);
		} else {
			out.push_str(" - ");
			out.push_str(path);
		}
	}
	out
}

/// Every shortening of `raw`, longest first and each strictly shorter than the
/// one before it. The first is the path in full; the last is the anchor alone,
/// so there is always something to draw even in a tab too narrow for a name.
pub fn path_forms(raw: &str, home: Option<&str>, style: Style) -> Vec<String> {
	let sep = style.sep();
	let (anchor, parts) = split(raw, home, style);
	let join = |items: &[String]| {
		let mut out = anchor.clone();
		for item in items {
			out.push_str(item);
			out.push(sep);
		}
		out
	};
	if parts.is_empty() {
		return vec![anchor];
	}
	let mut forms = vec![join(&parts)];
	// PyCmd's abbreviation: everything ABOVE the current directory shrinks to its
	// initial, the current one stays whole - that last name is the whole point of
	// showing a path at all.
	let last = parts.len() - 1;
	let abbreviated: Vec<String> = parts
		.iter()
		.enumerate()
		.map(|(i, part)| {
			if i == last {
				part.clone()
			} else {
				initial(part)
			}
		})
		.collect();
	push_shorter(&mut forms, join(&abbreviated));
	// Then eat the middle, one initial at a time. An ellipsis costs four columns
	// where an initial costs two, so the early steps are LONGER than what they
	// replace - push_shorter drops those, which is what "only if it shortens
	// further" means in practice.
	for keep in (0..last).rev() {
		let mut items = abbreviated[..keep].to_vec();
		items.push(ELLIPSIS.to_string());
		items.push(parts[last].clone());
		push_shorter(&mut forms, join(&items));
	}
	push_shorter(&mut forms, format!("{anchor}{ELLIPSIS}{sep}"));
	forms
}

fn push_shorter(forms: &mut Vec<String>, candidate: String) {
	if forms
		.last()
		.is_some_and(|prev| candidate.chars().count() < prev.chars().count())
	{
		forms.push(candidate);
	}
}

// A hidden directory keeps the character after its dot, or every one of them
// abbreviates to the same thing.
fn initial(part: &str) -> String {
	let take = if part.starts_with('.') { 2 } else { 1 };
	part.chars().take(take).collect()
}

// Split a path into the anchor it must always show and the directories under it.
// An anchor already ends with its separator, so a path with no directories left
// still reads as one.
fn split(raw: &str, home: Option<&str>, style: Style) -> (String, Vec<String>) {
	let trimmed = raw.trim();
	let (anchor, rest) = match style {
		Style::Posix => posix_anchor(trimmed, home),
		Style::Windows => windows_anchor(trimmed),
	};
	let parts = rest
		.split(|c| style.is_sep(c))
		.filter(|part| !part.is_empty())
		.map(str::to_string)
		.collect();
	(anchor, parts)
}

fn posix_anchor<'a>(path: &'a str, home: Option<&str>) -> (String, &'a str) {
	if let Some(home) = home
		.map(|h| h.trim_end_matches('/'))
		.filter(|h| !h.is_empty())
	{
		if path == home {
			return ("~/".to_string(), "");
		}
		if let Some(rest) = path.strip_prefix(home) {
			if rest.starts_with('/') {
				return ("~/".to_string(), rest);
			}
		}
	}
	match path.strip_prefix('/') {
		Some(rest) => ("/".to_string(), rest),
		// Not absolute, so there is no anchor to promise - show it as it came.
		None => (String::new(), path),
	}
}

fn windows_anchor(path: &str) -> (String, &str) {
	let bytes = path.as_bytes();
	// A UNC path's share IS its root: \\server\share\... anchors on the share,
	// since neither half alone names a place anything can be opened.
	if bytes.len() >= 2 && (bytes[0] == b'\\' || bytes[0] == b'/') && bytes[0] == bytes[1] {
		let rest = &path[2..];
		let mut walked = 0;
		let mut seen = 0;
		for (i, c) in rest.char_indices() {
			if c == '\\' || c == '/' {
				seen += 1;
				if seen == 2 {
					walked = i;
					break;
				}
			}
			walked = i + c.len_utf8();
		}
		let (head, tail) = rest.split_at(walked.min(rest.len()));
		if seen >= 2 || !head.is_empty() {
			let mut anchor = format!("\\\\{head}");
			if !anchor.ends_with('\\') {
				anchor.push('\\');
			}
			return (anchor, tail);
		}
	}
	if bytes.len() >= 3 && bytes[1] == b':' && (bytes[2] == b'\\' || bytes[2] == b'/') {
		return (format!("{}:\\", &path[..1]), &path[3..]);
	}
	(String::new(), path)
}

/// What one tab asks the bar for, in pixels: the width its longest label wants,
/// and the least it can be given and still say anything - which is the width of
/// its shortest label, the one that is nothing but a short shell name.
#[derive(Clone, Copy, Debug, Default)]
pub struct Demand {
	pub natural: f32,
	pub floor: f32,
}

// The two percentages read as a range rather than as two independent numbers:
// the config clamps them on load, but the Settings dialog hands its edits
// straight over, and a maximum dragged below the regular width must not make
// this answer something absurd.
fn bounds(total: f32, regular_pct: f32, max_pct: f32) -> (f32, f32) {
	let total = total.max(0.0);
	let regular = total * (regular_pct.min(max_pct) / 100.0);
	let max = total * (max_pct.max(regular_pct) / 100.0);
	(regular, max)
}

/// How wide each tab on the page is.
///
/// The regular width is a TARGET, not a share: with room to spare every tab
/// sits at it, and the bar simply ends early rather than stretching a couple of
/// tabs across the window. A tab whose label wants more grows past it, up to
/// the maximum; a crowded bar pushes every tab back below it, down to its own
/// floor. Whatever room is left over after all that stays empty.
pub fn widths(total: f32, demands: &[Demand], regular_pct: f32, max_pct: f32) -> Vec<f32> {
	let (regular, max) = bounds(total, regular_pct, max_pct);
	let floors: Vec<f32> = demands.iter().map(|d| d.floor.clamp(0.0, max)).collect();
	let mut alloc = floors.clone();
	let mut spare = total.max(0.0) - floors.iter().sum::<f32>();
	// Every tab up to the regular width first, and only then the ones whose
	// labels want more - so a long path never takes room another tab needs to
	// reach its ordinary size.
	let target: Vec<f32> = floors.iter().map(|f| f.max(regular)).collect();
	spread(&mut alloc, &target, &mut spare);
	let want: Vec<f32> = demands
		.iter()
		.zip(&target)
		.map(|(d, t)| d.natural.clamp(*t, max))
		.collect();
	spread(&mut alloc, &want, &mut spare);
	alloc
}

// Hand out `spare` toward `upto`, in proportion to what each tab still asks
// for - so when there is not enough to go round, every tab lands the same
// fraction of the way there rather than the first few taking it all.
fn spread(alloc: &mut [f32], upto: &[f32], spare: &mut f32) {
	if *spare <= 0.0 {
		return;
	}
	let asked: f32 = alloc
		.iter()
		.zip(upto)
		.map(|(have, want)| (want - have).max(0.0))
		.sum();
	if asked <= 0.0 {
		return;
	}
	let share = (*spare / asked).min(1.0);
	for (have, want) in alloc.iter_mut().zip(upto) {
		let give = (want - *have).max(0.0) * share;
		*have += give;
		*spare -= give;
	}
}

/// How many tabs the bar can show at once, starting at `first`: as many as fit
/// side by side at their floors. Always at least one, however narrow the
/// window - a tab bar showing no tab is worse than one showing a clipped tab.
///
/// The floor is why the strip PAGES. A tab that yielded past the point where
/// its label says anything would make the setting meaningless; honoring it
/// means some tabs do not fit, and the strip shows a page of them instead.
pub fn tabs_that_fit(total: f32, floors: &[f32], first: usize) -> usize {
	let mut used = 0.0;
	let mut fit = 0;
	for floor in floors.iter().skip(first) {
		used += floor.max(0.0);
		if used > total && fit > 0 {
			break;
		}
		fit += 1;
	}
	fit.max(1)
}

/// Which tab the strip starts at, given where it WANTS to start: pulled back so
/// the last page is full rather than half-empty.
pub fn clamp_page(want: usize, floors: &[f32], total: f32) -> usize {
	let mut first = want.min(floors.len().saturating_sub(1));
	while first > 0 && floors[first - 1..].iter().sum::<f32>() <= total {
		first -= 1;
	}
	first
}

/// The page holding `active`, moving as little as possible from `want`.
///
/// Deliberately NOT applied on every read of the strip. A page that is forced
/// to hold the active tab at all times is a page the wheel can never leave -
/// and leaving it is the one thing the wheel is for. So this runs when the
/// active tab CHANGES, and browsing is free in between.
pub fn page_for(want: usize, active: usize, floors: &[f32], total: f32) -> usize {
	let mut first = clamp_page(want, floors, total);
	if active < first {
		first = active;
	} else if active >= first + tabs_that_fit(total, floors, first) {
		// Back up from the active tab for as long as the page still reaches it.
		first = active;
		while first > 0 && floors[first - 1..=active].iter().sum::<f32>() <= total {
			first -= 1;
		}
	}
	clamp_page(first, floors, total)
}

/// Where the `slot`'th tab on the page is drawn, measured from the bar's left
/// edge. Tabs are no longer one width apiece, so this is a running total.
pub fn slot_x(widths: &[f32], slot: usize) -> f32 {
	widths.iter().take(slot).sum()
}

/// Which slot on the page a pointer at `x` is over - the exact inverse of
/// `slot_x`, and the only thing a hit test may use. Drawing and hit-testing
/// reading two different answers is how a click lands on a tab other than the
/// one under the pointer.
pub fn slot_at_x(widths: &[f32], x: f32) -> Option<usize> {
	if x < 0.0 {
		return None;
	}
	let mut edge = 0.0;
	for (slot, w) in widths.iter().enumerate() {
		edge += w;
		if x < edge {
			return Some(slot);
		}
	}
	None
}

/// How long a tab has been open, at the coarseness a person reads at a glance.
/// Two units is the most that stays legible in a tip line, and the smaller of
/// the two is zero-padded so the width does not jump as it ticks.
// One value on a hover-tip line, quoted only where the eye needs the boundary:
// a value carrying a space or a quote character. Which quote is picked follows
// the config file's own habit - single ones around a value that already holds
// double quotes, so a Windows command line reads inside them rather than
// fighting them - and a value holding both is escaped instead.
pub fn tip_value(value: &str) -> String {
	let has_double = value.contains('"');
	let has_single = value.contains('\'');
	if !value.contains(' ') && !has_double && !has_single {
		return value.to_string();
	}
	if has_double && has_single {
		return format!("\"{}\"", value.replace('"', "\\\""));
	}
	if has_double {
		return format!("'{value}'");
	}
	format!("\"{value}\"")
}

// The tip's lines: every value starts at one column, so the pairs read down the
// left the way a table does. This is the whole reason the tip is drawn in the
// TERMINAL font rather than the interface one - padding with spaces aligns
// nothing in a proportional face. The KEY is padded, never the value, so a long
// path runs on to the right and the box grows for it instead of the column
// moving.
pub fn tip_lines(rows: &[(&str, String)]) -> Vec<String> {
	let key_w = rows
		.iter()
		.map(|(key, _)| key.chars().count())
		.max()
		.unwrap_or(0);
	rows.iter()
		.map(|(key, value)| {
			let pad = " ".repeat(key_w - key.chars().count());
			format!("{key}:{pad} {value}")
		})
		.collect()
}

pub fn elapsed(secs: u64) -> String {
	const MINUTE: u64 = 60;
	const HOUR: u64 = 60 * MINUTE;
	const DAY: u64 = 24 * HOUR;
	if secs < MINUTE {
		format!("{secs}s")
	} else if secs < HOUR {
		format!("{}m {:02}s", secs / MINUTE, secs % MINUTE)
	} else if secs < DAY {
		format!("{}h {:02}m", secs / HOUR, (secs % HOUR) / MINUTE)
	} else {
		format!("{}d {:02}h", secs / DAY, (secs % DAY) / HOUR)
	}
}

#[cfg(test)]
mod tests {
	use super::{
		Demand, Rights, Style, Task, clamp_page, elapsed, label_forms, page_for, path_forms,
		program_says, shell_forms, slot_at_x, slot_x, tabs_that_fit, task_forms, tip_lines,
		tip_value, widths, window_suffix, window_title,
	};

	// Most of what follows is the same question either way, so it is asked with
	// nothing to take off the front.
	fn says(title: &str) -> Option<&str> {
		program_says(title, None)
	}

	// The tip is a table, so a value carries quotes only where its own edges are
	// in doubt. Quoting everything would put them round every friendly shell name
	// and every clock reading in the box.
	#[test]
	fn a_tip_value_is_quoted_only_where_its_edges_are_in_doubt() {
		assert_eq!(tip_value("Bash"), "Bash");
		assert_eq!(tip_value("/bin/bash"), "/bin/bash");
		assert_eq!(tip_value("PowerShell 7"), "\"PowerShell 7\"");
		// a command line already full of double quotes reads inside single ones
		assert_eq!(
			tip_value(r#""C:\Program Files\pwsh.exe" -NoLogo"#),
			r#"'"C:\Program Files\pwsh.exe" -NoLogo'"#
		);
		// both kinds present: escape, rather than pick a quote that cannot close
		assert_eq!(tip_value(r#"say "it's""#), r#""say \"it's\"""#);
		// a lone apostrophe still needs a boundary drawn round it
		assert_eq!(tip_value("it's"), "\"it's\"");
	}

	// The keys are padded so the values line up; a value is never padded, so a
	// long path widens the box instead of moving the column.
	#[test]
	fn tip_keys_pad_so_every_value_starts_in_one_column() {
		let lines = tip_lines(&[
			("Shell name", "Bash".to_string()),
			("Shell command", "/bin/bash".to_string()),
			("Open", "1m 26s".to_string()),
		]);
		assert_eq!(lines[0], "Shell name:    Bash");
		assert_eq!(lines[1], "Shell command: /bin/bash");
		assert_eq!(lines[2], "Open:          1m 26s");
		// and as the property rather than three strings: one column for them all
		let value_col = |line: &str| {
			let colon = line.find(':').expect("a key");
			line[colon..]
				.find(|c: char| c != ':' && c != ' ')
				.map(|i| colon + i)
				.expect("a value")
		};
		let first = value_col(&lines[0]);
		for line in &lines {
			assert_eq!(value_col(line), first, "{line:?} is out of column");
		}
	}

	// The anchor and the trailing separator are what tell a reader this is a
	// place and not a command, so no shortening may cost either of them.
	#[test]
	fn every_form_keeps_its_anchor_and_its_trailing_slash() {
		let windows = path_forms(r"C:\Users\jim\data\prs\dev", None, Style::Windows);
		assert!(windows.len() > 2, "expected several forms: {windows:?}");
		for form in &windows {
			assert!(form.starts_with(r"C:\"), "lost the drive: {form}");
			assert!(form.ends_with('\\'), "lost the trailing slash: {form}");
		}
		let posix = path_forms("/home/jim/data/prs/dev", None, Style::Posix);
		for form in &posix {
			assert!(form.starts_with('/'), "lost the root: {form}");
			assert!(form.ends_with('/'), "lost the trailing slash: {form}");
		}
	}

	#[test]
	fn a_home_directory_reads_as_a_tilde_on_posix_only() {
		let home = Some("/home/jim");
		assert_eq!(path_forms("/home/jim", home, Style::Posix), vec!["~/"]);
		assert_eq!(path_forms("/home/jim/dev", home, Style::Posix)[0], "~/dev/");
		// A path that merely SHARES the prefix is not inside it.
		assert_eq!(
			path_forms("/home/jimbo/dev", home, Style::Posix)[0],
			"/home/jimbo/dev/"
		);
		// Windows keeps the drive - the shells there never print a tilde.
		assert_eq!(
			path_forms(r"C:\Users\jim\dev", Some(r"C:\Users\jim"), Style::Windows)[0],
			r"C:\Users\jim\dev\"
		);
	}

	#[test]
	fn directories_above_the_current_one_drop_to_their_initials() {
		let forms = path_forms(r"C:\Users\jim\data\prs\dev", None, Style::Windows);
		assert_eq!(forms[0], r"C:\Users\jim\data\prs\dev\");
		assert_eq!(forms[1], r"C:\U\j\d\p\dev\");
		// A hidden directory keeps the letter after its dot, or every one of them
		// would abbreviate to a bare dot.
		assert_eq!(
			path_forms("/home/jim/.config/silkterm/x", None, Style::Posix)[1],
			"/h/j/.c/s/x/"
		);
	}

	// The ellipsis is a LAST resort and only earns its place when it is shorter
	// than the initials it replaces - four columns against two apiece.
	#[test]
	fn an_ellipsis_only_appears_where_it_actually_shortens() {
		let forms = path_forms(r"C:\a\b\c\d\e\project", None, Style::Windows);
		let first_ellipsis = forms
			.iter()
			.position(|form| form.contains("..."))
			.expect("expected an ellipsis form");
		let before = &forms[first_ellipsis - 1];
		assert!(
			forms[first_ellipsis].chars().count() < before.chars().count(),
			"{} is not shorter than {before}",
			forms[first_ellipsis]
		);
		// Two directories cannot be beaten by an ellipsis, so none is offered
		// until the anchor-only form at the end.
		let shallow = path_forms(r"C:\a\project", None, Style::Windows);
		assert_eq!(shallow, vec![r"C:\a\project\", r"C:\...\"]);
	}

	#[test]
	fn the_forms_only_ever_get_shorter() {
		for raw in [
			r"C:\Users\jim\data\prs\dev\github.com\jim-collier\silkterm",
			r"C:\a\project",
			r"C:\",
		] {
			let forms = path_forms(raw, None, Style::Windows);
			for pair in forms.windows(2) {
				assert!(
					pair[1].chars().count() < pair[0].chars().count(),
					"{:?} is not shorter than {:?}",
					pair[1],
					pair[0]
				);
			}
		}
	}

	#[test]
	fn a_unc_share_anchors_on_the_share_not_the_server() {
		let forms = path_forms(r"\\box\share\team\docs", None, Style::Windows);
		assert_eq!(forms[0], r"\\box\share\team\docs\");
		for form in &forms {
			assert!(form.starts_with(r"\\box\share\"), "lost the share: {form}");
		}
	}

	#[test]
	fn a_path_reported_with_forward_slashes_still_reads_as_windows() {
		// OSC 7 carries a URL, so a Windows shell reporting through it sends
		// forward slashes for a path the tab must still draw with backslashes.
		assert_eq!(
			path_forms("C:/Users/jim/dev", None, Style::Windows)[0],
			r"C:\Users\jim\dev\"
		);
	}

	// A shipped name is shortened the way a person would write it; "Cmd" is not
	// something a rule gets to from "Windows Cmd".
	#[test]
	fn a_shipped_shell_name_has_hand_picked_short_forms() {
		assert_eq!(shell_forms("Windows Cmd"), ["Windows Cmd", "Cmd", "C"]);
		assert_eq!(shell_forms("PowerShell 7"), ["PowerShell 7", "PS 7", "P7"]);
		// a curated short form that is already the shortest yields two rungs
		assert_eq!(shell_forms("Nushell"), ["Nushell", "Nu"]);
	}

	// Rename a shell and the table no longer knows it, so the forms are derived.
	// A name short enough to keep whole yields one shorter rung, not two.
	#[test]
	fn a_renamed_shell_falls_back_to_derived_forms() {
		assert_eq!(shell_forms("Bash"), ["Bash", "B"]);
		assert_eq!(
			shell_forms("My Build Shell"),
			["My Build Shell", "MBS", "MB"]
		);
		// the version digits survive, since they are what tell a family apart -
		// and a name whose two derived forms agree yields only one rung
		assert_eq!(shell_forms("Fancy Shell 9"), ["Fancy Shell 9", "FS9"]);
		// a distribution is named for itself, and a variant keeps a mark saying
		// it is not the ordinary one
		assert_eq!(shell_forms("WSL2; Ubuntu"), ["WSL2; Ubuntu", "Ubuntu", "U"]);
		assert_eq!(shell_forms("Zsh (no rc)"), ["Zsh (no rc)", "Zsh*", "Z"]);
		assert!(shell_forms("").is_empty());
	}

	// Which command is running matters more than the tail of its name, so the
	// marker stays and the name is what gets cut.
	#[test]
	fn a_task_is_cut_from_its_tail_and_keeps_its_marker() {
		assert_eq!(
			task_forms(Some(Task::Last("docker-compose"))),
			[
				"[last: docker-compose]",
				"[last: docker-...]",
				"[last: doc...]"
			]
		);
		// a name too short to cut offers nothing to cut: three dots cost more
		// than the letters they replace
		assert_eq!(task_forms(Some(Task::Running("cargo"))), ["[cargo]"]);
		assert!(task_forms(None).is_empty());
	}

	// The whole ladder, in the order the parts give way: the name shortens, then
	// the path abbreviates, then the task goes, then the path, and the last rung
	// is the name alone at its shortest.
	#[test]
	fn a_tab_says_the_shell_the_task_and_the_path_and_gives_them_up_in_order() {
		let forms = label_forms(
			"PowerShell 7",
			Some(Task::Running("cargo")),
			Some(r"C:\Users\jim\dev"),
			None,
			Style::Windows,
		);
		assert_eq!(
			forms,
			[
				r"PowerShell 7 [cargo] C:\Users\jim\dev\",
				r"PS 7 [cargo] C:\Users\jim\dev\",
				r"PS 7 [cargo] C:\U\j\dev\",
				r"PS 7 [cargo] C:\...\",
				r"PS 7 - C:\...\",
				"PS 7",
				"P7",
			]
		);
	}

	// An idle tab keeps the dash, since there is no bracket to separate the name
	// from the path.
	#[test]
	fn a_tab_with_nothing_running_still_says_where_it_is() {
		let idle = label_forms(
			"PowerShell 7",
			None,
			Some(r"C:\Users\jim\dev"),
			None,
			Style::Windows,
		);
		assert_eq!(idle[0], r"PowerShell 7 - C:\Users\jim\dev\");
		assert_eq!(idle.last().map(String::as_str), Some("P7"));
		// Nothing to say about a directory either: the shell's name stands alone.
		assert_eq!(
			label_forms("bash", None, None, None, Style::Posix),
			["bash", "b"]
		);
	}

	#[test]
	fn the_forms_of_a_label_only_ever_get_shorter() {
		let forms = label_forms(
			"Bash (MSYS2's full)",
			Some(Task::Last("docker-compose")),
			Some("/home/jim/data/prs/dev/silkterm"),
			Some("/home/jim"),
			Style::Posix,
		);
		assert!(forms.len() > 5, "expected a full ladder: {forms:?}");
		for pair in forms.windows(2) {
			assert!(
				pair[1].chars().count() < pair[0].chars().count(),
				"{:?} is not shorter than {:?}",
				pair[1],
				pair[0]
			);
		}
	}

	// The regular width is a target, not a share: three tabs on a wide bar sit
	// at it and leave the rest of the bar empty.
	#[test]
	fn a_tab_with_nothing_pressing_it_sits_at_the_regular_width() {
		let demands = vec![
			Demand {
				natural: 100.0,
				floor: 40.0
			};
			3
		];
		assert_eq!(widths(1000.0, &demands, 10.0, 100.0), [100.0, 100.0, 100.0]);
	}

	// A label that wants more gets more, and only after every other tab has its
	// regular width - a long path may not cost another tab its ordinary size.
	#[test]
	fn a_long_label_grows_its_own_tab_and_no_other() {
		let demands = [
			Demand {
				natural: 400.0,
				floor: 40.0,
			},
			Demand {
				natural: 100.0,
				floor: 40.0,
			},
		];
		assert_eq!(widths(1000.0, &demands, 10.0, 100.0), [400.0, 100.0]);
		// the maximum still caps it
		assert_eq!(widths(1000.0, &demands, 10.0, 25.0), [250.0, 100.0]);
	}

	// A crowded bar pushes every tab back below the regular width by the same
	// fraction, down to the floor - and no further, which is why the strip pages.
	#[test]
	fn a_crowded_bar_shrinks_every_tab_alike_and_stops_at_the_floor() {
		let demands = vec![
			Demand {
				natural: 100.0,
				floor: 40.0
			};
			12
		];
		let w = widths(600.0, &demands, 10.0, 100.0);
		for one in &w {
			assert!((one - 50.0).abs() < 0.01, "{w:?} is not an even share");
		}
		let floors = vec![40.0; 20];
		assert_eq!(tabs_that_fit(600.0, &floors, 0), 15);
		// however narrow the bar, it shows a tab
		assert_eq!(tabs_that_fit(10.0, &floors, 0), 1);
		assert_eq!(tabs_that_fit(0.0, &[], 0), 1);
	}

	// Switching tabs has to bring the new one onto the page, or Ctrl+Tab could
	// never reach the far end.
	#[test]
	fn switching_tabs_brings_the_new_one_onto_the_page() {
		let floors = vec![100.0; 12];
		let total = 400.0; // four tabs to a page
		for active in 0..12 {
			for want in 0..12 {
				let first = page_for(want, active, &floors, total);
				let fit = tabs_that_fit(total, &floors, first);
				assert!(
					(first..first + fit).contains(&active),
					"active {active} off the page {first}..{} (wanted {want})",
					first + fit
				);
				assert!(first + fit <= 12, "page runs past the last tab");
			}
		}
		// Everything fits: there is only ever one page, starting at the first tab.
		assert_eq!(page_for(3, 2, &[100.0; 4], 800.0), 0);
	}

	// ...but browsing must not be yanked back. A strip that always held the
	// active tab could never be paged away from it, which is the whole point of
	// being able to page at all.
	#[test]
	fn the_page_can_be_moved_away_from_the_active_tab() {
		let floors = vec![100.0; 12];
		assert_eq!(clamp_page(5, &floors, 400.0), 5);
		assert_eq!(clamp_page(0, &floors, 400.0), 0);
		// It still cannot run off the end, nor show a half-empty last page.
		assert_eq!(clamp_page(11, &floors, 400.0), 8);
		assert_eq!(clamp_page(3, &[100.0; 4], 800.0), 0);
	}

	// Drawing and hit-testing have to be the same answer read two ways, now that
	// the tabs on a page are no longer one width apiece - otherwise a click
	// selects a tab other than the one under the pointer.
	#[test]
	fn a_click_lands_on_the_tab_it_is_over() {
		let widths = [91.0, 140.0, 60.0, 200.0];
		let mut edge = 0.0;
		for (slot, w) in widths.iter().enumerate() {
			assert!((slot_x(&widths, slot) - edge).abs() < 0.01);
			for probe in [edge + 0.5, edge + w / 2.0, edge + w - 0.5] {
				assert_eq!(slot_at_x(&widths, probe), Some(slot), "x {probe}");
			}
			edge += w;
		}
		// Past the last drawn tab is the bare bar, not the tab before it.
		assert_eq!(slot_at_x(&widths, edge), None);
		assert_eq!(slot_at_x(&widths, edge + 50.0), None);
		assert_eq!(slot_at_x(&widths, -1.0), None);
		assert_eq!(slot_at_x(&[], 10.0), None);
	}

	#[test]
	fn the_two_percentages_are_read_as_a_range_either_way_round() {
		let demands = vec![
			Demand {
				natural: 500.0,
				floor: 40.0
			};
			3
		];
		assert_eq!(
			widths(1000.0, &demands, 30.0, 10.0),
			widths(1000.0, &demands, 10.0, 30.0)
		);
	}

	#[test]
	fn a_bar_with_no_width_still_answers() {
		let demands = [Demand {
			natural: 100.0,
			floor: 40.0,
		}];
		assert!(widths(0.0, &demands, 12.0, 26.0)[0] >= 0.0);
		assert!(widths(-5.0, &demands, 12.0, 26.0)[0] >= 0.0);
		assert!(widths(100.0, &[], 12.0, 26.0).is_empty());
	}

	#[test]
	fn elapsed_time_reads_at_two_units() {
		assert_eq!(elapsed(0), "0s");
		assert_eq!(elapsed(59), "59s");
		assert_eq!(elapsed(60), "1m 00s");
		assert_eq!(elapsed(3599), "59m 59s");
		assert_eq!(elapsed(3600), "1h 00m");
		assert_eq!(elapsed(86_399), "23h 59m");
		assert_eq!(elapsed(86_400), "1d 00h");
		assert_eq!(elapsed(200_000), "2d 07h");
	}

	#[test]
	fn a_console_title_that_only_names_a_program_is_dropped() {
		assert_eq!(
			says("C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"),
			None
		);
		// A path with a space in it is still a path.
		assert_eq!(says("C:\\Program Files\\PowerShell\\7\\pwsh.exe"), None);
		assert_eq!(says("powershell.exe"), None);
		assert_eq!(says("  CMD.EXE  "), None);
		assert_eq!(says("\\\\my server\\share\\tools\\run.bat"), None);
		// A lower-case drive spelled with forward slashes is the same path.
		assert_eq!(says("c:/windows/system32/cmd.exe"), None);
		assert_eq!(says(""), None);
		assert_eq!(says("   "), None);
	}

	#[test]
	fn a_title_that_says_something_survives() {
		let kept = |title| assert_eq!(says(title), Some(title));
		kept("jim@box: ~/src/silkterm");
		kept("C:\\Users\\jim\\src");
		kept("Building foo.exe");
		kept("nano");
		kept("MINGW64:/c/Users/jim");
		// A file name is only the answer when nothing is wrapped around it.
		kept("Running tools\\build.bat");
		kept("C:\\dev\\my build.cmd");
		assert_eq!(says(" vim foo.rs "), Some("vim foo.rs"));
	}

	#[test]
	fn a_hostname_or_a_directory_is_not_a_program() {
		// `.com` is a top-level domain far more often than it is a program, and
		// this tree is itself under a directory ending in one.
		let kept = |title| assert_eq!(says(title), Some(title));
		kept("jim@web01.example.com");
		kept("~/src/github.com");
		kept("C:\\www\\example.com");
	}

	#[test]
	fn a_console_that_names_the_command_it_is_running_keeps_the_command() {
		assert_eq!(
			says("C:\\WINDOWS\\system32\\cmd.exe - ping 8.8.8.8"),
			Some("ping 8.8.8.8")
		);
		// Spacing around the dash is the console's, not anybody's choice.
		assert_eq!(
			says("C:\\WINDOWS\\system32\\cmd.exe  -  build.bat "),
			Some("build.bat")
		);
		// A command given as a full path is shown as one. It is still what is
		// running, which is the thing the title is for.
		assert_eq!(
			says("C:\\WINDOWS\\system32\\cmd.exe - C:\\tools\\build.exe"),
			Some("C:\\tools\\build.exe")
		);
		// Not a program on the left, so both halves stand.
		assert_eq!(says("foo - bar"), Some("foo - bar"));
	}

	#[test]
	fn an_editor_naming_the_file_it_has_open_keeps_the_file() {
		// vim's default title is "<file> - VIM", and a bare file name on the left
		// must not be read as the program.
		let kept = |title| assert_eq!(says(title), Some(title));
		kept("build.bat - VIM");
		kept("setup.exe - NVIM");
		kept("run.cmd (~/src) - VIM");
	}

	#[test]
	fn either_spelling_of_a_program_path_reads_the_same() {
		// Only a Windows console writes one of these, but the test does not have to
		// know that. A posix path names no extension, so nothing there matches in
		// the first place.
		assert_eq!(says("/home/jim/games/setup.exe"), None);
		assert_eq!(says("/home/jim/my games/setup.exe"), None);
		assert_eq!(says("/usr/bin/bash"), Some("/usr/bin/bash"));
		assert_eq!(
			says("/opt/powershell/7/pwsh"),
			Some("/opt/powershell/7/pwsh")
		);
	}

	#[test]
	fn an_extension_has_to_be_the_whole_of_the_last_part() {
		assert_eq!(says("foo."), Some("foo."));
		assert_eq!(says("a.EXE."), Some("a.EXE."));
		assert_eq!(says(".exe"), None);
		// Multi-byte before the dot, to pin the slice against a char boundary.
		assert_eq!(says("caf\u{e9}.exe"), None);
		assert_eq!(says("caf\u{e9}"), Some("caf\u{e9}"));
	}

	#[test]
	fn the_window_title_takes_the_typed_name_then_the_program_then_the_tab() {
		let suffix = |typed, program| {
			window_suffix(Rights::default(), typed, program, || {
				"Bash - ~/src".to_string()
			})
		};
		assert_eq!(
			suffix(Some("build"), Some("vim foo.rs")).as_deref(),
			Some("build")
		);
		assert_eq!(
			suffix(None, Some("vim foo.rs")).as_deref(),
			Some("vim foo.rs")
		);
		assert_eq!(suffix(None, None).as_deref(), Some("Bash - ~/src"));
		// A tab blanked on purpose lets the program through, and says nothing when
		// the program has nothing to say either.
		assert_eq!(
			suffix(Some(" "), Some("vim foo.rs")).as_deref(),
			Some("vim foo.rs")
		);
		assert_eq!(suffix(Some(""), None), None);
		// A typed title is shown as typed; a program's is trimmed.
		assert_eq!(suffix(Some(" build "), None).as_deref(), Some(" build "));
	}

	#[test]
	fn the_tab_label_is_only_worked_out_when_it_is_needed() {
		let asked = std::cell::Cell::new(0);
		let suffix = |typed, program: Option<&str>| {
			window_suffix(Rights::default(), typed, program, || {
				asked.set(asked.get() + 1);
				"Bash - ~/src".to_string()
			})
		};
		suffix(Some("build"), Some("vim foo.rs"));
		suffix(None, Some("vim foo.rs"));
		suffix(Some(""), None);
		assert_eq!(asked.get(), 0);
		suffix(None, None);
		assert_eq!(asked.get(), 1);
	}

	#[test]
	fn a_program_naming_only_itself_falls_through_to_the_tab() {
		let exe = Some("C:\\WINDOWS\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
		let suffix = |typed, program| {
			window_suffix(Rights::default(), typed, program, || {
				"Windows PowerShell".to_string()
			})
		};
		assert_eq!(suffix(None, exe).as_deref(), Some("Windows PowerShell"));
		assert_eq!(suffix(Some(""), exe), None);
	}

	#[test]
	fn a_title_on_the_command_line_is_the_whole_answer() {
		assert_eq!(
			window_title(Rights::default(), Some("mine"), "SilkTerm", Some("Bash")),
			"mine"
		);
		// Even against nothing to say, and even when it is empty.
		assert_eq!(
			window_title(Rights::default(), Some("mine"), "SilkTerm", None),
			"mine"
		);
		assert_eq!(
			window_title(Rights::default(), Some(""), "SilkTerm", Some("Bash")),
			""
		);
		assert_eq!(
			window_title(Rights::default(), None, "SilkTerm", Some("Bash")),
			"SilkTerm - Bash"
		);
		assert_eq!(
			window_title(Rights::default(), None, "SilkTerm", None),
			"SilkTerm"
		);
	}

	#[test]
	fn nothing_the_title_rules_answer_is_blank() {
		// A blank answer would draw "SilkTerm - " with nothing after it.
		for title in [
			" - ",
			"C:\\WINDOWS\\system32\\cmd.exe - ",
			"   ",
			"x.exe - \t ",
			"Administrator: ",
			"Administrator: C:\\WINDOWS\\system32\\cmd.exe - ",
		] {
			for strip in [None, Some("Administrator")] {
				let said = program_says(title, strip);
				assert!(said.is_none_or(|said| !said.trim().is_empty()), "{title:?}");
			}
		}
	}

	// No console writes the separator with nothing after it, so such a title is
	// not taken apart. Recorded because the rule above only asks whether the
	// answer is blank, and this is what it is instead.
	#[test]
	fn a_title_ending_on_the_separator_is_shown_whole() {
		assert_eq!(
			says("C:\\WINDOWS\\system32\\cmd.exe - "),
			Some("C:\\WINDOWS\\system32\\cmd.exe -")
		);
		assert_eq!(
			program_says(
				"Administrator: C:\\WINDOWS\\system32\\cmd.exe - ",
				Some("Administrator")
			),
			Some("C:\\WINDOWS\\system32\\cmd.exe -")
		);
	}

	// Measured on both Windows machines: an elevated console puts its own rights
	// in front of every title it sends over a pseudoconsole, so the marker arrives
	// glued to the path the program-name rule was written to drop.
	#[test]
	fn an_elevated_console_marker_is_not_repeated() {
		let elevated = |title| program_says(title, Some("Administrator"));
		assert_eq!(
			elevated(
				"Administrator: C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe"
			),
			None
		);
		assert_eq!(elevated("Administrator: vim foo.rs"), Some("vim foo.rs"));
		assert_eq!(
			elevated("Administrator: C:\\WINDOWS\\system32\\cmd.exe - ping 8.8.8.8"),
			Some("ping 8.8.8.8")
		);
		assert_eq!(elevated("Administrator: "), None);
		// A path with a space in it takes the other half of the program-name test.
		assert_eq!(
			elevated("Administrator: C:\\Program Files\\PowerShell\\7\\pwsh.exe"),
			None
		);
	}

	// The word is matched exactly, and only where a console would have written
	// it. Anything else is somebody's real title.
	#[test]
	fn a_marker_the_console_did_not_write_survives() {
		let elevated = |title| program_says(title, Some("Administrator"));
		assert_eq!(
			elevated("Administrator:Backup"),
			Some("Administrator:Backup")
		);
		assert_eq!(elevated("Administrator"), Some("Administrator"));
		assert_eq!(
			elevated("ADMINISTRATOR: vim foo.rs"),
			Some("ADMINISTRATOR: vim foo.rs")
		);
		// A console that speaks another language writes another word, so the
		// title is shown as it arrived. See design.md.
		assert_eq!(
			elevated("Administrador: C:\\Windows\\System32\\cmd.exe"),
			Some("Administrador: C:\\Windows\\System32\\cmd.exe")
		);
	}

	// Nothing on unix writes a marker into a title, so running as root must not
	// start taking one off - there would be nothing to remove but real text.
	#[test]
	fn root_alone_does_not_strip_anything() {
		let rights = Rights {
			say: Some("Root"),
			decorated: false,
		};
		assert_eq!(
			window_suffix(rights, None, Some("Root: kernel notes"), || "Bash"
				.to_string()),
			Some("Root: kernel notes".to_string())
		);
	}

	// Only the word that is about to be put back is taken off. Anything else is
	// somebody's real title and has to survive.
	#[test]
	fn a_marker_that_was_never_added_is_left_alone() {
		assert_eq!(
			program_says("Administrator: setup.log", Some("Root")),
			Some("Administrator: setup.log")
		);
		assert_eq!(
			program_says("Root: notes", Some("Administrator")),
			Some("Root: notes")
		);
		assert_eq!(
			program_says("Administrators: three of them", Some("Administrator")),
			Some("Administrators: three of them")
		);
	}

	// A word to take off that nothing would put back could only lose text, so the
	// two are one answer rather than two fields to keep in step.
	#[test]
	fn only_a_decorated_console_leaves_a_word_to_take_off() {
		let marker = |say, decorated| Rights { say, decorated }.console_marker();
		assert_eq!(marker(Some("Root"), false), None);
		assert_eq!(marker(Some("Administrator"), true), Some("Administrator"));
		// Nothing to say, so nothing to take off either.
		assert_eq!(marker(None, true), None);
	}

	#[test]
	fn a_privileged_window_says_so_before_anything_else() {
		let root = Rights {
			say: Some("Root"),
			decorated: false,
		};
		assert_eq!(
			window_title(root, None, "SilkTerm", Some("Bash")),
			"Root: SilkTerm - Bash"
		);
		assert_eq!(
			window_title(
				Rights {
					say: Some("Administrator"),
					decorated: true
				},
				None,
				"SilkTerm",
				None
			),
			"Administrator: SilkTerm"
		);
		// A --title is otherwise the whole answer, but it does not get to drop this.
		assert_eq!(
			window_title(root, Some("deploy"), "SilkTerm", Some("Bash")),
			"Root: deploy"
		);
		// A title that already starts with the word is not given a second one,
		// and an empty one is not given a dangling colon either.
		assert_eq!(
			window_title(root, Some("Root: deploy"), "SilkTerm", None),
			"Root: deploy"
		);
		assert_eq!(window_title(root, Some(""), "SilkTerm", None), "Root");
		assert_eq!(window_title(root, Some("   "), "SilkTerm", None), "Root");
	}
}
