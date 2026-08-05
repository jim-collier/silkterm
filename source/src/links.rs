// Hyperlinks in terminal output: find URLs in a row of grid text, and hand one
// to the desktop's handler.
//
// Detection is allowlisted BY SCHEME and that is load-bearing twice over: it
// keeps the false-positive rate near zero (a bare word with a slash in it is not
// a link), and it is what stops `javascript:` / `data:` from ever reaching the
// opener. A scheme absent from SCHEMES is not a link, so it cannot be opened.

use std::io;
use std::process::{Command, Stdio};

// (scheme, needs "//"). Ordered longest-first within a family so "https" is
// tested before "http" - every candidate is tried anyway, but the order keeps
// the common case one comparison.
const SCHEMES: [(&str, bool); 8] = [
	("https", true),
	("http", true),
	("ftps", true),
	("ftp", true),
	("file", true),
	("sftp", true),
	("ssh", true),
	("mailto", false),
];

// Characters a URL may carry. ASCII only: percent-encoding covers the rest, and
// admitting non-ASCII would swallow the CJK sentence that follows a link. The
// exclusions are the delimiters RFC 3986 leaves out plus the quotes a shell
// wraps a URL in.
fn is_url_char(c: char) -> bool {
	c.is_ascii_graphic()
		&& !matches!(
			c,
			'"' | '\'' | '<' | '>' | '`' | '{' | '}' | '|' | '\\' | '^'
		)
}

// A link can't start mid-word, or "xhttp://x" reads as a link one char in.
fn boundary_before(text: &[char], start: usize) -> bool {
	match start.checked_sub(1).and_then(|i| text.get(i)) {
		Some(&c) => !c.is_ascii_alphanumeric() && !matches!(c, '+' | '-' | '.'),
		None => true,
	}
}

fn matches_scheme(text: &[char], start: usize, scheme: &str) -> bool {
	scheme.chars().enumerate().all(|(i, want)| {
		text.get(start + i)
			.is_some_and(|&c| c.eq_ignore_ascii_case(&want))
	})
}

// Sentence punctuation clings to the end of a URL far more often than it belongs
// to one, and a link inside brackets picks up the closer. Both come off; a
// closer that the URL itself opened (wikipedia's "(disambiguation)") stays.
fn trim_tail(text: &[char], body: usize, mut end: usize) -> usize {
	while end > body {
		let c = text[end - 1];
		if matches!(c, '.' | ',' | ';' | ':' | '!' | '?') {
			end -= 1;
			continue;
		}
		if let Some(open) = match c {
			')' => Some('('),
			']' => Some('['),
			_ => None,
		} {
			let opened = text[body..end].iter().filter(|&&x| x == open).count();
			let closed = text[body..end].iter().filter(|&&x| x == c).count();
			if closed > opened {
				end -= 1;
				continue;
			}
		}
		break;
	}
	end
}

// The link starting exactly at `start`, as a char range.
fn link_from(text: &[char], start: usize) -> Option<(usize, usize)> {
	if !boundary_before(text, start) {
		return None;
	}
	SCHEMES.iter().find_map(|&(scheme, slashes)| {
		if !matches_scheme(text, start, scheme) {
			return None;
		}
		let mut i = start + scheme.len();
		if text.get(i) != Some(&':') {
			return None;
		}
		i += 1;
		if slashes {
			if text.get(i) != Some(&'/') || text.get(i + 1) != Some(&'/') {
				return None;
			}
			i += 2;
		}
		let body = i;
		while text.get(i).is_some_and(|&c| is_url_char(c)) {
			i += 1;
		}
		let end = trim_tail(text, body, i);
		(end > body).then_some((start, end))
	})
}

// The link covering char `hit`, as (start, end, url). `text` is one logical
// line's chars; the caller maps the range back to grid cells.
pub fn find_at(text: &[char], hit: usize) -> Option<(usize, usize, String)> {
	if hit >= text.len() {
		return None;
	}
	let mut i = 0;
	while i < text.len() {
		match link_from(text, i) {
			Some((start, end)) => {
				if (start..end).contains(&hit) {
					return Some((start, end, text[start..end].iter().collect()));
				}
				i = end;
			}
			None => i += 1,
		}
	}
	None
}

// Hand `url` to the desktop. `open_command` (config) overrides the platform
// default: argv-split, the URL appended as the last argument. Runs detached -
// the child is reaped on its own thread so a browser launch can't zombie.
pub fn open(url: &str, open_command: &str) -> io::Result<()> {
	let mut cmd = if open_command.trim().is_empty() {
		default_command(url)
	} else {
		let argv = crate::cli::shell_split(open_command)
			.map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
		let (program, args) = argv
			.split_first()
			.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "empty open command"))?;
		let mut cmd = Command::new(program);
		cmd.args(args).arg(url);
		cmd
	};
	let mut child = cmd
		.stdin(Stdio::null())
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()?;
	std::thread::spawn(move || {
		let _ = child.wait();
	});
	Ok(())
}

#[cfg(target_os = "windows")]
fn default_command(url: &str) -> Command {
	use std::os::windows::process::CommandExt;
	// cmd's own parser sees the command line before argument quoting means
	// anything, so the metacharacters a query string carries have to be escaped
	// for it. The empty "" is start's title argument - without it start takes the
	// URL as the title and opens nothing.
	let mut escaped = String::with_capacity(url.len() + 8);
	for ch in url.chars() {
		if matches!(ch, '&' | '|' | '^' | '<' | '>' | '(' | ')') {
			escaped.push('^');
		}
		escaped.push(ch);
	}
	let mut cmd = Command::new("cmd");
	cmd.args(["/C", "start", "\"\""]).raw_arg(&escaped);
	cmd
}

#[cfg(target_os = "macos")]
fn default_command(url: &str) -> Command {
	let mut cmd = Command::new("open");
	cmd.arg(url);
	cmd
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn default_command(url: &str) -> Command {
	let mut cmd = Command::new("xdg-open");
	cmd.arg(url);
	cmd
}

#[cfg(test)]
mod tests {
	use super::*;

	fn chars(s: &str) -> Vec<char> {
		s.chars().collect()
	}

	// Find the link covering the first char of `needle`.
	fn at(line: &str, needle: &str) -> Option<String> {
		let text = chars(line);
		let hit = line[..line.find(needle).expect("needle")].chars().count();
		find_at(&text, hit).map(|(_, _, url)| url)
	}

	#[test]
	fn a_plain_url_is_found_anywhere_inside_it() {
		let line = "see https://example.com/a?b=1&c=2 for more";
		let text = chars(line);
		let (start, end, url) = find_at(&text, 10).expect("link");
		assert_eq!(url, "https://example.com/a?b=1&c=2");
		assert_eq!((start, end), (4, 33));
		// every cell of the span answers with the same span
		for hit in start..end {
			assert_eq!(find_at(&text, hit).map(|l| l.2), Some(url.clone()));
		}
		assert_eq!(
			find_at(&text, start - 1),
			None,
			"the space before is not one"
		);
		assert_eq!(find_at(&text, end), None, "the space after is not one");
	}

	#[test]
	fn sentence_punctuation_and_brackets_come_off_the_end() {
		assert_eq!(
			at("visit http://example.com/x.", "http"),
			Some("http://example.com/x".into())
		);
		assert_eq!(
			at("(see http://example.com/x)", "http"),
			Some("http://example.com/x".into())
		);
		// a closer the URL itself opened is part of it
		assert_eq!(
			at("http://en.wikipedia.org/wiki/Ruby_(gem)", "http"),
			Some("http://en.wikipedia.org/wiki/Ruby_(gem)".into())
		);
		assert_eq!(
			at("quoted 'https://example.com/q' here", "https"),
			Some("https://example.com/q".into())
		);
	}

	// The allowlist is the security boundary, not a convenience: a scheme that
	// isn't listed must never become clickable.
	#[test]
	fn only_allowlisted_schemes_are_links() {
		assert_eq!(at("javascript:alert(1)", "javascript"), None);
		assert_eq!(at("data:text/html;base64,AAAA", "data"), None);
		assert_eq!(at("vbscript:msgbox", "vbscript"), None);
		assert_eq!(
			at("mail me at mailto:a@b.com now", "mailto"),
			Some("mailto:a@b.com".into())
		);
	}

	#[test]
	fn ordinary_text_with_a_colon_is_not_a_link() {
		assert_eq!(at("aspect ratio 3:4 here", "3:4"), None);
		assert_eq!(at("C:\\Users\\jim\\file.txt", "C:"), None);
		assert_eq!(at("std::vec::Vec", "std"), None);
		assert_eq!(at("error: http", "http"), None, "no scheme separator");
		assert_eq!(at("xhttps://example.com", "https"), None, "mid-word start");
		assert_eq!(at("https://", "https"), None, "no body");
	}

	#[test]
	fn the_scheme_is_case_insensitive_and_several_links_coexist() {
		assert_eq!(
			at("HTTPS://Example.COM/A", "HTTPS"),
			Some("HTTPS://Example.COM/A".into())
		);
		let line = "a http://one.example b ftp://two.example c";
		assert_eq!(at(line, "http"), Some("http://one.example".into()));
		assert_eq!(at(line, "ftp"), Some("ftp://two.example".into()));
		assert_eq!(at(line, " c"), None);
	}
}
