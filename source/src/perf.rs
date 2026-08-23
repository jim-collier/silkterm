// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Where a burst of output actually goes (`SILK_PERF=1`).
//!
//! Throughput on Windows reads as one number - "it takes N seconds to swallow a
//! large `cat`" - and that number is the sum of three parties: the console host
//! delivering bytes, the engine parsing them, and this program reacting to each
//! batch. Only the last is ours to change, and it is invisible from outside the
//! process: a wall-clock stopwatch cannot say whether the time went into the
//! grid or into waking the event loop forty thousand times.
//!
//! The counters are plain atomics behind one cached flag, so an ordinary run
//! pays a relaxed load per site and nothing else. The report goes to stderr when
//! the loop exits.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::time::Instant;

pub fn on() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("SILK_PERF").is_some())
}

macro_rules! counters {
	($($name:ident),* $(,)?) => {
		$(pub static $name: AtomicU64 = AtomicU64::new(0);)*
	};
}

counters!(
	WAKEUPS,    // output notices delivered to the window (UserEvent::Wakeup)
	PASSES,     // about_to_wait passes (one per event-loop iteration)
	FRAMES,     // frames actually rendered
	BUILD_NS,   // Pane::build, summed over panes
	RENDER_NS,  // State::render, whole frame including build
	NOTE_NS,    // per-wakeup bookkeeping (note_output/note_history)
	LOCK_MISS,  // builds that gave up on the term lock
	PREP_NS,    // glyphon prepare (text + scrim source)
	ACQUIRE_NS, // waiting for a swapchain image
	ENCODE_NS,  // recording the passes
	SUBMIT_NS,  // submit + present
	EVENT_NS,   // our user_event handler, whole
	PASS_NS,    // our about_to_wait, whole
);

pub fn add(counter: &AtomicU64, n: u64) {
	if on() {
		counter.fetch_add(n, Relaxed);
	}
}

pub fn bump(counter: &AtomicU64) {
	add(counter, 1);
}

// Start of a stretch that can't be a scope (an early return in the middle, or a
// span that ends in another block). None when the counters are off, so a normal
// run doesn't even read the clock.
pub fn mark() -> Option<Instant> {
	on().then(Instant::now)
}

pub fn since(counter: &AtomicU64, mark: Option<Instant>) {
	if let Some(start) = mark {
		counter.fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
	}
}

// Time a block into a counter. Cheap enough to leave in the hot path: without
// the flag it is a load and a branch, no clock read at all.
pub fn timed<T>(counter: &AtomicU64, body: impl FnOnce() -> T) -> T {
	if !on() {
		return body();
	}
	let start = Instant::now();
	let out = body();
	counter.fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
	out
}

// CPU seconds this thread (the window/render thread) and the whole process have
// burned. The split is the point: a frame that BLOCKS on the display costs wall
// time and no CPU, so a wall-clock render figure on its own cannot say whether
// the main thread is working or waiting.
#[cfg(windows)]
#[allow(clippy::unnecessary_wraps)] // the other platforms answer None
fn cpu_seconds() -> Option<(f64, f64)> {
	use std::ptr::from_mut;
	use windows_sys::Win32::Foundation::FILETIME;
	use windows_sys::Win32::System::Threading::{
		GetCurrentProcess, GetCurrentThread, GetProcessTimes, GetThreadTimes,
	};

	let secs = |t: FILETIME| {
		((u64::from(t.dwHighDateTime) << 32) | u64::from(t.dwLowDateTime)) as f64 / 1e7
	};
	let zero = FILETIME {
		dwLowDateTime: 0,
		dwHighDateTime: 0,
	};
	let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
	let (created, exited, kernel, user) = (
		from_mut(&mut created),
		from_mut(&mut exited),
		from_mut(&mut kernel),
		from_mut(&mut user),
	);
	// SAFETY: plain Win32 calls on pseudo-handles; every output is owned here.
	unsafe {
		GetThreadTimes(GetCurrentThread(), created, exited, kernel, user);
		let thread = secs(*kernel) + secs(*user);
		GetProcessTimes(GetCurrentProcess(), created, exited, kernel, user);
		Some((thread, secs(*kernel) + secs(*user)))
	}
}

// Nothing equivalent is wired up elsewhere yet, and a pair of zeroes would read
// as "this thread did nothing" rather than "not measured here".
#[cfg(not(windows))]
fn cpu_seconds() -> Option<(f64, f64)> {
	None
}

// Scope timer for a whole function, where wrapping the body in a closure would
// fight the borrow checker.
pub struct Span<'a> {
	counter: &'a AtomicU64,
	start: Option<Instant>,
}

impl<'a> Span<'a> {
	pub fn new(counter: &'a AtomicU64) -> Self {
		Self {
			counter,
			start: on().then(Instant::now),
		}
	}
}

impl Drop for Span<'_> {
	fn drop(&mut self) {
		if let Some(start) = self.start {
			self.counter
				.fetch_add(start.elapsed().as_nanos() as u64, Relaxed);
		}
	}
}

pub fn report() {
	if !on() {
		return;
	}
	let cpu = cpu_seconds();
	let ms = |counter: &AtomicU64| counter.load(Relaxed) as f64 / 1e6;
	eprintln!(
		"[perf] wakeups {} passes {} frames {} | render {:.0}ms (build {:.0}ms) note {:.0}ms | lock misses {}",
		WAKEUPS.load(Relaxed),
		PASSES.load(Relaxed),
		FRAMES.load(Relaxed),
		ms(&RENDER_NS),
		ms(&BUILD_NS),
		ms(&NOTE_NS),
		LOCK_MISS.load(Relaxed),
	);
	eprintln!(
		"[perf] our handlers: user_event {:.0}ms about_to_wait {:.0}ms",
		ms(&EVENT_NS),
		ms(&PASS_NS),
	);
	eprintln!(
		"[perf] frame parts: prepare {:.0}ms acquire {:.0}ms encode {:.0}ms submit+present {:.0}ms",
		ms(&PREP_NS),
		ms(&ACQUIRE_NS),
		ms(&ENCODE_NS),
		ms(&SUBMIT_NS),
	);
	if let Some((thread_cpu, process_cpu)) = cpu {
		eprintln!("[perf] cpu: this thread {thread_cpu:.2}s of process {process_cpu:.2}s");
	}
}
