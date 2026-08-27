// What a double-click takes when the text under the pointer is a shape we can
// name: a URL, a Windows or posix path, an scp target. Word selection cuts those
// in the wrong places - a space in a folder name ends the path early, a `:12`
// line number gets dragged in, a bracket in a wiki URL splits it - so a shape
// that is recognized here is taken whole and outranks both the pair rule and the
// word rule.
//
// Everything a shape can start with is anchored: a drive letter, a UNC prefix, a
// leading slash or `~/`. Nothing else is a path, which is what keeps prose out.

// How far past a space a path separator may sit and still read as part of the
// same path. A folder name runs to a few words; prose does not reach a slash
// that soon.
const SPACE_REACH: usize = 40;

// The longest run of characters after a dot that still reads as a file
// extension. Long enough for the ones people use, short enough that a sentence
// ending in a dot plus a word does not qualify.
const EXT_MAX: usize = 8;

/// The shape covering char `hit`, as a char range, or None when the text there
/// is just text. `text` is one logical line's chars.
pub fn span_at(text: &[char], hit: usize) -> Option<(usize, usize)> {
	if hit >= text.len() {
		return None;
	}
	url_span(text, hit).or_else(|| path_span(text, hit))
}

fn url_span(text: &[char], hit: usize) -> Option<(usize, usize)> {
	crate::links::find_at(text, hit).map(|(start, end, _)| (start, end))
}

fn is_sep(c: char) -> bool {
	c == '\\' || c == '/'
}

// Characters a path may carry. Whitespace ends a run (crossing one is decided
// separately) and the rest are the ones Windows forbids in a name outright plus
// the double quote a shell would have wrapped the path in.
fn is_path_char(c: char) -> bool {
	!c.is_whitespace() && !c.is_control() && !matches!(c, '"' | '<' | '>' | '|' | '*' | '?')
}

// A shape cannot start mid-word: "xC:\a" is not a drive path and "and/or" is not
// an absolute one. Listed the other way round on purpose - what may sit in front
// of a path is a short closed set, and everything else is part of a name.
fn boundary_before(text: &[char], start: usize) -> bool {
	match start.checked_sub(1).and_then(|i| text.get(i)) {
		Some(&c) => {
			c.is_whitespace()
				|| matches!(
					c,
					'"' | '\'' | '(' | '[' | '{' | '<' | '=' | ',' | ';' | '|' | '`'
				)
		}
		None => true,
	}
}

// Does a path start exactly at `at`? Returns how many chars the prefix takes.
fn anchor_len(text: &[char], at: usize) -> Option<usize> {
	if !boundary_before(text, at) {
		return None;
	}
	let c = *text.get(at)?;
	// C:\ or C:/
	if c.is_ascii_alphabetic()
		&& text.get(at + 1) == Some(&':')
		&& text.get(at + 2).is_some_and(|&c| is_sep(c))
	{
		return Some(3);
	}
	// \\server\share
	if c == '\\' && text.get(at + 1) == Some(&'\\') && text.get(at + 2).is_some_and(|&c| !is_sep(c))
	{
		return Some(2);
	}
	// ~/ and ~\
	if c == '~' && text.get(at + 1).is_some_and(|&c| is_sep(c)) {
		return Some(2);
	}
	// /usr/... - a lone slash is not a path, so the next char has to carry one
	if c == '/'
		&& text
			.get(at + 1)
			.is_some_and(|&c| is_path_char(c) && !is_sep(c))
	{
		return Some(1);
	}
	None
}

// The nearest path start at or before `hit` whose span still reaches `hit`.
fn path_span(text: &[char], hit: usize) -> Option<(usize, usize)> {
	(0..=hit).rev().find_map(|start| {
		let prefix = anchor_len(text, start)?;
		let end = path_end(text, start, prefix);
		(end > hit && end > start + prefix).then_some((start, end))
	})
}

// Where the path starting at `start` ends. Runs of path characters, plus any
// space a folder name turns out to have in it, stopping at the file extension
// once there is one.
fn path_end(text: &[char], start: usize, prefix: usize) -> usize {
	let mut end = start + prefix;
	loop {
		while text.get(end).is_some_and(|&c| is_path_char(c)) {
			end += 1;
		}
		// A `:` after the extension is a line number, not part of the name.
		if let Some(cut) = ends_at_extension(&text[start..end]) {
			return start + cut;
		}
		if text.get(end) != Some(&' ') || !separator_within_reach(text, end + 1) {
			break;
		}
		end += 1;
	}
	trim_tail(text, start, end)
}

// Is there a path separator close enough after `from` to believe the space we
// just met is inside a folder name rather than after the path?
fn separator_within_reach(text: &[char], from: usize) -> bool {
	if !text.get(from).is_some_and(|&c| is_path_char(c)) {
		return false;
	}
	text.iter()
		.skip(from)
		.take(SPACE_REACH)
		.take_while(|c| !c.is_control())
		.any(|&c| is_sep(c))
}

// Where the name ends if this run already carries a file extension, as an offset
// into the run. Only the last segment counts, and what FOLLOWS the extension
// decides: another dot means a second part (`a.tar.gz`), a separator means it was
// a directory all along, and anything else means the name ended and the `:120:5`
// after it is somebody's line number.
fn ends_at_extension(run: &[char]) -> Option<usize> {
	let last_sep = run.iter().rposition(|&c| is_sep(c))?;
	let mut i = last_sep + 1;
	while i < run.len() {
		if run[i] != '.' || i == last_sep + 1 {
			i += 1;
			continue;
		}
		let mut j = i + 1;
		while j < run.len() && run[j].is_ascii_alphanumeric() {
			j += 1;
		}
		let ext = j - i - 1;
		if ext == 0 || ext > EXT_MAX {
			i += 1;
			continue;
		}
		match run.get(j) {
			Some(&'.') => i = j,
			Some(&c) if is_sep(c) => return None,
			_ => return Some(j),
		}
	}
	None
}

// Sentence punctuation clings to the end of a path as readily as to a URL, and a
// path inside brackets or quotes picks up the closer.
fn trim_tail(text: &[char], start: usize, mut end: usize) -> usize {
	while end > start {
		let c = text[end - 1];
		if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
			end -= 1;
			continue;
		}
		let opener = match c {
			')' => '(',
			']' => '[',
			'}' => '{',
			'\'' => '\'',
			_ => break,
		};
		let body = &text[start..end - 1];
		let opened = body.iter().filter(|&&x| x == opener).count();
		let closed = body.iter().filter(|&&x| x == c).count();
		if opener == c {
			if opened % 2 == 1 {
				break; // the path opened this quote itself
			}
		} else if opened > closed {
			break;
		}
		end -= 1;
	}
	end
}

#[cfg(test)]
mod tests {
	use super::*;

	fn at(line: &str, needle: &str) -> Option<String> {
		let text: Vec<char> = line.chars().collect();
		let hit = line.find(needle).expect("needle not in line");
		let hit = line[..hit].chars().count();
		span_at(&text, hit).map(|(s, e)| text[s..e].iter().collect())
	}

	#[test]
	fn a_drive_path_keeps_its_drive_letter() {
		assert_eq!(
			at("open C:\\Users\\jim\\notes.txt now", "jim").as_deref(),
			Some("C:\\Users\\jim\\notes.txt")
		);
		assert_eq!(
			at("C:/Users/jim/notes.txt", "Users").as_deref(),
			Some("C:/Users/jim/notes.txt")
		);
	}

	// The case word selection cannot do at all: the space ends the word, so a
	// double-click used to give back "Files\app.exe".
	#[test]
	fn a_folder_name_may_have_spaces_in_it() {
		assert_eq!(
			at("path C:\\Program Files\\app.exe end", "app").as_deref(),
			Some("C:\\Program Files\\app.exe")
		);
		assert_eq!(
			at("C:\\Program Files (x86)\\App\\a.exe more", "App").as_deref(),
			Some("C:\\Program Files (x86)\\App\\a.exe")
		);
	}

	// Prose after a path is prose, not more path - there is no separator in it.
	#[test]
	fn a_space_with_no_separator_after_it_ends_the_path() {
		assert_eq!(
			at("/home/u/Documents and more words", "Documents").as_deref(),
			Some("/home/u/Documents")
		);
		assert_eq!(
			at("see /etc/hosts for details", "etc").as_deref(),
			Some("/etc/hosts")
		);
	}

	#[test]
	fn a_line_number_after_the_extension_is_not_part_of_the_name() {
		assert_eq!(
			at("/src/app/main.rs:120:5", "main").as_deref(),
			Some("/src/app/main.rs")
		);
		assert_eq!(
			at("/tmp/x/backup.tar.gz done", "backup").as_deref(),
			Some("/tmp/x/backup.tar.gz")
		);
	}

	#[test]
	fn a_url_wins_over_everything_else() {
		assert_eq!(
			at("see https://example.com/a?b=c fine", "example").as_deref(),
			Some("https://example.com/a?b=c")
		);
		assert_eq!(
			at("https://en.wikipedia.org/wiki/Foo_(bar)", "wiki").as_deref(),
			Some("https://en.wikipedia.org/wiki/Foo_(bar)")
		);
		assert_eq!(
			at("file:///C:/Users/jim/a.txt", "Users").as_deref(),
			Some("file:///C:/Users/jim/a.txt")
		);
	}

	#[test]
	fn a_unc_path_and_a_home_path_are_shapes_too() {
		assert_eq!(
			at("\\\\server\\share\\file.txt", "share").as_deref(),
			Some("\\\\server\\share\\file.txt")
		);
		assert_eq!(at("~/bin/tool.sh", "bin").as_deref(), Some("~/bin/tool.sh"));
	}

	#[test]
	fn ordinary_text_is_left_to_the_word_rules() {
		assert_eq!(at("this and/or that", "and"), None);
		assert_eq!(at("a ratio of 3/4 here", "ratio"), None);
		assert_eq!(at("no shapes at all", "shapes"), None);
		assert_eq!(at("xC:\\notapath here", "notapath"), None);
	}

	#[test]
	fn a_quoted_or_bracketed_path_sheds_the_wrapper() {
		assert_eq!(
			at("'/home/u/a b/c.txt'", "home").as_deref(),
			Some("/home/u/a b/c.txt")
		);
		assert_eq!(
			at("(/home/u/notes.md), yes", "notes").as_deref(),
			Some("/home/u/notes.md")
		);
	}

	#[test]
	fn a_click_past_the_end_of_the_row_finds_nothing() {
		let text: Vec<char> = "/etc/hosts".chars().collect();
		assert_eq!(span_at(&text, 99), None);
	}
}
