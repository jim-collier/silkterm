// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::tty;
use winit::event_loop::EventLoopProxy;

pub type PaneId = u64;

#[derive(Debug, Clone)]
pub enum UserEvent {
	// new output in this pane's terminal (render only what changed)
	Wakeup(PaneId),
	Title(PaneId, String),
	// terminal replies (cursor position report, device attributes, ...) that
	// must be written back to the PTY
	PtyWrite(PaneId, Vec<u8>),
	Exit(PaneId),
	// terminal bell (BEL): drives a brief visual flash (text brightens, fades back)
	Bell,
}

// bridges alacritty's PTY thread back to the winit loop
#[derive(Clone)]
pub struct EventProxy {
	id: PaneId,
	proxy: EventLoopProxy<UserEvent>,
}

impl EventProxy {
	pub fn new(id: PaneId, proxy: EventLoopProxy<UserEvent>) -> Self {
		Self { id, proxy }
	}
}

impl EventListener for EventProxy {
	fn send_event(&self, event: Event) {
		let _ = match event {
			Event::Wakeup => self.proxy.send_event(UserEvent::Wakeup(self.id)),
			Event::Title(t) => self.proxy.send_event(UserEvent::Title(self.id, t)),
			Event::ResetTitle => self
				.proxy
				.send_event(UserEvent::Title(self.id, crate::config::APP_NAME.into())),
			Event::Exit | Event::ChildExit(_) => self.proxy.send_event(UserEvent::Exit(self.id)),
			Event::PtyWrite(text) => self
				.proxy
				.send_event(UserEvent::PtyWrite(self.id, text.into_bytes())),
			Event::Bell => self.proxy.send_event(UserEvent::Bell),
			Event::MouseCursorDirty => Ok(()),
			_ => Ok(()),
		};
	}
}

// size descriptor handed to the crate; history is set separately via Config
#[derive(Clone, Copy)]
pub struct TermDimensions {
	pub columns: usize,
	pub screen_lines: usize,
}

impl Dimensions for TermDimensions {
	fn total_lines(&self) -> usize {
		self.screen_lines
	}
	fn screen_lines(&self) -> usize {
		self.screen_lines
	}
	fn columns(&self) -> usize {
		self.columns
	}
}

pub struct TermInstance {
	pub term: Arc<FairMutex<Term<EventProxy>>>,
	pub cols: usize,
	pub lines: usize,
	sender: EventLoopSender,
	io: Option<std::thread::JoinHandle<()>>,
	// for tab titles: the PTY master fd (foreground-process group) + shell pid;
	// `shell_name` is cached, `last_program` tracks the most recent foreground.
	#[cfg(unix)]
	master_fd: std::os::unix::io::RawFd,
	#[cfg(unix)]
	shell_pid: u32,
	#[cfg(unix)]
	shell_name: Option<String>,
	#[cfg(unix)]
	last_program: Option<String>,
}

impl TermInstance {
	pub fn spawn(
		id: PaneId,
		cols: usize,
		lines: usize,
		cell_w: u16,
		cell_h: u16,
		proxy: EventLoopProxy<UserEvent>,
		command: Option<Vec<String>>,
	) -> anyhow::Result<Self> {
		let cols = cols.max(1);
		let lines = lines.max(1);

		let mut config = Config::default();
		config.scrolling_history = crate::config::settings().scrollback;
		config.semantic_escape_chars = crate::config::settings().word_separators.clone();

		let dims = TermDimensions {
			columns: cols,
			screen_lines: lines,
		};
		let event_proxy = EventProxy::new(id, proxy);
		let term = Arc::new(FairMutex::new(Term::new(
			config,
			&dims,
			event_proxy.clone(),
		)));

		let win = WindowSize {
			num_cols: cols as u16,
			num_lines: lines as u16,
			cell_width: cell_w,
			cell_height: cell_h,
		};

		// a CLI/menu-supplied command runs as argv[0] + args; else the default shell
		let mut opts = tty::Options::default();
		if let Some((prog, args)) = command.as_ref().and_then(|c| c.split_first()) {
			opts.shell = Some(tty::Shell::new(prog.clone(), args.to_vec()));
		}
		let pty = tty::new(&opts, win, id)?;
		// Capture the master fd + shell pid before the event loop takes the pty;
		// they drive the tab title (foreground program). The fd stays valid for
		// the pane's life (the loop owns the pty until close).
		#[cfg(unix)]
		let master_fd = {
			use std::os::unix::io::AsRawFd;
			pty.file().as_raw_fd()
		};
		#[cfg(unix)]
		let shell_pid = pty.child().id();
		let event_loop = EventLoop::new(term.clone(), event_proxy, pty, false, false)?;
		let sender = event_loop.channel();
		let handle = event_loop.spawn();
		// wrap the join handle so we don't carry its tuple return type around
		let io = std::thread::spawn(move || {
			let _ = handle.join();
		});

		Ok(Self {
			term,
			cols,
			lines,
			sender,
			io: Some(io),
			#[cfg(unix)]
			master_fd,
			#[cfg(unix)]
			shell_pid,
			#[cfg(unix)]
			shell_name: None,
			#[cfg(unix)]
			last_program: None,
		})
	}

	// Tab title: "<shell> [<program>]" while a foreground program runs, or
	// "<shell> [last: <program>]" / "<shell>" when only the shell is at the
	// prompt. Names are executable basenames (from /proc comm), not full
	// command lines. Unix only; elsewhere falls back to the app name.
	#[cfg(unix)]
	pub fn tab_title(&mut self) -> String {
		let shell = self
			.shell_name
			.get_or_insert_with(|| proc_comm(self.shell_pid).unwrap_or_else(|| "shell".into()))
			.clone();
		let pgid = unsafe { libc::tcgetpgrp(self.master_fd) };
		let fg = if pgid > 0 {
			proc_comm(pgid as u32)
		} else {
			None
		};
		match fg {
			Some(p) if p != shell => {
				self.last_program = Some(p.clone());
				format!("{shell} [{p}]")
			}
			_ => match &self.last_program {
				Some(l) => format!("{shell} [last: {l}]"),
				None => shell,
			},
		}
	}

	#[cfg(not(unix))]
	pub fn tab_title(&mut self) -> String {
		crate::config::APP_NAME.to_string()
	}

	pub fn write<B: Into<Vec<u8>>>(&self, bytes: B) {
		let _ = self.sender.send(Msg::Input(bytes.into().into()));
	}

	pub fn resize(&mut self, cols: usize, lines: usize, cell_w: u16, cell_h: u16) {
		let cols = cols.max(1);
		let lines = lines.max(1);
		if cols == self.cols && lines == self.lines {
			return;
		}
		self.cols = cols;
		self.lines = lines;
		let dims = TermDimensions {
			columns: cols,
			screen_lines: lines,
		};
		self.term.lock_unfair().resize(dims);
		let win = WindowSize {
			num_cols: cols as u16,
			num_lines: lines as u16,
			cell_width: cell_w,
			cell_height: cell_h,
		};
		let _ = self.sender.send(Msg::Resize(win));
	}
}

impl Drop for TermInstance {
	fn drop(&mut self) {
		let _ = self.sender.send(Msg::Shutdown);
		if let Some(io) = self.io.take() {
			let _ = io.join();
		}
	}
}

// Executable basename of a process from /proc/<pid>/comm (Linux/most Unix).
#[cfg(unix)]
fn proc_comm(pid: u32) -> Option<String> {
	let s = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
	let s = s.trim();
	if s.is_empty() {
		None
	} else {
		Some(s.to_string())
	}
}
