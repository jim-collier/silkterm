// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

//! Thin wrapper over copypasta for the regular CLIPBOARD plus, on Linux/X11,
//! the PRIMARY selection (middle-click paste). The contexts are held for the
//! app's lifetime: X11 selection ownership lives in the context, so dropping it
//! would drop the copied text. Everything is best-effort - a missing/foreign
//! display just yields no clipboard rather than an error.

use copypasta::ClipboardProvider;

#[cfg(target_os = "linux")]
use copypasta::x11_clipboard::{Primary, X11ClipboardContext};

pub struct Clipboard {
	clipboard: Option<copypasta::ClipboardContext>,
	#[cfg(target_os = "linux")]
	primary: Option<X11ClipboardContext<Primary>>,
}

impl Clipboard {
	pub fn new() -> Self {
		Self {
			clipboard: copypasta::ClipboardContext::new().ok(),
			#[cfg(target_os = "linux")]
			primary: X11ClipboardContext::<Primary>::new().ok(),
		}
	}

	/// Copy text to the primary selection (falls back to the clipboard where
	/// there's no primary, i.e. non-Linux).
	pub fn set_primary(&mut self, text: String) {
		#[cfg(target_os = "linux")]
		if let Some(p) = self.primary.as_mut() {
			let _ = p.set_contents(text);
			return;
		}
		self.set_clipboard(text);
	}

	/// Read the primary selection (middle-click paste source).
	pub fn get_primary(&mut self) -> Option<String> {
		#[cfg(target_os = "linux")]
		if let Some(p) = self.primary.as_mut() {
			return p.get_contents().ok().filter(|s| !s.is_empty());
		}
		self.get_clipboard()
	}

	pub fn set_clipboard(&mut self, text: String) {
		if let Some(c) = self.clipboard.as_mut() {
			let _ = c.set_contents(text);
		}
	}

	pub fn get_clipboard(&mut self) -> Option<String> {
		self.clipboard
			.as_mut()?
			.get_contents()
			.ok()
			.filter(|s| !s.is_empty())
	}
}
