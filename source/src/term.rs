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
	// the shell's own process ended, with the status it ended on (--keep-open
	// shows this). Always followed by Exit for the same pane.
	ChildExit(PaneId, String),
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
	// shell scan (shells.rs): the stored shell list with whatever the scan found
	// folded in.
	ShellsReady(Vec<crate::shells::Found>),
	// VT watcher thread (app.rs spawn_vt_watch): the active console changed.
	// Linux GL path only; never constructed elsewhere.
	#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
	VtSwitched,
}

// One line to roll the folding back, should a platform ever need every notice
// delivered separately.
const COALESCE_WAKEUPS: bool = true;

// One outstanding "there is new output" notice per pane.
//
// The engine finishes a read cycle roughly every 900 bytes under a flood, and
// each cycle used to become its own window event: measured on 32 MiB of output,
// about 20,000 of them, costing 2.5 SECONDS of main-thread CPU inside the OS
// message pump alone - more than the parsing and the drawing put together, for
// a message that says nothing but "look again".
//
// Nothing is lost by folding them. The notice carries no payload: whenever the
// window gets round to one it reads the grid as it stands, so a queue of twenty
// identical notices produced twenty identical reads. `handled` is cleared BEFORE
// the window acts on it, so a cycle that lands mid-handling posts a fresh notice
// rather than being dropped.
#[derive(Default)]
pub struct WakeGate {
	pending: std::sync::atomic::AtomicBool,
}

impl WakeGate {
	// True when this notice has to be posted (nothing outstanding).
	pub fn post(&self) -> bool {
		!COALESCE_WAKEUPS || !self.pending.swap(true, std::sync::atomic::Ordering::AcqRel)
	}

	pub fn handled(&self) {
		self.pending
			.store(false, std::sync::atomic::Ordering::Release);
	}
}

// bridges alacritty's PTY thread back to the winit loop
#[derive(Clone)]
pub struct EventProxy {
	id: PaneId,
	proxy: EventLoopProxy<UserEvent>,
	wake: Arc<WakeGate>,
}

impl EventProxy {
	pub fn new(id: PaneId, proxy: EventLoopProxy<UserEvent>) -> Self {
		Self {
			id,
			proxy,
			wake: Arc::new(WakeGate::default()),
		}
	}
}

// How a shell's exit reads on screen. Platform Display spellings vary
// ("exit status: 1", "exit code: 1"), so say it ourselves.
fn status_text(status: std::process::ExitStatus) -> String {
	if let Some(code) = status.code() {
		return code.to_string();
	}
	#[cfg(unix)]
	{
		use std::os::unix::process::ExitStatusExt;
		if let Some(sig) = status.signal() {
			return format!("signal {sig}");
		}
	}
	"unknown".into()
}

impl EventListener for EventProxy {
	fn send_event(&self, event: Event) {
		let _ = match event {
			Event::Wakeup if !self.wake.post() => Ok(()), // one notice is enough
			Event::Wakeup => self.proxy.send_event(UserEvent::Wakeup(self.id)),
			Event::Title(t) => self.proxy.send_event(UserEvent::Title(self.id, t)),
			// Empty, not the app name: that is how "the program set no title" is
			// told apart from one it happened to set to our own name.
			Event::ResetTitle => self
				.proxy
				.send_event(UserEvent::Title(self.id, String::new())),
			Event::ChildExit(status) => self
				.proxy
				.send_event(UserEvent::ChildExit(self.id, status_text(status))),
			Event::Exit => self.proxy.send_event(UserEvent::Exit(self.id)),
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

// What a pane's shell is doing, as its tab reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Task {
	/// A command is in the foreground right now.
	Running(String),
	/// Back at the prompt; this is the last command that ran.
	Last(String),
	/// This shell has never run anything.
	Idle,
}

pub struct TermInstance {
	pub term: Arc<FairMutex<Term<EventProxy>>>,
	// same gate the engine's thread posts through, so the window can re-arm it
	notifier: EventProxy,
	pub cols: usize,
	pub lines: usize,
	sender: EventLoopSender,
	io: Option<std::thread::JoinHandle<()>>,
	// where the shell last SAID it is (OSC 7 / OSC 9;9, see cwd.rs) - the only
	// answer for a shell whose location the OS cannot see, PowerShell above all
	reported_cwd: crate::cwd::Reported,
	// for tab titles: the PTY master fd, which names the foreground process group.
	#[cfg(unix)]
	master_fd: std::os::unix::io::RawFd,
	#[cfg(unix)]
	shell_pid: u32,
	// The last command this shell ran, so an idle tab can still say what it was
	// doing. Both platforms answer "what is running" differently and both keep
	// this the same way.
	last_program: Option<String>,
	// throttles the per-frame task probe (see task())
	task_cache: Option<(std::time::Instant, Task)>,
	// windows: the shell's pid and the time it started, for the child-process
	// probe that stands in for a foreground process group (see at_shell_prompt),
	// plus that probe's answer for as long as it holds (see note_activity).
	// Either 0 means "unknown"; the inner Option is the running command's name.
	#[cfg(windows)]
	shell_pid: u32,
	#[cfg(windows)]
	shell_started: u64,
	#[cfg(windows)]
	child_probe: std::cell::RefCell<Option<Option<String>>>,
}

impl TermInstance {
	// The window has taken delivery of an output notice, so the next read cycle
	// posts a fresh one (see WakeGate).
	pub fn wake_handled(&self) {
		self.notifier.wake.handled();
	}

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
		opts.env
			.extend(crate::integration::pane_env(command.as_deref()));
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
		let notifier = event_proxy.clone();
		// the tap goes between the PTY and the parser; the bytes are untouched
		let reported_cwd = crate::cwd::Reported::default();
		let pty = crate::cwd::TappedPty::new(pty, reported_cwd.clone());
		let event_loop = EventLoop::new(term.clone(), event_proxy, pty, false, false)?;
		let sender = event_loop.channel();
		let handle = event_loop.spawn();
		// wrap the join handle so we don't carry its tuple return type around
		let io = std::thread::spawn(move || {
			let _ = handle.join();
		});

		Ok(Self {
			term,
			notifier,
			cols,
			lines,
			sender,
			io: Some(io),
			reported_cwd,
			#[cfg(unix)]
			master_fd,
			#[cfg(unix)]
			shell_pid,
			last_program: None,
			task_cache: None,
			#[cfg(windows)]
			shell_pid,
			#[cfg(windows)]
			shell_started: process_start_time(shell_pid).unwrap_or(0),
			#[cfg(windows)]
			child_probe: std::cell::RefCell::new(None),
		})
	}

	// What this pane's shell is doing, for the tab to say. "Running" is the
	// program the shell has in the foreground right now; "Last" is what it ran
	// most recently, which is what an idle tab reports instead. A shell that has
	// never run anything is Idle, and the tab shows its directory instead.
	//
	// Both platforms answer this, by quite different means - a foreground process
	// group on unix, a live child process on Windows (see at_shell_prompt) - and
	// both are throttled the same way: render asks per tab per frame, and paying
	// for a probe on every idle blink frame added up.
	pub fn task(&mut self) -> Task {
		const PROBE_IVL: std::time::Duration = std::time::Duration::from_millis(250);
		let now = std::time::Instant::now();
		if let Some((at, task)) = &self.task_cache {
			if now.duration_since(*at) < PROBE_IVL {
				return task.clone();
			}
		}
		let task = match self.running_program() {
			Some(program) => {
				self.last_program = Some(program.clone());
				Task::Running(program)
			}
			None => match &self.last_program {
				Some(last) => Task::Last(last.clone()),
				None => Task::Idle,
			},
		};
		self.task_cache = Some((now, task.clone()));
		task
	}

	// The foreground process group is the shell's own while it sits at its prompt,
	// and a command's while one runs - so a pgid that is neither answers None.
	#[cfg(unix)]
	fn running_program(&mut self) -> Option<String> {
		let pgid = unsafe { libc::tcgetpgrp(self.master_fd) };
		if pgid <= 0 || pgid as u32 == self.shell_pid {
			return None;
		}
		proc_comm(pgid as u32)
	}

	#[cfg(windows)]
	fn running_program(&mut self) -> Option<String> {
		self.command_child()
	}

	#[cfg(not(any(unix, windows)))]
	#[allow(clippy::unused_self)]
	fn running_program(&mut self) -> Option<String> {
		None
	}

	// The shell's current directory, for a new tab/split to start in.
	//
	// What the shell SAID wins over what the OS can see, and that order is the
	// point: a shell reporting its directory is answering the question directly,
	// while the OS can only see where the process itself sits - which for
	// PowerShell is the launch directory forever. A report that no longer names
	// a directory (a stale one, or a path on the far side of an ssh) is dropped
	// rather than trusted, and the OS answer stands instead.
	pub fn cwd(&self) -> Option<std::path::PathBuf> {
		self.reported_cwd
			.get()
			.filter(|dir| dir.is_dir())
			.or_else(|| self.os_cwd())
	}

	// Where the OS says the shell process itself is. A deleted dir reads back
	// with a " (deleted)" suffix, so require it to still exist.
	#[cfg(unix)]
	fn os_cwd(&self) -> Option<std::path::PathBuf> {
		std::fs::read_link(format!("/proc/{}/cwd", self.shell_pid))
			.ok()
			.filter(|dir| dir.is_dir())
	}

	// Windows keeps a process's current directory in its own address space
	// rather than anywhere the OS will hand out, so the answer is read from the
	// shell's PEB (see peb_cwd). What that CAN'T see is a shell that keeps its
	// own idea of "where I am" and never tells the OS: measured on this box,
	// PowerShell 7 and Windows PowerShell 5.1 both leave the process directory
	// at the launch directory across a `Set-Location`, so a PowerShell pane
	// reports where it started. cmd.exe, Git Bash and MSYS2 all call
	// SetCurrentDirectory and read back correctly.
	#[cfg(windows)]
	fn os_cwd(&self) -> Option<std::path::PathBuf> {
		peb_cwd(self.shell_pid)
	}

	// Neither /proc nor a PEB: only what a shell reports can answer here.
	#[cfg(not(any(unix, windows)))]
	#[allow(clippy::unused_self)]
	fn os_cwd(&self) -> Option<std::path::PathBuf> {
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
		self.command_child().is_none()
	}

	// The name of the command the shell is running, if any - the one scan answers
	// both questions, so a tab title costs nothing on top of copy-output's.
	#[cfg(windows)]
	fn command_child(&self) -> Option<String> {
		if self.shell_pid == 0 {
			return None; // no pid to probe: report "at prompt" (the feature stays inert)
		}
		if let Some(answer) = self.child_probe.borrow().as_ref() {
			return answer.clone();
		}
		let answer = command_child_name(self.shell_pid, self.shell_started);
		*self.child_probe.borrow_mut() = Some(answer.clone());
		answer
	}

	// Windows: the cached at-prompt answer holds only until the terminal next
	// stirs. Both halves matter - anything typed can start a command, and a
	// command that ends always brings the prompt back with it, so its own last
	// act is PTY output. Anywhere else this is nothing.
	#[cfg(windows)]
	pub fn note_activity(&self) {
		*self.child_probe.borrow_mut() = None;
	}

	#[cfg(not(windows))]
	#[allow(clippy::unused_self)]
	pub fn note_activity(&self) {}

	pub fn write<B: Into<Vec<u8>>>(&self, bytes: B) {
		self.note_activity();
		let _ = self.sender.send(Msg::Input(bytes.into().into()));
	}

	// Put text on screen without a PTY behind it. The engine's thread owns the
	// parser and dies with the shell, so anything added afterwards (the
	// --keep-open exit line) needs a parser of our own.
	pub fn feed(&self, bytes: &[u8]) {
		let mut parser = alacritty_terminal::vte::ansi::Processor::<
			alacritty_terminal::vte::ansi::StdSyncHandler,
		>::default();
		parser.advance(&mut *self.term.lock(), bytes);
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

// Variables the launching shell keeps for ITSELF, which must not ride along
// into a different shell.
//
// A terminal hands its child whatever environment it was launched with, and for
// anything the user exported that is exactly right. A shell's own private
// bookkeeping is not: pwsh 7 PREPENDS its own module directories to
// PSModulePath in its process, so a Windows PowerShell 5.1 pane opened anywhere
// below one resolves PSReadLine to pwsh's copy instead of its own, and cannot
// load it - the 5.1 copy is signed as a Windows OS component and is exempt from
// a Restricted execution policy while the pwsh 7 copy is not, so 5.1 starts
// with "Cannot load PSReadline module." and no line editing. Measured on this
// box; it reproduces in a bare cmd.exe launched from pwsh, with no terminal
// involved at all. PSExecutionPolicyPreference is the same shape - pwsh's
// -ExecutionPolicy sets it and EVERY descendant inherits it, so a pane can run
// under a policy nobody chose for it.
//
// pwsh runs on Linux and macOS too and mutates the same variable there, and a
// side-by-side install (a distro pwsh beside a preview, or a Homebrew one beside
// the .pkg) is the same collision as 5.1-below-7 - so the list is not
// platform-split. OLDPWD earns its place on every platform for a different
// reason: it is the launching shell's own `cd -` target, and a pane opens
// somewhere else entirely, so inheriting it points `cd -` at a directory the
// user was never in.
//
// What is deliberately NOT here: VIRTUAL_ENV and CONDA_*, which a user activates
// and then WANTS a pane to keep (and which cannot be dropped honestly anyway -
// the matching PATH edits would stay, leaving a half-activated environment);
// SHLVL, which is a real nesting count every shell agrees on rather than one
// shell's private state.
//
// A name may only join this list if a desktop session NEVER sets it - see
// session_env below, whose unix arm cannot tell the difference.
const SHELL_PRIVATE_ENV: &[&str] = &["PSModulePath", "PSExecutionPolicyPreference", "OLDPWD"];

// Put the shell-private variables back to what a freshly launched process would
// see, so a pane's shell starts the way it would from the desktop. Everything
// else is left exactly as inherited - discarding the whole environment would
// throw away the user's own exports, which is the one thing inheriting from a
// shell is for.
//
// Called ONCE from main, before any thread exists: an environment write is
// process-global and unsound beside a reader. Doing it to our own environment
// rather than per spawn is what makes it cover every path at once - the first
// pane, a split, a new tab, a new window, the shell scan and the PowerShell the
// profile installer starts.
pub fn sanitize_shell_env() {
	// No answer means leave the environment alone. A pane that starts with a
	// stale variable beats one that starts with none.
	let Some(session) = session_env() else {
		return;
	};
	let inherited: std::collections::HashMap<String, String> = std::env::vars().collect();
	for (name, value) in env_fixups(SHELL_PRIVATE_ENV, &session, &inherited) {
		// SAFETY: single-threaded here - this runs at the top of main, before the
		// event loop, any worker thread or any PTY exists.
		unsafe {
			match value {
				Some(want) => std::env::set_var(&name, want),
				None => std::env::remove_var(&name),
			}
		}
	}
}

// What has to change for each named variable to read the way a freshly launched
// process would see it: Some(value) to set, None to drop (the session never set
// it, so neither should a pane). Pure, and the list is passed in rather than
// read off cfg!, so both platforms' answers are testable from either box - the
// same reason config_base_for takes a Layout.
#[cfg_attr(not(windows), allow(dead_code))]
fn env_fixups(
	names: &[&str],
	session: &std::collections::HashMap<String, String>,
	inherited: &std::collections::HashMap<String, String>,
) -> Vec<(String, Option<String>)> {
	// Windows environment names are case-insensitive, and a block keeps whatever
	// spelling set it first, so the two sides can disagree on case alone.
	let find = |vars: &std::collections::HashMap<String, String>, name: &str| {
		vars.iter()
			.find(|(key, _)| key.eq_ignore_ascii_case(name))
			.map(|(_, value)| value.clone())
	};
	names
		.iter()
		.filter_map(|name| {
			let want = find(session, name);
			// already what it should be (launched from the desktop, say)
			if want == find(inherited, name) {
				return None;
			}
			Some(((*name).to_string(), want))
		})
		.collect()
}

// The environment a freshly launched process would see - machine plus user,
// merged the way the desktop composes it, so it stays right on a box whose
// PowerShell lives somewhere unusual or whose variables come from domain
// policy. NULL as the token yields SYSTEM variables only, so the process token
// is required.
#[cfg(windows)]
fn session_env() -> Option<std::collections::HashMap<String, String>> {
	use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
	use windows_sys::Win32::Security::TOKEN_QUERY;
	use windows_sys::Win32::System::Environment::{
		CreateEnvironmentBlock, DestroyEnvironmentBlock,
	};
	use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

	// SAFETY: plain Win32 calls. The token and the block are each released on
	// every path out, and any failure returns None so the caller stands aside.
	unsafe {
		let mut token: HANDLE = std::ptr::null_mut();
		if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) == 0 {
			return None;
		}
		let mut block: *mut core::ffi::c_void = std::ptr::null_mut();
		let made = CreateEnvironmentBlock(&raw mut block, token, 0);
		CloseHandle(token);
		if made == 0 || block.is_null() {
			return None;
		}
		let raw = parse_env_block(&read_env_block(block.cast::<u16>()));
		DestroyEnvironmentBlock(block);
		// A machine variable is stored as REG_EXPAND_SZ and comes back with its
		// references intact - PSModulePath really is spelled
		// "%ProgramFiles%\\WindowsPowerShell\\Modules" here - so a shell handed one
		// raw would search a directory that does not exist. Windows expands them
		// once when it composes an environment; so does this, against the SESSION
		// block rather than our own, since ours is the thing being replaced.
		let vars = raw
			.iter()
			.map(|(name, value)| (name.clone(), expand_refs(value, &raw)))
			.collect();
		Some(vars)
	}
}

// Unix has no equivalent of CreateEnvironmentBlock - nothing will say what a
// freshly launched program would see, because the answer is composed by PAM, the
// session manager and the login shell between them and is never recorded
// anywhere afterwards. But every name on the list above is one a desktop session
// does not set, so the answer for those IS the empty set, and each of them is
// dropped rather than reset. That is the whole reason the list may only ever
// carry variables a session never sets: a name that a login profile legitimately
// exports would be dropped here rather than restored, and nothing on this
// platform could tell the two apart.
#[cfg(not(windows))]
// The Option is the Windows arm's failure, kept so both arms read the same.
#[allow(clippy::unnecessary_wraps)]
fn session_env() -> Option<std::collections::HashMap<String, String>> {
	Some(std::collections::HashMap::new())
}

// Copy an environment block out of the OS's memory so the parsing above it can
// be an ordinary function over a slice.
#[cfg(windows)]
unsafe fn read_env_block(block: *const u16) -> Vec<u16> {
	let mut len = 0;
	// two NULs in a row close the block
	while unsafe { *block.add(len) } != 0 || unsafe { *block.add(len + 1) } != 0 {
		len += 1;
	}
	unsafe { std::slice::from_raw_parts(block, len + 1) }.to_vec()
}

// One pass of %NAME% substitution, the way Windows expands a REG_EXPAND_SZ when
// it builds an environment: a name it does not know is left standing rather than
// blanked, and a lone % is a literal.
#[cfg_attr(not(windows), allow(dead_code))]
fn expand_refs(value: &str, vars: &std::collections::HashMap<String, String>) -> String {
	let mut out = String::with_capacity(value.len());
	let mut rest = value;
	while let Some(open) = rest.find('%') {
		let (before, tail) = rest.split_at(open);
		out.push_str(before);
		match tail[1..].find('%') {
			Some(len) if len > 0 => {
				let name = &tail[1..=len];
				match vars.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)) {
					Some((_, found)) => out.push_str(found),
					None => out.push_str(&tail[..=len + 1]),
				}
				rest = &tail[len + 2..];
			}
			_ => {
				out.push('%');
				rest = &tail[1..];
			}
		}
	}
	out.push_str(rest);
	out
}

// An environment block is NAME=VALUE runs separated by NUL. A name is never
// empty, so a leading '=' marks one of the hidden per-drive entries Windows
// keeps ("=C:=C:\dir") and the split starts past the first character.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_env_block(block: &[u16]) -> std::collections::HashMap<String, String> {
	block
		.split(|unit| *unit == 0)
		.filter_map(|entry| {
			let text = String::from_utf16_lossy(entry);
			let at = text
				.char_indices()
				.skip(1)
				.find(|(_, ch)| *ch == '=')
				.map(|(idx, _)| idx)?;
			Some((text[..at].to_string(), text[at + 1..].to_string()))
		})
		.collect()
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

// Windows: the name of `shell_pid`'s live child process, if it has one. Walks
// the process table - there is no narrower query - and stops at the first real
// child. The name is the executable's, without its extension, which is the same
// shape unix reports through /proc comm.
#[cfg(windows)]
fn command_child_name(shell_pid: u32, shell_started: u64) -> Option<String> {
	use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
	use windows_sys::Win32::System::Diagnostics::ToolHelp::{
		CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
		TH32CS_SNAPPROCESS,
	};

	let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
	if snapshot == INVALID_HANDLE_VALUE {
		return None; // can't tell: answer "no command", i.e. at the prompt
	}
	let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
	entry.dwSize = u32::try_from(size_of::<PROCESSENTRY32W>()).unwrap_or(0);
	let mut found = None;
	let mut more = unsafe { Process32FirstW(snapshot, &raw mut entry) };
	while more != 0 {
		if entry.th32ParentProcessID == shell_pid
			&& is_command_child(process_start_time(entry.th32ProcessID), shell_started)
		{
			found = Some(exe_display_name(&entry.szExeFile));
			break;
		}
		more = unsafe { Process32NextW(snapshot, &raw mut entry) };
	}
	unsafe { CloseHandle(snapshot) };
	found
}

// A PROCESSENTRY32W name (NUL-padded UTF-16) as the bare program name a tab
// shows: no directory, no extension. An empty or unreadable name still has to
// answer something, or a running command would read as an idle prompt.
#[cfg(windows)]
fn exe_display_name(raw: &[u16]) -> String {
	let end = raw.iter().position(|&c| c == 0).unwrap_or(raw.len());
	let name = String::from_utf16_lossy(&raw[..end]);
	let name = name.rsplit(['\\', '/']).next().unwrap_or(&name);
	let stem = name
		.rfind('.')
		.filter(|dot| *dot > 0)
		.map_or(name, |dot| &name[..dot]);
	if stem.is_empty() {
		"command".to_string()
	} else {
		stem.to_string()
	}
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

// Windows: a process's current directory, out of its own PEB. There is no API
// that answers this for another process - GetCurrentDirectory only ever reports
// the caller's - so the walk is: ProcessBasicInformation for the PEB address,
// then two reads across it. Both offsets are undocumented but have been fixed
// since Vista, and the result is checked with is_dir() before it is believed,
// so a layout that ever did move degrades to "don't know" rather than to a
// wrong directory.
#[cfg(all(windows, target_pointer_width = "64"))]
fn peb_cwd(shell_pid: u32) -> Option<std::path::PathBuf> {
	use std::ffi::c_void;
	use std::os::windows::ffi::OsStringExt;

	use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
	use windows_sys::Win32::Foundation::CloseHandle;
	use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
	use windows_sys::Win32::System::Threading::{
		OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
	};

	// PEB -> RTL_USER_PROCESS_PARAMETERS -> CurrentDirectory.DosPath, 64-bit.
	const PROCESS_PARAMETERS: usize = 0x20;
	const CURRENT_DIRECTORY: usize = 0x38;

	if shell_pid == 0 {
		return None;
	}
	// SAFETY: every call below is a plain FFI call on values this function owns;
	// the handle is closed on every path out, and ReadProcessMemory reports a
	// failure rather than faulting when an address isn't mapped.
	unsafe {
		let process = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, shell_pid);
		if process.is_null() {
			return None;
		}
		let read = |at: usize, into: *mut c_void, len: usize| -> bool {
			let mut got = 0usize;
			at != 0
				&& ReadProcessMemory(process, at as *const c_void, into, len, &raw mut got) != 0
				&& got == len
		};
		let mut basic: ProcessBasicInfo = std::mem::zeroed();
		let status = NtQueryInformationProcess(
			process,
			ProcessBasicInformation,
			(&raw mut basic).cast::<c_void>(),
			u32::try_from(size_of::<ProcessBasicInfo>()).unwrap_or(0),
			std::ptr::null_mut(),
		);
		let peb = usize::try_from(basic.peb).unwrap_or(0);
		let mut params = 0usize;
		let mut dos_path = UnicodeString::default();
		let ok = status == 0
			&& read(
				peb + PROCESS_PARAMETERS,
				(&raw mut params).cast::<c_void>(),
				size_of::<usize>(),
			) && read(
			params + CURRENT_DIRECTORY,
			(&raw mut dos_path).cast::<c_void>(),
			size_of::<UnicodeString>(),
		);
		// Length is in BYTES, and a path is UTF-16 - halve it for the buffer.
		let chars = usize::from(dos_path.length) / 2;
		let mut wide = vec![0u16; chars];
		let ok = ok
			&& chars > 0
			&& read(
				dos_path.buffer as usize,
				wide.as_mut_ptr().cast::<c_void>(),
				chars * 2,
			);
		CloseHandle(process);
		if !ok {
			return None;
		}
		// It comes back with a trailing separator and may name a directory that
		// has since been removed, so it is checked the way the unix side is.
		let dir = std::path::PathBuf::from(std::ffi::OsString::from_wide(&wide));
		dir.is_dir().then_some(dir)
	}
}

// The 32-bit PEB is laid out differently and no 32-bit Windows target is built,
// so it answers "don't know" rather than reading the wrong offsets.
#[cfg(all(windows, not(target_pointer_width = "64")))]
fn peb_cwd(_shell_pid: u32) -> Option<std::path::PathBuf> {
	None
}

// The two structures the walk reads, declared here rather than taken from
// windows-sys because they describe ANOTHER process's memory: what matters is
// the 64-bit layout of the process being read, which is fixed, and declaring
// them costs less than a windows-sys feature pulled in for two fields.
#[cfg(all(windows, target_pointer_width = "64"))]
#[repr(C)]
struct ProcessBasicInfo {
	exit_status: i32,
	peb: u64, // repr(C) pads to the 8-byte alignment, as the real one does
	affinity_mask: usize,
	base_priority: i32,
	unique_pid: usize,
	parent_pid: usize,
}

// The UNICODE_STRING sitting inside RTL_USER_PROCESS_PARAMETERS.
#[cfg(all(windows, target_pointer_width = "64"))]
#[derive(Default)]
#[repr(C)]
struct UnicodeString {
	length: u16,
	capacity: u16,
	_pad: u32,
	buffer: u64,
}

#[cfg(test)]
mod tests {
	use super::{
		SHELL_PRIVATE_ENV, WakeGate, env_fixups, expand_refs, is_command_child, parse_env_block,
	};
	#[cfg(unix)]
	use super::status_text;

	// The --keep-open line reads this out to the user, so it has to say the same
	// thing on every platform.
	#[cfg(unix)]
	#[test]
	fn an_exit_status_reads_the_same_on_every_platform() {
		use std::os::unix::process::ExitStatusExt;
		let st = |raw| status_text(std::process::ExitStatus::from_raw(raw));
		assert_eq!(st(0), "0");
		assert_eq!(st(1 << 8), "1");
		assert_eq!(st(9), "signal 9");
	}

	// Folding the notices may never LOSE one: the window clears the gate before
	// it looks at the grid, so a read cycle that lands mid-handling posts again.
	#[test]
	fn one_notice_stands_until_the_window_takes_it() {
		let gate = WakeGate::default();
		assert!(gate.post()); // nothing outstanding: this one goes
		assert!(!gate.post()); // ... and the next hundred ride on it
		assert!(!gate.post());
		gate.handled();
		assert!(gate.post());
	}

	// Windows has no /proc, so a new tab or split can only inherit a directory
	// if the shell's own PEB can be read - and the point is the directory it is
	// in NOW. A process that never moves must read back where it was started
	// (that half is deterministic), and one that calls SetCurrentDirectory - as
	// cmd.exe's `cd` does - must read back the new place, not the old one.
	#[cfg(windows)]
	#[test]
	fn a_windows_shell_reports_where_it_is_now_not_where_it_started() {
		use std::io::Write;
		use std::process::{Command, Stdio};

		use super::peb_cwd;
		let real = |dir: &std::path::Path| std::fs::canonicalize(dir).expect("canonicalize");
		let started_in = std::env::temp_dir();
		let moved_to = std::path::PathBuf::from(r"C:\Windows");

		// Each shell is held open on its own stdin pipe, and runs NOTHING. That is
		// load-bearing rather than tidy: `Child::kill` is TerminateProcess, which
		// ends one process and never its tree, so the old `cmd /c ping ...` left the
		// ping behind every time this test ran. They do not exit on their own either
		// (measured: alive for days at 0% CPU, 2 MB apiece), so they accumulate one
		// or two per `cargo test` - fifteen had piled up before anyone counted. A
		// cmd.exe waiting on a pipe is the same shell doing the same `cd` with no
		// grandchild under it to strand.
		let hold = |dir: &std::path::Path| {
			Command::new("cmd.exe")
				.arg("/k")
				.current_dir(dir)
				.stdin(Stdio::piped())
				.stdout(Stdio::null())
				.stderr(Stdio::null())
				.spawn()
				.expect("spawn cmd.exe")
		};
		// one that stays put, for the plain "did we read the right one"
		let mut still = hold(&started_in);
		// and one that moves, for the half that matters - told to down its own stdin
		let mut roams = hold(&started_in);
		if let Some(pipe) = roams.stdin.as_mut() {
			let _ = writeln!(pipe, "cd /d {}", moved_to.display());
			let _ = pipe.flush();
		}

		// the move takes a moment to happen; nothing else here waits on a clock
		let mut roamed = None;
		for _ in 0..60 {
			roamed = peb_cwd(roams.id());
			if roamed
				.as_deref()
				.map(std::path::Path::to_path_buf)
				.map(|dir| real(&dir))
				== Some(real(&moved_to))
			{
				break;
			}
			std::thread::sleep(std::time::Duration::from_millis(50));
		}
		let stayed = peb_cwd(still.id());
		// killed BEFORE the assertions, so a failing assert still cleans up after
		// itself - these two wait forever otherwise, being fed by a pipe
		let _ = still.kill();
		let _ = roams.kill();
		// reaped, or the test leaves two zombies behind it
		let _ = still.wait();
		let _ = roams.wait();

		assert_eq!(
			stayed.map(|dir| real(&dir)),
			Some(real(&started_in)),
			"a process that never moved read back somewhere else"
		);
		assert_eq!(
			roamed.map(|dir| real(&dir)),
			Some(real(&moved_to)),
			"the directory the shell moved to never came back"
		);
	}

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

	fn vars(pairs: &[(&str, &str)]) -> std::collections::HashMap<String, String> {
		pairs
			.iter()
			.map(|(name, value)| ((*name).to_string(), (*value).to_string()))
			.collect()
	}

	// A shell's private bookkeeping must not decide how a DIFFERENT shell starts.
	// pwsh 7 prepends its own module directories to PSModulePath in its process,
	// so a Windows PowerShell 5.1 pane opened below one finds pwsh's PSReadLine
	// ahead of its own, cannot load it, and starts with no line editing.
	#[test]
	fn a_shell_private_variable_is_put_back_to_the_session_value() {
		let session = vars(&[(
			"PSModulePath",
			r"C:\Program Files\WindowsPowerShell\Modules",
		)]);
		let inherited = vars(&[(
			"PSModulePath",
			r"C:\Program Files\PowerShell\7\Modules;C:\Program Files\WindowsPowerShell\Modules",
		)]);
		assert_eq!(
			env_fixups(&["PSModulePath"], &session, &inherited),
			vec![(
				"PSModulePath".to_string(),
				Some(r"C:\Program Files\WindowsPowerShell\Modules".to_string()),
			)]
		);
	}

	// pwsh's -ExecutionPolicy sets this and every descendant inherits it. The
	// session never sets it, so neither should a pane - and that means DROPPING
	// it, not handing the shell an empty one to read.
	#[test]
	fn a_variable_the_session_never_set_is_dropped() {
		let session = vars(&[("PATH", r"C:\bin")]);
		let inherited = vars(&[("PSExecutionPolicyPreference", "Bypass")]);
		assert_eq!(
			env_fixups(&["PSExecutionPolicyPreference"], &session, &inherited),
			vec![("PSExecutionPolicyPreference".to_string(), None)]
		);
	}

	// Launched from the desktop rather than from a shell, the inherited
	// environment already IS the session one. That is the ordinary case and it
	// must write nothing at all.
	#[test]
	fn an_environment_that_already_matches_needs_no_fixups() {
		let session = vars(&[("PSModulePath", "one;two")]);
		let inherited = vars(&[("PSModulePath", "one;two")]);
		assert!(env_fixups(&["PSModulePath"], &session, &inherited).is_empty());
	}

	// Everything the user exported themselves is the reason a pane inherits at
	// all, so a variable off the list is left alone however far it has drifted.
	#[test]
	fn only_the_named_variables_are_touched() {
		let session = vars(&[("VIRTUAL_ENV", ""), ("PSModulePath", "one")]);
		let inherited = vars(&[("VIRTUAL_ENV", "/home/me/venv"), ("PSModulePath", "one")]);
		assert!(env_fixups(&["PSModulePath"], &session, &inherited).is_empty());
	}

	// Windows environment names are case-insensitive and a block keeps whichever
	// spelling set it first, so the two sides can differ by case alone - which is
	// not a difference and must not read as one.
	#[test]
	fn a_name_that_differs_only_in_case_is_the_same_variable() {
		let session = vars(&[("PSModulePath", "one")]);
		let inherited = vars(&[("PSMODULEPATH", "one")]);
		assert!(env_fixups(&["PSModulePath"], &session, &inherited).is_empty());
	}

	// The block is NUL-separated NAME=VALUE closed by a second NUL, and it also
	// carries the hidden per-drive entries Windows keeps, whose NAME begins with
	// '=' - so the separator can never be the first character.
	#[test]
	fn an_environment_block_splits_names_from_values() {
		let mut block: Vec<u16> = Vec::new();
		for entry in [r"=C:=C:\work", "PSModulePath=one;two", r"Path=C:\bin"] {
			block.extend(entry.encode_utf16());
			block.push(0);
		}
		block.push(0);

		let found = parse_env_block(&block);
		assert_eq!(
			found.get("PSModulePath").map(String::as_str),
			Some("one;two")
		);
		assert_eq!(found.get("Path").map(String::as_str), Some(r"C:\bin"));
		assert_eq!(found.get("=C:").map(String::as_str), Some(r"C:\work"));
	}

	// PSModulePath is stored as REG_EXPAND_SZ, so the session block hands it over
	// spelled with its references intact. A shell given that raw would search a
	// directory that does not exist - and an unknown name has to survive rather
	// than collapse to nothing, which would silently shorten a search path.
	#[test]
	fn a_stored_reference_expands_and_an_unknown_one_survives() {
		let vars = vars(&[("ProgramFiles", r"C:\Program Files")]);
		assert_eq!(
			expand_refs(r"%ProgramFiles%\WindowsPowerShell\Modules", &vars),
			r"C:\Program Files\WindowsPowerShell\Modules"
		);
		assert_eq!(expand_refs("%NotAThing%;tail", &vars), "%NotAThing%;tail");
		assert_eq!(expand_refs("100% done", &vars), "100% done");
	}

	// The unix arm has no way to ask what a freshly launched program would see, so
	// it answers with the empty set and every listed variable is DROPPED. That is
	// only honest because nothing on the list is a variable a session sets - and it
	// is the same path Windows takes for a variable its session block lacks.
	#[test]
	fn an_empty_session_drops_every_private_variable() {
		let inherited = vars(&[
			("PSModulePath", "/opt/microsoft/powershell/7/Modules"),
			("PSExecutionPolicyPreference", "Bypass"),
			("OLDPWD", "/home/me/elsewhere"),
			("PATH", "/usr/bin"),
		]);
		let fixups = env_fixups(SHELL_PRIVATE_ENV, &vars(&[]), &inherited);
		assert_eq!(fixups.len(), SHELL_PRIVATE_ENV.len());
		assert!(fixups.iter().all(|(_, value)| value.is_none()));
		// and the one nobody asked about is untouched
		assert!(!fixups.iter().any(|(name, _)| name == "PATH"));
	}
}
