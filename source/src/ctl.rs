// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Control socket: each running instance listens on a per-process Unix socket
//! and exports its path to child shells via `SILKTERM_SOCKET`, so `silkterm
//! --wallpaper`/`--reload-settings` run from a shell inside a window reaches
//! exactly that window's process. Protocol: one text line per connection
//! (`reload`, or `wallpaper` + optional tab + path), reply `ok` / `err <msg>`.
//! Unix only for now (std has no `AF_UNIX` on Windows).

use std::path::PathBuf;

use winit::event_loop::EventLoopProxy;

use crate::term::UserEvent;

#[cfg_attr(not(unix), allow(dead_code))] // ctl is Unix-only (AF_UNIX)
pub const ENV_SOCK: &str = "SILKTERM_SOCKET";

// Holds the socket path so the file goes away with the process.
#[cfg(unix)]
pub struct CtlServer {
	path: PathBuf,
}

#[cfg(unix)]
impl Drop for CtlServer {
	fn drop(&mut self) {
		let _ = std::fs::remove_file(&self.path);
	}
}

#[cfg(not(unix))]
pub struct CtlServer;

// Bind the socket, export SILKTERM_SOCKET, and serve commands on a background
// thread. Call before any PTY spawns so shells inherit the variable. Failure is
// non-fatal (the app just isn't remotely controllable).
#[cfg(unix)]
pub fn serve(proxy: EventLoopProxy<UserEvent>) -> Option<CtlServer> {
	use std::io::{BufRead, BufReader, Write};

	let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
	let path = dir.join(format!("silkterm-ctl-{}.sock", std::process::id()));
	let _ = std::fs::remove_file(&path); // stale leftover from a recycled pid
	let listener = match std::os::unix::net::UnixListener::bind(&path) {
		Ok(listener) => listener,
		Err(e) => {
			eprintln!("{}: control socket: {e}", crate::config::APP_NAME);
			return None;
		}
	};
	// Sound here: no PTY or render thread exists yet (set_var is unsafe under
	// edition 2024 because of concurrent readers).
	unsafe { std::env::set_var(ENV_SOCK, &path) };
	std::thread::spawn(move || {
		for stream in listener.incoming() {
			let Ok(stream) = stream else { continue };
			let mut reader = BufReader::new(stream);
			let mut line = String::new();
			if reader.read_line(&mut line).is_err() {
				continue;
			}
			let reply = match parse(line.trim_end()) {
				Ok(event) => {
					if proxy.send_event(event).is_err() {
						return; // event loop gone; the process is exiting
					}
					"ok\n".to_string()
				}
				Err(e) => format!("err {e}\n"),
			};
			let mut stream = reader.into_inner();
			let _ = stream.write_all(reply.as_bytes());
		}
	});
	Some(CtlServer { path })
}

#[cfg(not(unix))]
pub fn serve(_proxy: EventLoopProxy<UserEvent>) -> Option<CtlServer> {
	None
}

// One command line -> the event the app applies. Tab separates verb from value
// so paths with spaces survive.
#[cfg_attr(not(unix), allow(dead_code))]
fn parse(line: &str) -> Result<UserEvent, String> {
	let (verb, value) = match line.split_once('\t') {
		Some((verb, rest)) => (verb, Some(rest)),
		None => (line, None),
	};
	match verb {
		"reload" => Ok(UserEvent::ReloadSettings),
		"wallpaper" => Ok(UserEvent::SetWallpaper(value.map(PathBuf::from))),
		_ => Err(format!("unknown command: {verb}")),
	}
}

// Client side: deliver one command to the window this shell runs inside.
#[cfg(unix)]
pub fn send(cmd: &str) -> Result<(), String> {
	use std::io::{Read, Write};

	let sock =
		std::env::var(ENV_SOCK).map_err(|_| "not inside a running SilkTerm window".to_string())?;
	let mut stream =
		std::os::unix::net::UnixStream::connect(&sock).map_err(|e| format!("{sock}: {e}"))?;
	stream
		.write_all(cmd.as_bytes())
		.and_then(|()| stream.write_all(b"\n"))
		.map_err(|e| e.to_string())?;
	let _ = stream.shutdown(std::net::Shutdown::Write);
	let mut reply = String::new();
	stream
		.read_to_string(&mut reply)
		.map_err(|e| e.to_string())?;
	let reply = reply.trim();
	match reply.strip_prefix("err ") {
		Some(e) => Err(e.to_string()),
		None if reply == "ok" => Ok(()),
		None => Err(format!("unexpected reply: {reply}")),
	}
}

#[cfg(not(unix))]
pub fn send(_cmd: &str) -> Result<(), String> {
	Err("control commands aren't supported on this platform yet".into())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_commands() {
		assert!(matches!(parse("reload"), Ok(UserEvent::ReloadSettings)));
		match parse("wallpaper\t/a dir/pic 1.png") {
			Ok(UserEvent::SetWallpaper(Some(p))) => {
				assert_eq!(p, PathBuf::from("/a dir/pic 1.png"));
			}
			other => panic!("{other:?}"),
		}
		assert!(matches!(
			parse("wallpaper"),
			Ok(UserEvent::SetWallpaper(None))
		));
		assert!(parse("bogus").is_err());
		assert!(parse("").is_err());
	}
}
