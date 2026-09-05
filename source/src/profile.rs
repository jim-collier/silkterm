// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

//! Performance profiles: one setting that decides how much the look is allowed
//! to cost, and the rating that picks it on a machine that cannot keep up.
//!
//! A profile sits ON TOP of the stored settings rather than in them. The file
//! and the dialog keep the user's own values; `apply` overwrites the fields a
//! profile governs when settings go live and keeps the originals in a shadow,
//! so any code that reads the live settings and writes them back cannot leak
//! a profile's value into the file. Choosing Custom is then just a profile
//! that governs nothing.
//!
//! The rating watches how a scroll ease is paced. A display that keeps its
//! refresh rate paces one frame per refresh; one that cannot stretches every
//! frame, and a run of stretched frames steps the profile down. It never steps
//! up on the same hardware: a lighter profile renders less, so a fast-looking
//! run under it says nothing about the heavier one.

use crate::config::Settings;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Profile {
	Custom,
	Max,
	High,
	Low,
	Standard,
	// Standard's values, chosen for a remote screen and never written down:
	// it lives in `Settings::remote_override`, not in `performance_profile`
	Remote,
}

impl Profile {
	// dialog order, which is also the order they cost in
	pub const ALL: [Profile; 6] = [
		Profile::Custom,
		Profile::Max,
		Profile::High,
		Profile::Low,
		Profile::Standard,
		Profile::Remote,
	];

	// the spelling the config file uses
	pub fn key(self) -> &'static str {
		match self {
			Profile::Custom => "custom",
			Profile::Max => "max",
			Profile::High => "high",
			Profile::Low => "low",
			Profile::Standard => "standard",
			Profile::Remote => "remote",
		}
	}

	pub fn label(self) -> &'static str {
		match self {
			Profile::Custom => "Custom",
			Profile::Max => "Max silk",
			Profile::High => "High",
			Profile::Low => "Low",
			Profile::Standard => "Standard terminal",
			Profile::Remote => "Remote (temporary)",
		}
	}

	// An unknown spelling reads as the shipped default, the way every other
	// named option in the config does.
	pub fn parse(text: &str) -> Profile {
		Profile::ALL
			.into_iter()
			.find(|p| p.key().eq_ignore_ascii_case(text.trim()))
			.unwrap_or(Profile::Max)
	}

	pub fn index(self) -> usize {
		Profile::ALL.iter().position(|p| *p == self).unwrap_or(0)
	}

	pub fn from_index(index: usize) -> Profile {
		Profile::ALL.get(index).copied().unwrap_or(Profile::Max)
	}

	// The next cheaper profile, or None at the bottom. Custom has no neighbor:
	// the user's own values are not on the ladder.
	pub fn lower(self) -> Option<Profile> {
		match self {
			Profile::Max => Some(Profile::High),
			Profile::High => Some(Profile::Low),
			Profile::Low => Some(Profile::Standard),
			Profile::Standard | Profile::Remote | Profile::Custom => None,
		}
	}
}

// The user's own values of every field a profile governs, kept beside the
// live settings while a profile is in force. `put` is the whole of "choose
// Custom and everything comes back".
#[derive(Clone, PartialEq, Debug)]
pub struct Shadow {
	scroll_smooth: bool,
	scroll_ease_in_ms: f32,
	scroll_ramp_up_ms: f32,
	scroll_single_screen_tau_ms: f32,
	scroll_ramp_down_ms: f32,
	scroll_ease_out_ms: f32,
	smooth_scroll_apps: bool,
	cursor_animation: String,
	text_scrim: bool,
	text_scrim_radius: f32,
	text_scrim_strength: f32,
	text_scrim_softness: f32,
	text_scrim_function: String,
	text_outline: f32,
	wallpaper_enabled: bool,
	wallpaper_blur: f32,
	wallpaper_contrast_mask: bool,
}

impl Shadow {
	fn of(s: &Settings) -> Shadow {
		Shadow {
			scroll_smooth: s.scroll_smooth,
			scroll_ease_in_ms: s.scroll_ease_in_ms,
			scroll_ramp_up_ms: s.scroll_ramp_up_ms,
			scroll_single_screen_tau_ms: s.scroll_single_screen_tau_ms,
			scroll_ramp_down_ms: s.scroll_ramp_down_ms,
			scroll_ease_out_ms: s.scroll_ease_out_ms,
			smooth_scroll_apps: s.smooth_scroll_apps,
			cursor_animation: s.cursor_animation.clone(),
			text_scrim: s.text_scrim,
			text_scrim_radius: s.text_scrim_radius,
			text_scrim_strength: s.text_scrim_strength,
			text_scrim_softness: s.text_scrim_softness,
			text_scrim_function: s.text_scrim_function.clone(),
			text_outline: s.text_outline,
			wallpaper_enabled: s.wallpaper_enabled,
			wallpaper_blur: s.wallpaper_blur,
			wallpaper_contrast_mask: s.wallpaper_contrast_mask,
		}
	}

	fn put(&self, s: &mut Settings) {
		s.scroll_smooth = self.scroll_smooth;
		s.scroll_ease_in_ms = self.scroll_ease_in_ms;
		s.scroll_ramp_up_ms = self.scroll_ramp_up_ms;
		s.scroll_single_screen_tau_ms = self.scroll_single_screen_tau_ms;
		s.scroll_ramp_down_ms = self.scroll_ramp_down_ms;
		s.scroll_ease_out_ms = self.scroll_ease_out_ms;
		s.smooth_scroll_apps = self.smooth_scroll_apps;
		s.cursor_animation.clone_from(&self.cursor_animation);
		s.text_scrim = self.text_scrim;
		s.text_scrim_radius = self.text_scrim_radius;
		s.text_scrim_strength = self.text_scrim_strength;
		s.text_scrim_softness = self.text_scrim_softness;
		s.text_scrim_function.clone_from(&self.text_scrim_function);
		s.text_outline = self.text_outline;
		s.wallpaper_enabled = self.wallpaper_enabled;
		s.wallpaper_blur = self.wallpaper_blur;
		s.wallpaper_contrast_mask = self.wallpaper_contrast_mask;
	}
}

// Put the user's own values back. Safe on settings that carry no profile.
pub fn unapply(s: &mut Settings) {
	if let Some(shadow) = s.profile_shadow.take() {
		shadow.put(s);
	}
}

// Overwrite the governed fields with the profile's, keeping the user's values
// in the shadow. Idempotent: a live copy that already carries a profile is
// unwound first, so a changed profile field is honored rather than stacked.
pub fn apply(s: &mut Settings) {
	unapply(s);
	let profile = current(s);
	if profile == Profile::Custom {
		return;
	}
	let shadow = Shadow::of(s);
	values(profile, s);
	s.profile_shadow = Some(Box::new(shadow));
}

// The profile in force: the remote override while it is on, else the stored one.
pub fn current(s: &Settings) -> Profile {
	if s.remote_override {
		Profile::Remote
	} else {
		Profile::parse(&s.performance_profile)
	}
}

// What each profile sets. Every profile starts from the shipped defaults, so
// Max is exactly "the defaults for everything" and the others name only what
// they change.
fn values(profile: Profile, s: &mut Settings) {
	let defaults = Settings::default();
	Shadow::of(&defaults).put(s);
	match profile {
		Profile::Custom | Profile::Max => {}
		Profile::High => quicker(s),
		// the wallpaper is decoded once and costs nothing per frame, so Low keeps
		// it and drops the halo, which is paid on every frame
		Profile::Low => {
			quicker(s);
			s.cursor_animation = "none".to_string();
			s.text_scrim = false;
			s.text_outline = 2.0;
		}
		Profile::Standard | Profile::Remote => {
			s.scroll_smooth = false;
			s.smooth_scroll_apps = false;
			s.cursor_animation = "none".to_string();
			s.text_scrim = false;
			s.text_outline = 0.0;
			s.wallpaper_enabled = false;
			s.wallpaper_blur = 0.0;
			s.wallpaper_contrast_mask = false;
		}
	}
}

// Shorter eases on the three stretches a slow display shows most, and a halo
// that costs fewer taps: the square metric with a smaller reach.
fn quicker(s: &mut Settings) {
	s.scroll_ease_in_ms /= 2.0;
	s.scroll_ease_out_ms /= 2.0;
	s.scroll_single_screen_tau_ms /= 2.0;
	s.text_scrim_function = "dilate".to_string();
	s.text_scrim_radius = 3.0;
}

// Names the adapter closely enough that a new card or a switch to software
// rendering reads as new hardware, and a driver update does not.
pub fn fingerprint(info: &wgpu::AdapterInfo) -> String {
	format!(
		"{} ({:?}, {:?})",
		info.name.trim(),
		info.device_type,
		info.backend
	)
}

// The names a driver gives itself when there is no card behind it. wgpu reports
// most of these as DeviceType::Cpu already; the ones that do not are the remote
// and virtual display drivers, which is exactly where the pick was going wrong.
const SOFTWARE_ADAPTERS: &[&str] = &[
	"llvmpipe",
	"softpipe",
	"swrast",
	"lavapipe",
	"basic render",
	"remote display",
	"microsoft remote",
];

// Is there a real graphics processor behind this adapter?
pub fn software_adapter(info: &wgpu::AdapterInfo) -> bool {
	if info.device_type == wgpu::DeviceType::Cpu {
		return true;
	}
	let name = info.name.to_ascii_lowercase();
	SOFTWARE_ADAPTERS.iter().any(|s| name.contains(s))
}

// Is the screen this window draws to somewhere else? Every frame is then encoded
// and shipped over a network, so what the graphics card can do says nothing about
// what the person sees, and timing it would only ever flatter the machine. A
// remote session takes the Remote profile for as long as it lasts and writes
// nothing down, so the console keeps the rating it had.
pub fn remote_session() -> bool {
	#[cfg(windows)]
	{
		// SM_REMOTESESSION. The environment check is the backstop for a session
		// the metric misses, a service-hosted one in particular.
		const SM_REMOTESESSION: i32 = 0x1000;
		let metric = unsafe {
			windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(SM_REMOTESESSION)
		};
		metric != 0
			|| std::env::var("SESSIONNAME")
				.is_ok_and(|name| name.to_ascii_uppercase().starts_with("RDP-"))
	}
	#[cfg(not(windows))]
	{
		// A VNC or xrdp server says so in the environment it starts the session
		// with. A forwarded X display names a host before the colon, where a local
		// one is bare or says unix/localhost.
		["VNCDESKTOP", "XRDP_SESSION", "RFB_PORT"]
			.iter()
			.any(|key| std::env::var_os(key).is_some())
			|| std::env::var("DISPLAY").is_ok_and(|d| forwarded_display(&d))
	}
}

// Does this DISPLAY name a host other than this machine? ":0" and "unix:0" are
// local; "somebox:0" and "1.2.3.4:0" are not.
#[cfg(not(windows))]
fn forwarded_display(display: &str) -> bool {
	let host = display.split(':').next().unwrap_or("");
	!matches!(host, "" | "unix" | "localhost" | "127.0.0.1" | "::1")
}

// The parts of the id that need no graphics adapter, read once on a worker so
// the first frame never waits on a file read. Started from main.
static MACHINE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn probe_machine() {
	std::thread::spawn(|| {
		let _ = MACHINE.set(machine_parts());
	});
}

fn machine_parts() -> String {
	format!("{}|{}", cpu_name(), memory_gib())
}

// Everything the pick depends on, as one short id: the processor, the graphics
// adapter and how much memory there is. Change any of them and the machine has
// to be rated again. Hashed rather than spelled out, so the config carries no
// description of the box it is on.
pub fn hardware_id(info: &wgpu::AdapterInfo) -> String {
	// the worker starts with the process and answers in microseconds; reading it
	// here rather than waiting is the cheaper way to handle "not yet"
	let machine = MACHINE.get().cloned().unwrap_or_else(machine_parts);
	let parts = format!("{machine}|{}", fingerprint(info));
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	for byte in parts.as_bytes() {
		hash = (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3);
	}
	format!("{hash:016x}")
}

// The processor's own name where the OS offers one, plus how many cores are
// usable. The count alone would miss a swap between two chips of one size, and
// the name alone misses a core count the OS was told to restrict.
fn cpu_name() -> String {
	let threads = std::thread::available_parallelism().map_or(0, std::num::NonZero::get);
	#[cfg(windows)]
	let model = std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_default();
	#[cfg(not(windows))]
	let model = std::fs::read_to_string("/proc/cpuinfo")
		.ok()
		.and_then(|text| {
			text.lines()
				.find(|line| line.starts_with("model name"))
				.and_then(|line| line.split_once(':'))
				.map(|(_, value)| value.trim().to_string())
		})
		.unwrap_or_default();
	format!("{model}/{threads}")
}

// Installed memory in whole GiB. Rounded, so the few MiB a driver or a firmware
// update reserves does not read as new hardware.
fn memory_gib() -> u64 {
	#[cfg(windows)]
	{
		use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
		let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
		status.dwLength = u32::try_from(std::mem::size_of::<MEMORYSTATUSEX>()).unwrap_or(0);
		if unsafe { GlobalMemoryStatusEx(&raw mut status) } != 0 {
			return status.ullTotalPhys / (1 << 30);
		}
		0
	}
	#[cfg(not(windows))]
	{
		let pages = unsafe { libc::sysconf(libc::_SC_PHYS_PAGES) };
		let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
		if pages > 0 && page > 0 {
			(pages as u64).saturating_mul(page as u64) / (1 << 30)
		} else {
			0
		}
	}
}

// Where a machine starts before anything is measured, and where it stays when
// there is nothing worth measuring: an adapter with no card behind it is not
// going to hold any rung above Low.
pub fn first_pick(info: &wgpu::AdapterInfo) -> Profile {
	if software_adapter(info) {
		Profile::Low
	} else {
		Profile::Max
	}
}

// Is there anything to time here, or is the first pick already the answer?
// SILK_BENCH=1 forces a run: the banner and the ladder walk are otherwise only
// reachable by putting a different graphics card in the machine.
pub fn worth_measuring(info: &wgpu::AdapterInfo) -> bool {
	std::env::var_os("SILK_BENCH").is_some() || !software_adapter(info)
}

// A short measured run over the ladder, in place of guessing from the adapter's
// name. Each rung gets a moment of full-rate frames with its own settings live,
// and the first whose median frame period fits the display's budget is the
// answer. Standard is never timed: if Low cannot hold the rate, nothing below
// it is in question.
const BENCH_WARMUP: usize = 3; // frames discarded while a rung's settings settle
const BENCH_FRAMES: usize = 40; // frames measured per rung...
const BENCH_RUNG_MS: f32 = 800.0; // ...or this long, whichever comes first
const BENCH_MIN_FRAMES: usize = 5; // never judge a rung on fewer than this
// How far past the budget a rung has to run before the rest of the ladder is
// pointless. The profiles change the per-pixel work by around half; a machine
// several times over is not going to be rescued by any of them, and timing the
// two below it is a few seconds spent on a foregone answer. That case is also
// the slowest to measure, which is exactly the wrong place to be thorough.
const BENCH_HOPELESS: f32 = 4.0;

// What the caller does with the frame it just measured.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Step {
	Measuring,
	Rung(Profile), // this rung missed: put the next one live and keep going
	Done(Profile), // the answer
}

pub struct Bench {
	rungs: &'static [Profile],
	at: usize,
	seen: usize,
	periods: Vec<f32>,
	last: Option<Instant>,
	rung_start: Option<Instant>,
}

impl Bench {
	const LADDER: &'static [Profile] = &[Profile::Max, Profile::High, Profile::Low];

	pub fn new() -> Bench {
		Bench {
			rungs: Bench::LADDER,
			at: 0,
			seen: 0,
			periods: Vec::with_capacity(BENCH_FRAMES),
			last: None,
			rung_start: None,
		}
	}

	// The profile whose settings must be live while this rung is measured.
	pub fn profile(&self) -> Profile {
		self.rungs
			.get(self.at)
			.copied()
			.unwrap_or(Profile::Standard)
	}

	// A frame went out; answer what to do next.
	pub fn note(&mut self, now: Instant, budget_ms: f32) -> Step {
		self.seen += 1;
		if self.seen <= BENCH_WARMUP {
			self.last = Some(now);
			self.rung_start = Some(now);
			return Step::Measuring;
		}
		if let Some(last) = self.last {
			self.periods.push((now - last).as_secs_f32() * 1000.0);
		}
		self.last = Some(now);
		let elapsed = self
			.rung_start
			.map_or(0.0, |start| (now - start).as_secs_f32() * 1000.0);
		let enough = self.periods.len() >= BENCH_FRAMES
			|| (elapsed >= BENCH_RUNG_MS && self.periods.len() >= BENCH_MIN_FRAMES);
		if !enough {
			return Step::Measuring;
		}
		let period = median(&mut self.periods);
		if period <= budget_ms {
			return Step::Done(self.profile());
		}
		if period > budget_ms * BENCH_HOPELESS {
			return Step::Done(Profile::Standard);
		}
		self.at += 1;
		self.periods.clear();
		self.seen = 0;
		self.last = None;
		self.rung_start = None;
		match self.rungs.get(self.at) {
			Some(next) => Step::Rung(*next),
			None => Step::Done(Profile::Standard),
		}
	}
}

// Frames an ease has to pace before its median says anything.
pub const WINDOW: usize = 48;

// How far past the refresh period a frame may run before it counts as a miss:
// half again, so the occasional stretched frame of a busy desktop passes and
// a display dropping every third frame does not.
pub fn budget_ms(refresh_hz: f32) -> f32 {
	1000.0 / refresh_hz.max(1.0) * 1.5
}

pub struct Rating {
	periods: Vec<f32>,
	last: Option<Instant>,
}

impl Rating {
	pub fn new() -> Rating {
		Rating {
			periods: Vec::with_capacity(WINDOW),
			last: None,
		}
	}

	// A frame just went out while an ease was running. Only the gap to the
	// previous such frame is a period; the first after a pause is a start.
	pub fn note(&mut self, now: Instant) {
		if let Some(last) = self.last {
			self.periods.push((now - last).as_secs_f32() * 1000.0);
		}
		self.last = Some(now);
	}

	// The ease stopped, so the next frame's gap means nothing.
	pub fn pause(&mut self) {
		self.last = None;
	}

	// Start over, with nothing measured - the profile changed, so what was
	// measured was measured under another workload.
	pub fn reset(&mut self) {
		self.periods.clear();
		self.last = None;
	}

	// Once a window is full: did the display miss its budget? Empties the
	// window either way, so a verdict is one window's worth of evidence.
	pub fn verdict(&mut self, budget_ms: f32) -> Option<bool> {
		if self.periods.len() < WINDOW {
			return None;
		}
		let over = median(&mut self.periods) > budget_ms;
		self.periods.clear();
		Some(over)
	}
}

fn median(values: &mut [f32]) -> f32 {
	values.sort_by(f32::total_cmp);
	values[values.len() / 2]
}

#[cfg(test)]
mod tests {
	use super::{
		Bench, Profile, Rating, Step, WINDOW, apply, budget_ms, first_pick, software_adapter,
		unapply,
	};
	use crate::config::Settings;
	use std::time::{Duration, Instant};

	fn adapter(name: &str, device_type: wgpu::DeviceType) -> wgpu::AdapterInfo {
		wgpu::AdapterInfo {
			name: name.to_string(),
			vendor: 0,
			device: 0,
			device_type,
			device_pci_bus_id: String::new(),
			driver: String::new(),
			driver_info: String::new(),
			backend: wgpu::Backend::Gl,
			subgroup_min_size: 0,
			subgroup_max_size: 0,
			transient_saves_memory: false,
		}
	}

	#[test]
	fn a_missing_card_is_picked_for_rather_than_timed() {
		let card = adapter("NVIDIA GeForce RTX 3060 Ti", wgpu::DeviceType::DiscreteGpu);
		let none = adapter("llvmpipe (LLVM 19.1.7, 256 bits)", wgpu::DeviceType::Cpu);
		assert_eq!(first_pick(&card), Profile::Max);
		assert_eq!(first_pick(&none), Profile::Low);
		// wgpu labels most software renderers Cpu, but the remote and virtual
		// display drivers arrive as something else and have to be named
		assert!(software_adapter(&adapter(
			"Anything At All",
			wgpu::DeviceType::Cpu
		)));
		assert!(software_adapter(&adapter(
			"llvmpipe (LLVM 19.1.7, 256 bits)",
			wgpu::DeviceType::Other
		)));
		assert!(software_adapter(&adapter(
			"Microsoft Basic Render Driver",
			wgpu::DeviceType::VirtualGpu
		)));
		assert!(!software_adapter(&adapter(
			"NVIDIA GeForce RTX 3060 Ti",
			wgpu::DeviceType::DiscreteGpu
		)));
		assert!(!software_adapter(&adapter(
			"Intel(R) UHD Graphics",
			wgpu::DeviceType::IntegratedGpu
		)));
	}

	#[cfg(not(windows))]
	#[test]
	fn a_display_naming_another_host_is_a_remote_screen() {
		use super::forwarded_display;
		for local in [":0", ":98.0", "unix:0", "localhost:10.0"] {
			assert!(!forwarded_display(local), "{local} is this machine");
		}
		for away in ["b29w:0", "192.168.1.9:0.0"] {
			assert!(forwarded_display(away), "{away} is somewhere else");
		}
	}

	// Frames at `period` ms until the run answers, so a whole run can be walked
	// without a clock.
	fn run_bench(period: f32, budget_ms: f32) -> (Profile, Vec<Profile>) {
		let mut bench = Bench::new();
		let mut now = Instant::now();
		let mut rungs = vec![bench.profile()];
		for _ in 0..4000 {
			now += Duration::from_micros((period * 1000.0) as u64);
			match bench.note(now, budget_ms) {
				Step::Measuring => {}
				Step::Rung(next) => rungs.push(next),
				Step::Done(pick) => return (pick, rungs),
			}
		}
		panic!("the run never answered");
	}

	#[test]
	fn the_run_stops_at_the_first_rung_that_holds_the_rate() {
		let budget = budget_ms(60.0);
		// comfortably inside the budget: the heaviest rung stands, and nothing
		// below it is ever put live
		let (pick, rungs) = run_bench(16.0, budget);
		assert_eq!(pick, Profile::Max);
		assert_eq!(rungs, vec![Profile::Max]);
		// a little over: the ladder is walked and Standard is what is left
		let (pick, rungs) = run_bench(budget * 1.2, budget);
		assert_eq!(pick, Profile::Standard);
		assert_eq!(
			rungs,
			vec![Profile::Max, Profile::High, Profile::Low],
			"each rung has to go live before it is judged"
		);
		// far over: nothing on the ladder can help, so the rest is not timed -
		// which is the case that would otherwise take longest
		let (pick, rungs) = run_bench(budget * 6.0, budget);
		assert_eq!(pick, Profile::Standard);
		assert_eq!(rungs, vec![Profile::Max]);
	}

	fn tuned() -> Settings {
		Settings {
			scroll_ease_in_ms: 300.0,
			scroll_smooth: false,
			cursor_animation: "phase".to_string(),
			text_scrim_radius: 9.0,
			wallpaper_enabled: false,
			..Settings::default()
		}
	}

	#[test]
	fn a_profile_masks_the_stored_values_and_custom_puts_them_back() {
		let mut s = tuned();
		s.performance_profile = "max".to_string();
		apply(&mut s);
		assert!(s.scroll_smooth, "Max is the shipped default");
		assert_eq!(s.scroll_ease_in_ms, Settings::default().scroll_ease_in_ms);
		assert_eq!(s.cursor_animation, "pulse_vertical");
		assert!(s.wallpaper_enabled);

		s.performance_profile = "custom".to_string();
		apply(&mut s);
		assert!(!s.scroll_smooth);
		assert_eq!(s.scroll_ease_in_ms, 300.0);
		assert_eq!(s.cursor_animation, "phase");
		assert_eq!(s.text_scrim_radius, 9.0);
		assert!(!s.wallpaper_enabled);
		assert!(s.profile_shadow.is_none());
	}

	#[test]
	fn applying_twice_does_not_stack() {
		let mut s = tuned();
		s.performance_profile = "low".to_string();
		apply(&mut s);
		s.performance_profile = "high".to_string();
		apply(&mut s);
		assert!(s.wallpaper_enabled, "High keeps the wallpaper");
		unapply(&mut s);
		assert_eq!(s.scroll_ease_in_ms, 300.0, "the user's value, not Low's");
		assert!(!s.wallpaper_enabled, "the user's value, not High's");
	}

	#[test]
	fn each_profile_costs_less_than_the_one_above() {
		let mut s = Settings::default();
		let mut radius = f32::MAX;
		for profile in [Profile::Max, Profile::High] {
			s.performance_profile = profile.key().to_string();
			apply(&mut s);
			assert!(s.scroll_smooth);
			assert!(s.text_scrim);
			assert!(s.text_scrim_radius <= radius);
			radius = s.text_scrim_radius;
		}
		s.performance_profile = "low".to_string();
		apply(&mut s);
		assert!(s.scroll_smooth);
		assert_eq!(s.cursor_animation, "none");
		assert!(s.wallpaper_enabled, "Low keeps the wallpaper");
		assert!(!s.text_scrim, "Low drops the halo");
		assert_eq!(s.text_outline, 2.0, "and leans on the outline");
		for flat in ["standard", "remote"] {
			s.performance_profile = flat.to_string();
			apply(&mut s);
			assert!(!s.scroll_smooth);
			assert!(!s.smooth_scroll_apps);
			assert!(!s.text_scrim);
			assert_eq!(s.text_outline, 0.0);
		}
	}

	// The override is a profile that is never in the file: it sits over whatever
	// the stored one says and lifts off without touching it.
	#[test]
	fn the_remote_override_sits_over_the_stored_profile() {
		let mut s = tuned();
		s.performance_profile = "max".to_string();
		s.remote_override = true;
		apply(&mut s);
		assert_eq!(super::current(&s), Profile::Remote);
		assert!(!s.scroll_smooth);
		assert_eq!(
			s.performance_profile, "max",
			"the stored profile is untouched"
		);
		s.remote_override = false;
		apply(&mut s);
		assert_eq!(super::current(&s), Profile::Max);
		assert!(s.scroll_smooth);
	}

	#[test]
	fn the_ladder_ends_at_standard_and_custom_is_off_it() {
		assert_eq!(Profile::Max.lower(), Some(Profile::High));
		assert_eq!(Profile::Standard.lower(), None);
		assert_eq!(Profile::Remote.lower(), None);
		assert_eq!(Profile::Custom.lower(), None);
		assert_eq!(Profile::parse("LOW"), Profile::Low);
		assert_eq!(
			Profile::parse("silky"),
			Profile::Max,
			"unknown reads as the default"
		);
		for p in Profile::ALL {
			assert_eq!(Profile::from_index(p.index()), p);
		}
	}

	#[test]
	fn a_window_of_stretched_frames_is_a_miss_and_a_pause_breaks_the_chain() {
		let budget = budget_ms(60.0);
		let mut r = Rating::new();
		let mut t = Instant::now();
		for _ in 0..=WINDOW {
			r.note(t);
			t += Duration::from_millis(16);
		}
		assert_eq!(r.verdict(budget), Some(false));
		assert_eq!(r.verdict(budget), None, "the window was spent");
		// a long gap while paused is not a period
		r.pause();
		t += Duration::from_secs(5);
		for _ in 0..=WINDOW {
			r.note(t);
			t += Duration::from_millis(30);
		}
		assert_eq!(r.verdict(budget), Some(true));
	}
}
