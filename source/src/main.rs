// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright (C) 2026 Jim Collier

#![cfg_attr(
	all(target_os = "windows", not(debug_assertions)),
	windows_subsystem = "windows"
)]

mod app;
mod bgimage;
mod cli;
mod clipboard;
mod config;
mod dialog;
mod gfx;
mod glow;
mod input;
mod palette;
mod pane;
mod scroll;
mod settings_ui;
mod sysfont;
mod term;
mod text;
mod theme;

use winit::event_loop::{ControlFlow, EventLoop};

use crate::app::App;
use crate::term::UserEvent;

fn main() -> anyhow::Result<()> {
	env_logger::init();
	alacritty_terminal::tty::setup_env();

	let mut cli = match cli::parse(std::env::args().skip(1)) {
		Ok(c) => c,
		Err(e) => {
			eprintln!("{}: {e}\nTry --help.", config::APP_NAME);
			std::process::exit(2);
		}
	};
	let version = format!("{} {}", config::APP_NAME, env!("CARGO_PKG_VERSION"));
	if cli.help {
		println!("{version}\n\n{}", cli::usage());
		return Ok(());
	}
	if cli.syntax {
		print!("{}", cli::usage());
		return Ok(());
	}
	if cli.version {
		println!("{version}");
		return Ok(());
	}
	if let Some(path) = &cli.config {
		config::set_config_override(path.clone());
	}

	// Launched with no arguments? Fall back to a config-defined command line
	// (real CLI arguments override it entirely).
	if std::env::args().count() <= 1 {
		let cl = config::settings().command_line.clone();
		if !cl.trim().is_empty() {
			match cli::shell_split(&cl).and_then(cli::parse) {
				Ok(c) => cli = c,
				Err(e) => eprintln!("{}: config command_line: {e}", config::APP_NAME),
			}
		}
	}

	let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;
	event_loop.set_control_flow(ControlFlow::Wait);

	let proxy = event_loop.create_proxy();
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
