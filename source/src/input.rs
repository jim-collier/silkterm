// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use winit::event::KeyEvent;
use winit::keyboard::{Key, ModifiersState, NamedKey};

// Cursor-key sequence. In application-cursor-keys mode (DECCKM, set by full
// screen apps like `less`/vim) these use the SS3 (`ESC O`) form; otherwise CSI
// (`ESC [`). Sending the wrong form is why `less` arrow keys did nothing.
pub fn cursor_seq(letter: u8, app_cursor: bool) -> Vec<u8> {
	let prefix = if app_cursor { b'O' } else { b'[' };
	vec![0x1b, prefix, letter]
}

// Translate a key press into the bytes a PTY expects. Returns None for keys
// we don't forward (modifiers alone, unhandled named keys, etc.).
pub fn encode(ev: &KeyEvent, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
	encode_key(&ev.logical_key, ev.text.as_deref(), mods, app_cursor)
}

// The tilde-form CSI number for a key, if it has one (`ESC [ <n> ~`).
fn tilde_num(named: &NamedKey) -> Option<u8> {
	Some(match named {
		NamedKey::Insert => 2,
		NamedKey::Delete => 3,
		NamedKey::PageUp => 5,
		NamedKey::PageDown => 6,
		NamedKey::F5 => 15,
		NamedKey::F6 => 17, // 16 is skipped by xterm, as are 22 and 25
		NamedKey::F7 => 18,
		NamedKey::F8 => 19,
		NamedKey::F9 => 20,
		NamedKey::F10 => 21,
		NamedKey::F11 => 23,
		NamedKey::F12 => 24,
		_ => return None,
	})
}

// The final byte of the letter-form CSI for a key (`ESC [ 1 ; <m> <letter>`);
// same finals as the unmodified SS3/CSI forms for cursor keys and F1-F4.
fn letter_final(named: &NamedKey) -> Option<u8> {
	Some(match named {
		NamedKey::ArrowUp => b'A',
		NamedKey::ArrowDown => b'B',
		NamedKey::ArrowRight => b'C',
		NamedKey::ArrowLeft => b'D',
		NamedKey::Home => b'H',
		NamedKey::End => b'F',
		NamedKey::F1 => b'P',
		NamedKey::F2 => b'Q',
		NamedKey::F3 => b'R',
		NamedKey::F4 => b'S',
		_ => return None,
	})
}

fn encode_key(
	key: &Key,
	text: Option<&str>,
	mods: ModifiersState,
	app_cursor: bool,
) -> Option<Vec<u8>> {
	let ctrl = mods.control_key();
	let alt = mods.alt_key();
	let shift = mods.shift_key();
	// xterm modifier parameter: 1 + shift(1) + alt(2) + ctrl(4)
	let m = 1 + shift as u8 + ((alt as u8) << 1) + ((ctrl as u8) << 2);

	let with_alt = |bytes: Vec<u8>| -> Vec<u8> {
		if alt {
			let mut v = vec![0x1b];
			v.extend_from_slice(&bytes);
			v
		} else {
			bytes
		}
	};

	match key {
		Key::Named(named) => {
			// Modified navigation/function keys use the xterm `;<m>` forms
			// (Ctrl+Arrow word-skip, Ctrl+Del, Shift+F<n>, ...). These replace
			// the ESC prefix for Alt too - apps expect CSI 1;3A, not ESC CSI A.
			if m > 1 {
				if *named == NamedKey::Backspace && ctrl {
					// xterm/VTE convention; shells bind ^H to a word delete
					return Some(with_alt(vec![0x08]));
				}
				if let Some(l) = letter_final(named) {
					return Some(format!("\x1b[1;{m}{}", l as char).into_bytes());
				}
				if let Some(n) = tilde_num(named) {
					return Some(format!("\x1b[{n};{m}~").into_bytes());
				}
			}
			let bytes: Vec<u8> = match named {
				NamedKey::Enter => vec![b'\r'],
				NamedKey::Backspace => vec![0x7f],
				NamedKey::Tab => {
					if shift {
						return Some(b"\x1b[Z".to_vec());
					}
					vec![b'\t']
				}
				NamedKey::Escape => vec![0x1b],
				NamedKey::Space => vec![b' '],
				NamedKey::ArrowUp => cursor_seq(b'A', app_cursor),
				NamedKey::ArrowDown => cursor_seq(b'B', app_cursor),
				NamedKey::ArrowRight => cursor_seq(b'C', app_cursor),
				NamedKey::ArrowLeft => cursor_seq(b'D', app_cursor),
				NamedKey::Home => cursor_seq(b'H', app_cursor),
				NamedKey::End => cursor_seq(b'F', app_cursor),
				// F1-F4 are SS3; the rest of the named keys we forward are tilde-form
				NamedKey::F1 => b"\x1bOP".to_vec(),
				NamedKey::F2 => b"\x1bOQ".to_vec(),
				NamedKey::F3 => b"\x1bOR".to_vec(),
				NamedKey::F4 => b"\x1bOS".to_vec(),
				_ => match tilde_num(named) {
					Some(n) => format!("\x1b[{n}~").into_bytes(),
					None => return None,
				},
			};
			Some(with_alt(bytes))
		}
		Key::Character(s) => {
			if ctrl {
				// map ctrl+<char> to its control code
				let c = s.chars().next()?;
				let lower = c.to_ascii_lowercase();
				let code = match lower {
					'a'..='z' => (lower as u8 - b'a') + 1,
					'@' => 0,
					'[' => 0x1b,
					'\\' => 0x1c,
					']' => 0x1d,
					'^' => 0x1e,
					'_' => 0x1f,
					' ' => 0,
					_ => return None,
				};
				return Some(with_alt(vec![code]));
			}
			// printable text from the platform layout
			let text = text.map(|t| t.as_bytes().to_vec())?;
			Some(with_alt(text))
		}
		_ => None,
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const NONE: ModifiersState = ModifiersState::empty();

	fn enc(named: NamedKey, mods: ModifiersState, app_cursor: bool) -> Option<Vec<u8>> {
		encode_key(&Key::Named(named), None, mods, app_cursor)
	}

	#[test]
	fn arrows_follow_decckm() {
		assert_eq!(enc(NamedKey::ArrowUp, NONE, false).unwrap(), b"\x1b[A");
		assert_eq!(enc(NamedKey::ArrowUp, NONE, true).unwrap(), b"\x1bOA");
		assert_eq!(enc(NamedKey::End, NONE, false).unwrap(), b"\x1b[F");
	}

	#[test]
	fn modified_arrows_use_csi_mod_form() {
		// Ctrl+Right = word skip in readline/most TUIs
		let ctrl = ModifiersState::CONTROL;
		assert_eq!(
			enc(NamedKey::ArrowRight, ctrl, false).unwrap(),
			b"\x1b[1;5C"
		);
		// modified keys stay CSI even in app-cursor mode
		assert_eq!(enc(NamedKey::ArrowRight, ctrl, true).unwrap(), b"\x1b[1;5C");
		assert_eq!(
			enc(NamedKey::ArrowLeft, ModifiersState::SHIFT, false).unwrap(),
			b"\x1b[1;2D"
		);
		assert_eq!(
			enc(NamedKey::ArrowUp, ModifiersState::ALT, false).unwrap(),
			b"\x1b[1;3A"
		);
		assert_eq!(
			enc(NamedKey::ArrowDown, ctrl | ModifiersState::SHIFT, false).unwrap(),
			b"\x1b[1;6B"
		);
	}

	#[test]
	fn function_keys() {
		assert_eq!(enc(NamedKey::F1, NONE, false).unwrap(), b"\x1bOP");
		assert_eq!(enc(NamedKey::F5, NONE, false).unwrap(), b"\x1b[15~");
		assert_eq!(enc(NamedKey::F6, NONE, false).unwrap(), b"\x1b[17~");
		assert_eq!(enc(NamedKey::F10, NONE, false).unwrap(), b"\x1b[21~");
		assert_eq!(enc(NamedKey::F12, NONE, false).unwrap(), b"\x1b[24~");
		assert_eq!(
			enc(NamedKey::F1, ModifiersState::CONTROL, false).unwrap(),
			b"\x1b[1;5P"
		);
		assert_eq!(
			enc(NamedKey::F5, ModifiersState::SHIFT, false).unwrap(),
			b"\x1b[15;2~"
		);
	}

	#[test]
	fn editing_keys() {
		let ctrl = ModifiersState::CONTROL;
		assert_eq!(enc(NamedKey::Delete, NONE, false).unwrap(), b"\x1b[3~");
		assert_eq!(enc(NamedKey::Delete, ctrl, false).unwrap(), b"\x1b[3;5~");
		assert_eq!(enc(NamedKey::Backspace, NONE, false).unwrap(), [0x7f]);
		assert_eq!(enc(NamedKey::Backspace, ctrl, false).unwrap(), [0x08]);
		assert_eq!(enc(NamedKey::PageUp, NONE, false).unwrap(), b"\x1b[5~");
		assert_eq!(
			enc(NamedKey::Tab, ModifiersState::SHIFT, false).unwrap(),
			b"\x1b[Z"
		);
	}

	#[test]
	fn ctrl_chars_and_alt_prefix() {
		let ctrl = ModifiersState::CONTROL;
		let a = Key::Character("a".into());
		assert_eq!(encode_key(&a, Some("a"), ctrl, false).unwrap(), [0x01]);
		assert_eq!(
			encode_key(&a, Some("a"), NONE, false).unwrap(),
			b"a".to_vec()
		);
		assert_eq!(
			encode_key(&a, Some("a"), ModifiersState::ALT, false).unwrap(),
			b"\x1ba".to_vec()
		);
	}
}
