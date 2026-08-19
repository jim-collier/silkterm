// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

#![cfg_attr(
	all(target_os = "windows", not(debug_assertions)),
	windows_subsystem = "windows"
)]

mod app;
mod bgimage;
mod cli;
mod clipboard;
mod coloremoji;
mod config;
mod contrast;
mod ctl;
mod dialog;
mod gfx;
mod input;
mod links;
mod palette;
mod pane;
mod perf;
mod scrim;
mod scroll;
mod settings_ui;
mod shells;
mod sysfont;
mod term;
mod text;
mod theme;
mod ui_spec;
mod wallpaper;
mod xmp;

use winit::event_loop::{ControlFlow, EventLoop};

use crate::app::App;
use crate::term::UserEvent;

// Make stdout/stderr reach the terminal we were launched from.
//
// A Windows release build is GUI-subsystem (see the attribute at the top of this
// file), so the loader gives it no console and a plain println! from a CLI-only
// flag lands NOWHERE - measured: run from a real console, the output simply never
// appears, while the same command through a pipe works, which is what makes this
// so easy to miss. Joining the parent's console fixes it.
//
// Called only on the paths that print and exit. NOT on the normal launch path: a
// terminal window that owns a console would die with the shell that started it.
// Everywhere else this is a no-op (already have one, or nothing to join).
fn open_console() {
	#[cfg(windows)]
	// SAFETY: a plain Win32 call taking a constant; failure is reported by the
	// return value, which we have nothing useful to do about.
	unsafe {
		windows_sys::Win32::System::Console::AttachConsole(
			windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
		);
	}
}

fn main() -> anyhow::Result<()> {
	env_logger::init();
	alacritty_terminal::tty::setup_env();

	let mut cli = match cli::parse(std::env::args().skip(1)) {
		Ok(parsed) => parsed,
		Err(e) => {
			open_console();
			eprintln!("{}: {e}\nTry --help.", config::APP_NAME);
			std::process::exit(2);
		}
	};
	// CLI-only flags: print and exit, before anything reads a config or opens a
	// window. All but --version are padded with a blank line either side so the
	// block stands clear of the prompts above and below it; --version stays flush
	// because its job is to be captured.
	if cli.help || cli.syntax || cli.about || cli.donate || cli.version {
		open_console();
		if cli.help {
			print!(
				"{}",
				cli::padded(&format!("{}\n\n{}", cli::version_line(), cli::usage()))
			);
		} else if cli.syntax {
			print!("{}", cli::padded(cli::usage()));
		} else if cli.about {
			print!(
				"{}",
				cli::padded(&cli::about(gfx::probe_adapter_info().as_ref()))
			);
		} else if cli.donate {
			print!("{}", cli::padded(&cli::donate()));
		} else {
			println!("{}", cli::version_line());
		}
		return Ok(());
	}
	// Control commands: talk to the already-running window this shell lives in
	// (via SILKTERM_SOCKET), then exit - nothing here launches a window. Reload
	// first so --reload-settings --wallpaper x ends with x applied.
	if cli.reload || cli.wallpaper.is_some() {
		let mut cmds: Vec<String> = Vec::new();
		if cli.reload {
			cmds.push("reload".into());
		}
		if let Some(img) = &cli.wallpaper {
			cmds.push(match img {
				// resolve against this shell's cwd; the window's cwd differs
				Some(p) => match std::fs::canonicalize(p) {
					Ok(abs) => format!("wallpaper\t{}", abs.display()),
					Err(e) => {
						eprintln!("{}: --wallpaper {p}: {e}", config::APP_NAME);
						std::process::exit(2);
					}
				},
				None => "wallpaper".into(),
			});
		}
		for cmd in &cmds {
			if let Err(e) = ctl::send(cmd) {
				eprintln!("{}: {e}", config::APP_NAME);
				std::process::exit(2);
			}
		}
		return Ok(());
	}

	if let Some(path) = &cli.config {
		config::set_config_override(path.clone());
	}

	// Start over from the shipped defaults: move the current config aside before
	// anything reads it, so the load below writes a fresh one. Runs after --config
	// so the two combine (reset THAT file).
	if cli.reset_config {
		match config::reset_config() {
			Some(backup) => println!(
				"{}: previous config saved as {}",
				config::APP_NAME,
				backup.display()
			),
			None => println!("{}: no config to reset", config::APP_NAME),
		}
	}

	// Launched with no layout arguments? Fall back to a config-defined command
	// line (real CLI arguments override it entirely). A bare --config still takes
	// the fallback - it picks WHICH config, so that config's command_line applies.
	if cli::only_config_args(std::env::args().skip(1)) {
		let command_line = config::settings().command_line.clone();
		if !command_line.trim().is_empty() {
			match cli::shell_split(&command_line).and_then(cli::parse) {
				Ok(parsed) => cli = parsed,
				Err(e) => eprintln!("{}: config command_line: {e}", config::APP_NAME),
			}
		}
	}

	let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
	event_loop.set_control_flow(ControlFlow::Wait);

	let proxy = event_loop.create_proxy();
	// control socket up before any PTY spawns, so shells inherit SILKTERM_SOCKET
	let _ctl = ctl::serve(proxy.clone());
	let mut app = App::new(proxy, cli);

	// cicd profiler stage: SILK_PROFILE_OUT set -> sample this run and write a
	// flamegraph SVG when the app exits (App exits itself after SILK_PROFILE_SECS).
	#[cfg(feature = "profiling")]
	let profile_guard = std::env::var("SILK_PROFILE_OUT").ok().map(|_| {
		pprof::ProfilerGuardBuilder::default()
			.frequency(199)
			.blocklist(&["libc", "libpthread", "vdso", "libgcc"])
			.build()
			.expect("pprof: failed to start profiler")
	});

	event_loop.run_app(&mut app)?;

	#[cfg(feature = "profiling")]
	if let Some(guard) = profile_guard {
		let out = std::env::var("SILK_PROFILE_OUT").unwrap();
		let report = guard
			.report()
			.build()
			.expect("pprof: failed to build report");
		let file = std::fs::File::create(&out).expect("pprof: failed to create SVG");
		report
			.flamegraph(file)
			.expect("pprof: failed to write flamegraph");
		eprintln!("{}: wrote flamegraph -> {out}", config::APP_NAME);
	}

	Ok(())
}
