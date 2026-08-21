//! What a tab says, and how it is shortened to fit.
//!
//! A tab reads "<shell> [<task>]" while a command runs, "<shell> [last: <task>]"
//! once it finishes, and "<shell> - <path>" when the shell has never run
//! anything - a fresh tab has nothing to report but where it is.
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

/// The tab's text, longest form first. The caller measures each against the
/// space it has and takes the first that fits - only a path has more than one
/// form, since only a path can be shortened without losing which pane it is.
pub fn label_forms(
	friendly: &str,
	task: Option<Task>,
	cwd: Option<&str>,
	home: Option<&str>,
	style: Style,
) -> Vec<String> {
	let friendly = friendly.trim();
	match task {
		Some(Task::Running(program)) => vec![join_label(friendly, &format!("[{program}]"))],
		Some(Task::Last(program)) => vec![join_label(friendly, &format!("[last: {program}]"))],
		None => match cwd.filter(|dir| !dir.trim().is_empty()) {
			Some(dir) => path_forms(dir, home, style)
				.into_iter()
				.map(|form| join_label(friendly, &format!("- {form}")))
				.collect(),
			None => vec![friendly.to_string()],
		},
	}
}

// An empty shell name would otherwise leave the tail hanging off a separator.
fn join_label(friendly: &str, tail: &str) -> String {
	if friendly.is_empty() {
		tail.trim_start_matches("- ").to_string()
	} else {
		format!("{friendly} {tail}")
	}
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

/// How wide one tab is, given the bar's width and how many tabs share it.
///
/// Tabs divide the bar evenly, bounded by the two percentages. The maximum
/// stops a lone tab stretching across a whole window, which reads as no tab bar
/// at all; the minimum stops a tab shrinking past the point where its text says
/// anything.
///
/// The minimum is why the strip PAGES (see `tabs_that_fit`). Under an even
/// share alone a floor cannot ever bind - "the share is below the floor" and
/// "the tabs no longer fit at the floor" are the same condition, `tabs >
/// 100/min_pct` - so a minimum that yields to keep everything on the bar is a
/// setting that provably does nothing. Honouring it means some tabs do not fit,
/// and the strip shows a page of them instead.
pub fn tab_width(total: f32, tabs: usize, min_pct: f32, max_pct: f32) -> f32 {
	let total = total.max(0.0);
	let tabs = tabs.max(1) as f32;
	// Taken as a range rather than as two independent numbers: the config clamps
	// them on load, but the Settings dialog hands its edits straight over, and a
	// max dragged below the min must not make this answer something absurd.
	let max_w = total * (max_pct.max(min_pct) / 100.0);
	let min_w = total * (min_pct.min(max_pct) / 100.0);
	(total / tabs).clamp(min_w.min(max_w), max_w)
}

/// How many whole tabs of `tab_w` the bar can show at once - the page size.
/// Always at least one, however narrow the window: a tab bar showing no tab is
/// worse than one showing a clipped tab.
pub fn tabs_that_fit(total: f32, tab_w: f32) -> usize {
	if tab_w <= 0.0 {
		return 1;
	}
	((total / tab_w).floor() as usize).max(1)
}

/// Which tab the strip starts at, given where it WANTS to start: pulled back so
/// the last page is full rather than half-empty.
pub fn clamp_page(want: usize, tabs: usize, fit: usize) -> usize {
	want.min(tabs.saturating_sub(fit))
}

/// The page holding `active`, moving as little as possible from `want`.
///
/// Deliberately NOT applied on every read of the strip. A page that is forced
/// to hold the active tab at all times is a page the wheel can never leave -
/// and leaving it is the one thing the wheel is for. So this runs when the
/// active tab CHANGES, and browsing is free in between.
pub fn page_for(want: usize, active: usize, tabs: usize, fit: usize) -> usize {
	let mut first = clamp_page(want, tabs, fit);
	if active < first {
		first = active;
	} else if active >= first + fit {
		first = active + 1 - fit;
	}
	clamp_page(first, tabs, fit)
}

/// Where tab `i` is drawn on the strip, or None when it is on another page.
pub fn slot_x(i: usize, tab_w: f32, first: usize, shown: usize) -> Option<f32> {
	(i >= first && i < first + shown).then(|| (i - first) as f32 * tab_w)
}

/// Which tab a pointer at `x` is over - the exact inverse of `slot_x`, and the
/// only thing a hit test may use. Drawing and hit-testing reading two different
/// answers is how a click lands on a tab other than the one under the pointer.
pub fn tab_at_x(x: f32, tab_w: f32, first: usize, shown: usize) -> Option<usize> {
	if tab_w <= 0.0 || x < 0.0 {
		return None;
	}
	let slot = (x / tab_w).floor() as usize;
	(slot < shown).then_some(first + slot)
}

/// How long a tab has been open, at the coarseness a person reads at a glance.
/// Two units is the most that stays legible in a tip line, and the smaller of
/// the two is zero-padded so the width does not jump as it ticks.
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
		Style, Task, clamp_page, elapsed, label_forms, page_for, path_forms, slot_x, tab_at_x,
		tab_width, tabs_that_fit,
	};

	// The anchor and the trailing separator are what tell a reader this is a
	// place and not a command, so no shortening may cost either of them.
	#[test]
	fn every_form_keeps_its_anchor_and_its_trailing_slash() {
		let windows = path_forms(r"C:\Users\collierjr\data\prs\dev", None, Style::Windows);
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
		let forms = path_forms(r"C:\Users\collierjr\data\prs\dev", None, Style::Windows);
		assert_eq!(forms[0], r"C:\Users\collierjr\data\prs\dev\");
		assert_eq!(forms[1], r"C:\U\c\d\p\dev\");
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
			r"C:\Users\collierjr\data\prs\dev\github.com\jim-collier\silkterm",
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

	#[test]
	fn a_running_command_wins_the_tab_and_a_path_only_fills_an_idle_one() {
		let running = label_forms(
			"PowerShell 7",
			Some(Task::Running("cargo")),
			Some(r"C:\dev"),
			None,
			Style::Windows,
		);
		assert_eq!(running, vec!["PowerShell 7 [cargo]"]);
		let done = label_forms(
			"PowerShell 7",
			Some(Task::Last("cargo")),
			Some(r"C:\dev"),
			None,
			Style::Windows,
		);
		assert_eq!(done, vec!["PowerShell 7 [last: cargo]"]);
		let idle = label_forms(
			"PowerShell 7",
			None,
			Some(r"C:\Users\jim\dev"),
			None,
			Style::Windows,
		);
		assert_eq!(idle[0], r"PowerShell 7 - C:\Users\jim\dev\");
		assert!(idle.len() > 1, "an idle tab needs shorter forms: {idle:?}");
		// Nothing to say about a directory either: the shell's name stands alone.
		assert_eq!(
			label_forms("bash", None, None, None, Style::Posix),
			vec!["bash"]
		);
	}

	#[test]
	fn one_tab_is_capped_and_many_tabs_share_what_is_left() {
		// A lone tab takes the maximum, not the window.
		assert!((tab_width(1000.0, 1, 10.0, 26.0) - 260.0).abs() < 0.01);
		// Four still fit inside the cap, so they sit at it.
		assert!((tab_width(1000.0, 4, 10.0, 26.0) - 250.0).abs() < 0.01);
		// Six divide evenly, between the two bounds.
		assert!((tab_width(1000.0, 6, 10.0, 26.0) - 166.67).abs() < 0.01);
	}

	// The whole reason the strip pages. A floor that yielded to keep every tab on
	// the bar could never bind at all - the two conditions are the same one - so
	// it has to hold, and the tabs past the edge become a page.
	#[test]
	fn a_tab_never_shrinks_past_the_minimum() {
		for tabs in 1..40 {
			let w = tab_width(1000.0, tabs, 20.0, 30.0);
			assert!(w >= 199.9, "{tabs} tabs shrank to {w}, under the minimum");
			assert!(w <= 300.1, "{tabs} tabs breached the maximum at {w}");
		}
		assert_eq!(tabs_that_fit(1000.0, 200.0), 5);
		assert_eq!(tabs_that_fit(1000.0, 300.0), 3);
		// However narrow the window, the strip shows a tab.
		assert_eq!(tabs_that_fit(10.0, 300.0), 1);
		assert_eq!(tabs_that_fit(0.0, 0.0), 1);
	}

	// Switching tabs has to bring the new one onto the page, or Ctrl+Tab could
	// never reach the far end.
	#[test]
	fn switching_tabs_brings_the_new_one_onto_the_page() {
		for active in 0..12 {
			for want in 0..12 {
				let first = page_for(want, active, 12, 4);
				assert!(
					(first..first + 4).contains(&active),
					"active {active} off the page {first}..{} (wanted {want})",
					first + 4
				);
				assert!(first + 4 <= 12, "page runs past the last tab");
			}
		}
		// Everything fits: there is only ever one page, starting at the first tab.
		assert_eq!(page_for(3, 2, 4, 8), 0);
	}

	// ...but browsing must not be yanked back. A strip that always held the
	// active tab could never be paged away from it, which is the whole point of
	// being able to page at all.
	#[test]
	fn the_page_can_be_moved_away_from_the_active_tab() {
		assert_eq!(clamp_page(5, 12, 4), 5);
		assert_eq!(clamp_page(0, 12, 4), 0);
		// It still cannot run off the end, nor show a half-empty last page.
		assert_eq!(clamp_page(11, 12, 4), 8);
		assert_eq!(clamp_page(3, 4, 8), 0);
	}

	#[test]
	fn the_two_percentages_are_read_as_a_range_either_way_round() {
		assert!((tab_width(1000.0, 3, 30.0, 10.0) - tab_width(1000.0, 3, 10.0, 30.0)).abs() < 0.01);
	}

	// Drawing and hit-testing have to be the same answer read two ways, on every
	// page - otherwise a click selects a tab other than the one under the pointer,
	// which is the way a paging strip breaks.
	#[test]
	fn a_click_lands_on_the_tab_it_is_over() {
		let (tab_w, shown) = (91.0, 12);
		for first in [0, 1, 4] {
			for i in first..first + shown {
				let x = slot_x(i, tab_w, first, shown).expect("on the page");
				for probe in [x + 0.5, x + tab_w / 2.0, x + tab_w - 0.5] {
					assert_eq!(
						tab_at_x(probe, tab_w, first, shown),
						Some(i),
						"x {probe} on page {first} should be tab {i}"
					);
				}
			}
			// A tab on another page is neither drawn nor hit.
			assert_eq!(slot_x(first + shown, tab_w, first, shown), None);
			if first > 0 {
				assert_eq!(slot_x(first - 1, tab_w, first, shown), None);
			}
			// Past the last drawn tab is the bare bar, not the tab before it.
			assert_eq!(tab_at_x(shown as f32 * tab_w, tab_w, first, shown), None);
			assert_eq!(tab_at_x(-1.0, tab_w, first, shown), None);
		}
		assert_eq!(tab_at_x(10.0, 0.0, 0, 3), None);
	}

	#[test]
	fn a_bar_with_no_width_still_answers() {
		assert!(tab_width(0.0, 3, 12.0, 26.0) >= 0.0);
		assert!(tab_width(-5.0, 0, 12.0, 26.0) >= 0.0);
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
}
