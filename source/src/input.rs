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
	let ctrl = mods.control_key();
	let alt = mods.alt_key();
	let shift = mods.shift_key();

	let with_alt = |bytes: Vec<u8>| -> Vec<u8> {
		if alt {
			let mut v = vec![0x1b];
			v.extend_from_slice(&bytes);
			v
		} else {
			bytes
		}
	};

	match &ev.logical_key {
		Key::Named(named) => {
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
				NamedKey::PageUp => b"\x1b[5~".to_vec(),
				NamedKey::PageDown => b"\x1b[6~".to_vec(),
				NamedKey::Insert => b"\x1b[2~".to_vec(),
				NamedKey::Delete => b"\x1b[3~".to_vec(),
				NamedKey::F1 => b"\x1bOP".to_vec(),
				NamedKey::F2 => b"\x1bOQ".to_vec(),
				NamedKey::F3 => b"\x1bOR".to_vec(),
				NamedKey::F4 => b"\x1bOS".to_vec(),
				_ => return None,
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
			let text = ev.text.as_ref().map(|t| t.as_bytes().to_vec())?;
			Some(with_alt(text))
		}
		_ => None,
	}
}
