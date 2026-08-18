// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

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
	// control socket (ctl.rs): change the background image live (None = clear).
	// ctl is Unix-only, so these are never constructed on non-unix.
	#[cfg_attr(not(unix), allow(dead_code))]
	SetWallpaper(Option<std::path::PathBuf>),
	// control socket: re-read config.shcl and apply it (same as Menu > Reload)
	#[cfg_attr(not(unix), allow(dead_code))]
	ReloadSettings,
	// wallpaper worker (wallpaper.rs): decoded pixels, ready to upload. Boxed -
	// it carries a whole image, and every other variant is small.
	WallpaperReady(Box<crate::wallpaper::Loaded>),
	// VT watcher thread (app.rs spawn_vt_watch): the active console changed.
	// Linux GL path only; never constructed elsewhere.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	VtSwitched,
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
			// MouseCursorDirty and any other events: nothing to forward
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
	// throttles the per-frame title probe (see tab_title)
	#[cfg(unix)]
	title_cache: Option<(std::time::Instant, String)>,
	// windows: the shell's pid and the time it started, for the child-process
	// probe that stands in for a foreground process group (see at_shell_prompt),
	// plus that probe's answer for as long as it holds (see note_activity).
	// Either 0 means "unknown".
	#[cfg(windows)]
	shell_pid: u32,
	#[cfg(windows)]
	shell_started: u64,
	#[cfg(windows)]
	prompt_probe: std::cell::Cell<Option<bool>>,
}

impl TermInstance {
	// command is owned by the spawned terminal conceptually; the by-value
	// constructor input threads through split_at/spawn_pane as a move
	#[allow(clippy::needless_pass_by_value)]
	pub fn spawn(
		id: PaneId,
		cols: usize,
		lines: usize,
		cell_w: u16,
		cell_h: u16,
		proxy: EventLoopProxy<UserEvent>,
		command: Option<Vec<String>>,
		cwd: Option<std::path::PathBuf>,
	) -> anyhow::Result<Self> {
		let cols = cols.max(1);
		let lines = lines.max(1);

		let mut config = Config::default();
		config.scrolling_history = crate::config::settings().scrollback;
		config
			.semantic_escape_chars
			.clone_from(&crate::config::settings().word_separators);

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
		// start in an inherited directory (new tab/split follows the source pane)
		opts.working_directory = cwd;
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
		// Windows has no master fd; the ConPTY child watcher carries the shell pid.
		#[cfg(windows)]
		let shell_pid = pty
			.child_watcher()
			.pid()
			.map_or(0, std::num::NonZeroU32::get);
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
			#[cfg(unix)]
			title_cache: None,
			#[cfg(windows)]
			shell_pid,
			#[cfg(windows)]
			shell_started: process_start_time(shell_pid).unwrap_or(0),
			#[cfg(windows)]
			prompt_probe: std::cell::Cell::new(None),
		})
	}

	// Tab title: "<shell> [<program>]" while a foreground program runs, or
	// "<shell> [last: <program>]" / "<shell>" when only the shell is at the
	// prompt. Names are executable basenames (from /proc comm), not full
	// command lines. Unix only; elsewhere falls back to the app name.
	// The probe (tcgetpgrp + a /proc read) is throttled: render asks per tab
	// per frame, and paying syscalls on every idle blink frame added up.
	#[cfg(unix)]
	pub fn tab_title(&mut self) -> String {
		const PROBE_IVL: std::time::Duration = std::time::Duration::from_millis(250);
		let now = std::time::Instant::now();
		if let Some((at, title)) = &self.title_cache {
			if now.duration_since(*at) < PROBE_IVL {
				return title.clone();
			}
		}
		let title = self.probe_title();
		self.title_cache = Some((now, title.clone()));
		title
	}

	#[cfg(unix)]
	fn probe_title(&mut self) -> String {
		let shell = self
			.shell_name
			.get_or_insert_with(|| proc_comm(self.shell_pid).unwrap_or_else(|| "shell".into()))
			.clone();
		let pgid = unsafe { libc::tcgetpgrp(self.master_fd) };
		let fg_program = if pgid > 0 {
			proc_comm(pgid as u32)
		} else {
			None
		};
		match fg_program {
			Some(program) if program != shell => {
				self.last_program = Some(program.clone());
				format!("{shell} [{program}]")
			}
			_ => match &self.last_program {
				Some(last_program) => format!("{shell} [last: {last_program}]"),
				None => shell,
			},
		}
	}

	// self kept for signature parity with the unix version above
	#[cfg(not(unix))]
	#[allow(clippy::unused_self)]
	pub fn tab_title(&mut self) -> String {
		crate::config::APP_NAME.to_string()
	}

	// The shell's current directory, for a new tab/split to start in. A deleted
	// dir reads back with a " (deleted)" suffix, so require it to still exist.
	#[cfg(unix)]
	pub fn cwd(&self) -> Option<std::path::PathBuf> {
		std::fs::read_link(format!("/proc/{}/cwd", self.shell_pid))
			.ok()
			.filter(|dir| dir.is_dir())
	}

	// No /proc equivalent wired up off-unix; callers fall back to the default.
	#[cfg(not(unix))]
	#[allow(clippy::unused_self)]
	pub fn cwd(&self) -> Option<std::path::PathBuf> {
		None
	}

	// Is the shell itself (not a spawned command) the terminal's foreground
	// process? Drives copy-output's command start/end detection. On unix that is
	// the foreground process group: the fg pgid equals the shell's while at the
	// prompt, and a command's while it runs. Windows answers the same question a
	// different way, below.
	#[cfg(unix)]
	pub fn at_shell_prompt(&self) -> bool {
		let pgid = unsafe { libc::tcgetpgrp(self.master_fd) };
		pgid <= 0 || pgid as u32 == self.shell_pid
	}
	// A Windows console has no foreground process group, so the stand-in is "does
	// the shell have a live child?" - measured on this box: a command the shell
	// launches is its DIRECT child and is gone again by the time the prompt
	// returns, while the console host (conhost/OpenConsole) hangs off OUR process,
	// never the shell's. A shell builtin spawns nothing, which reads as "at the
	// prompt" throughout - right, since its output ends when the prompt returns.
	// A background job (PowerShell's Start-Job) does read as a command still
	// running; Windows offers nothing that tells one from a foreground command.
	// The only way to ask is a walk of the whole process table, and that is not
	// cheap - 6.4ms per scan measured here, release and debug alike, across 237
	// processes - so the answer is cached until the terminal next stirs (see
	// note_activity) instead of being taken per event-loop pass. Callers only
	// ask once output has gone quiet, so in practice this is one scan per
	// command rather than one per frame.
	#[cfg(windows)]
	pub fn at_shell_prompt(&self) -> bool {
		if self.shell_pid == 0 {
			return true; // no pid to probe: report "at prompt" (the feature stays inert)
		}
		if let Some(answer) = self.prompt_probe.get() {
			return answer;
		}
		let answer = !has_command_child(self.shell_pid, self.shell_started);
		self.prompt_probe.set(Some(answer));
		answer
	}

	// Windows: the cached at-prompt answer holds only until the terminal next
	// stirs. Both halves matter - anything typed can start a command, and a
	// command that ends always brings the prompt back with it, so its own last
	// act is PTY output. Anywhere else this is nothing.
	#[cfg(windows)]
	pub fn note_activity(&self) {
		self.prompt_probe.set(None);
	}

	#[cfg(not(windows))]
	#[allow(clippy::unused_self)]
	pub fn note_activity(&self) {}

	pub fn write<B: Into<Vec<u8>>>(&self, bytes: B) {
		self.note_activity();
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
	let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
	let comm = comm.trim();
	if comm.is_empty() {
		None
	} else {
		Some(comm.to_string())
	}
}

// Does a process-table row belong to a command this shell launched? Windows
// recycles pids aggressively and a row keeps whatever parent id it was born
// with, so the parent id ALONE can name an unrelated long-lived process whose
// creator's pid the shell later inherited - which would read as a command that
// never finishes and would silently kill copy-output for that pane. A child
// cannot predate its parent, so the start times settle it. An unknown child
// start time (a process we may not open, so never one of ours) answers no; an
// unknown shell start time has nothing to compare against, so the parent id
// stands on its own.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_command_child(child_started: Option<u64>, shell_started: u64) -> bool {
	shell_started == 0 || child_started.is_some_and(|started| started >= shell_started)
}

// Windows: does `shell_pid` have a live child process? Walks the process table -
// there is no narrower query - and stops at the first real child.
#[cfg(windows)]
fn has_command_child(shell_pid: u32, shell_started: u64) -> bool {
	use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
	use windows_sys::Win32::System::Diagnostics::ToolHelp::{
		CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
		TH32CS_SNAPPROCESS,
	};

	let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
	if snapshot == INVALID_HANDLE_VALUE {
		return false; // can't tell: answer "no command", i.e. at the prompt
	}
	let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
	entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0);
	let mut found = false;
	let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) };
	while more != 0 {
		if entry.th32ParentProcessID == shell_pid
			&& is_command_child(process_start_time(entry.th32ProcessID), shell_started)
		{
			found = true;
			break;
		}
		more = unsafe { Process32NextW(snapshot, &raw mut entry) };
	}
	unsafe { CloseHandle(snapshot) };
	found
}

// Windows: a process's creation time as a raw FILETIME, for is_command_child.
// None when the process is gone or can't be opened (a protected/system process,
// which is never a command our shell started).
#[cfg(windows)]
fn process_start_time(pid: u32) -> Option<u64> {
	use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
	use windows_sys::Win32::System::Threading::{
		GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
	};

	let mut created = FILETIME {
		dwLowDateTime: 0,
		dwHighDateTime: 0,
	};
	let mut ignored = [FILETIME {
		dwLowDateTime: 0,
		dwHighDateTime: 0,
	}; 3];
	let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
	if process.is_null() {
		return None;
	}
	let ok = unsafe {
		GetProcessTimes(
			process,
			&raw mut created,
			&raw mut ignored[0],
			&raw mut ignored[1],
			&raw mut ignored[2],
		)
	};
	unsafe { CloseHandle(process) };
	(ok != 0).then(|| u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
}

#[cfg(test)]
mod tests {
	use super::is_command_child;

	// A recycled pid is the failure this guard exists for: the row claims the
	// shell as its parent but started before the shell did, so it belongs to
	// whoever held that pid before.
	#[test]
	fn a_child_that_predates_the_shell_is_not_its_command() {
		assert!(is_command_child(Some(200), 100));
		assert!(is_command_child(Some(100), 100)); // same tick: still a child
		assert!(!is_command_child(Some(50), 100));
		assert!(!is_command_child(None, 100)); // unopenable, so not ours
		assert!(is_command_child(None, 0)); // shell time unknown: parent id alone
	}
}
