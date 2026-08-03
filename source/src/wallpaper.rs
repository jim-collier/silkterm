// SPDX-License-Identifier: GPL-2.0-or-later
// Copyright © 2026 Jim Collier

// The wallpaper pipeline, off the winit thread.
//
// Everything here touches the filesystem or spends real CPU: scanning the
// rotation folder, reading the shuffle history, decoding the image, blurring and
// contrast-flattening it, reading its XMP tags. Any of those paths can be a
// mounted share that answers slowly or not at all, and the blur alone costs
// hundreds of milliseconds on a large image - so none of it may sit between
// launch and the first frame. The window paints with no wallpaper and picks one
// up when the result arrives (UserEvent::WallpaperReady).
//
// Only the GPU upload stays on the winit thread; it needs the device, and it is
// a plain texture write.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use winit::event_loop::EventLoopProxy;

use crate::config::{self, Fit, Settings};
use crate::term::UserEvent;

// A built-in wallpaper baked into the binary, shown when the user has none
// configured (wallpaper_fallback_builtin). ~100KB - negligible next to the binary.
const DEFAULT_BACKGROUND: &[u8] = include_bytes!("../assets/default-background.jpg");

// How many recently-shown images the shuffle holds back at most.
const WP_AVOID_MAX: usize = 32;

// What the worker was asked to do. `settings` is a snapshot: the worker must
// never read the live store, since it outlives the settings it was started with.
pub struct Request {
	pub seq: u64,
	pub settings: Arc<Settings>,
	// also scan the rotation folder and pick from it (startup and each rotation
	// step); false just loads whatever `settings.wallpaper` names.
	pub scan: bool,
	// The image showing now. Order-mode rotation advances from it (by name, so a
	// re-scan that moved things around still lands in the right place), and a
	// non-scanning request keeps it when the settings name none - otherwise
	// re-reading the config while rotating would blank the wallpaper until the
	// next tick, since a rotated pick is live-only and never written to the file.
	pub current: Option<PathBuf>,
}

// Image pixels ready for upload, with the layout the file's own tags asked for.
#[derive(Debug, Clone)]
pub struct Prepared {
	pub rgba: image::RgbaImage,
	pub opacity: f32,
	pub fit: Fit,
	pub anchor: [f32; 2],
}

// What a scan found. Absent when the request didn't scan, or when the folder
// turned out to hold nothing.
#[derive(Debug, Clone)]
pub struct Rotation {
	pub count: usize,
	pub current: PathBuf,
}

#[derive(Debug, Clone)]
pub struct Loaded {
	pub seq: u64,
	pub image: Option<Prepared>,
	pub rotation: Option<Rotation>,
	pub scanned: bool,
}

// Run one request on its own thread and post the result back to the event loop.
//
// A thread per request rather than one long-lived worker, deliberately: a
// request that hangs on a dead mount blocks its own thread forever, and a shared
// worker would leave every later request queued behind it. The stale result is
// harmless when it finally lands - the sequence stamp retires it.
pub fn spawn(proxy: &EventLoopProxy<UserEvent>, request: Request) {
	let proxy = proxy.clone();
	let spawned = std::thread::Builder::new()
		.name("wallpaper".into())
		.spawn(move || {
			let loaded = run(&request);
			let _ = proxy.send_event(UserEvent::WallpaperReady(Box::new(loaded)));
		});
	if let Err(e) = spawned {
		eprintln!(
			"{}: could not start wallpaper loader: {e}",
			config::APP_NAME
		);
	}
}

fn run(request: &Request) -> Loaded {
	let settings = &request.settings;
	let mut rotation = None;
	let mut path = settings.wallpaper.clone();
	if request.scan {
		let showing = request
			.current
			.as_ref()
			.and_then(|path| path.file_name())
			.map(|name| name.to_string_lossy().into_owned());
		rotation = rotate(settings, showing.as_deref());
		if let Some(picked) = &rotation {
			path = Some(picked.current.clone());
		}
	} else if path.is_none() && settings.rotation_folder().is_some() {
		path.clone_from(&request.current);
	}
	// A configured folder that supplies images owns the wallpaper, so the
	// built-in must not stand in for it. On a scan we know what the folder
	// actually holds; otherwise fall back to "is one configured at all".
	let folder_active = if request.scan {
		rotation.is_some()
	} else {
		settings.rotation_folder().is_some()
	};
	let image = settings
		.wallpaper_enabled
		.then(|| prepare(settings, path.as_deref(), folder_active))
		.flatten();
	Loaded {
		seq: request.seq,
		image,
		rotation,
		scanned: request.scan,
	}
}

// Scan the rotation folder, pick the next image, and record the pick.
fn rotate(settings: &Settings, current: Option<&str>) -> Option<Rotation> {
	let dir = settings.rotation_folder()?;
	let images = list_folder_images(dir);
	if images.is_empty() {
		// Silent when the folder was auto-detected - the user never asked for
		// rotation, so an absent or empty dir is not a mistake to report.
		if !settings.wallpaper_folder_auto {
			eprintln!(
				"{}: wallpaper_folder {} has no images",
				config::APP_NAME,
				dir.display()
			);
		}
		return None;
	}
	let mut recent = load_history();
	let showing = current.and_then(|name| index_of(&images, name));
	let index = if settings.wallpaper_rotate_random {
		let held: Vec<usize> = recent
			.iter()
			.filter_map(|name| index_of(&images, name))
			.collect();
		shuffle_pick(images.len(), &held, time_entropy())
	} else {
		next_wallpaper_index(images.len(), showing.unwrap_or(0))
	};
	let current = images[index].clone();
	if let Some(name) = current
		.file_name()
		.map(|n| n.to_string_lossy().into_owned())
	{
		recent.retain(|seen| *seen != name);
		recent.insert(0, name);
		recent.truncate(WP_AVOID_MAX);
		write_history(&recent);
	}
	Some(Rotation {
		count: images.len(),
		current,
	})
}

fn index_of(images: &[PathBuf], name: &str) -> Option<usize> {
	images
		.iter()
		.position(|path| path.file_name().is_some_and(|f| f == name))
}

// Decode the wallpaper and apply everything that is fixed at load time (blur,
// contrast mask, the image's own layout tags). `folder_active` suppresses the
// built-in stand-in, which belongs to rotation when rotation has images.
fn prepare(settings: &Settings, path: Option<&Path>, folder_active: bool) -> Option<Prepared> {
	let mut source = None;
	let mut img = match path {
		Some(path) => match image::open(path) {
			Ok(loaded) => {
				source = Some(path);
				loaded.to_rgba8()
			}
			Err(e) => {
				eprintln!(
					"{}: background image {}: {e}",
					config::APP_NAME,
					path.display()
				);
				builtin(settings, folder_active)?
			}
		},
		// No image or rotation folder configured: fall back to the embedded default
		// so a fresh install still looks the part. Opt out with wallpaper_fallback_builtin.
		None => builtin(settings, folder_active)?,
	};
	// Blur and contrast-flatten, done in LINEAR light (decode sRGB -> process in
	// f32 -> re-encode) so transitions are gamma-correct; an sRGB-space blur
	// darkens edges. The f32 intermediate also avoids 8-bit banding inside the
	// blur (final banding is handled by the high-precision offscreen + the blit's
	// dither).
	if settings.wallpaper_blur > 0.0 || settings.wallpaper_contrast_mask {
		let (w, h) = img.dimensions();
		let mut linear: image::ImageBuffer<image::Rgba<f32>, Vec<f32>> =
			image::ImageBuffer::new(w, h);
		for (dst, src) in linear.pixels_mut().zip(img.pixels()) {
			*dst = image::Rgba([
				config::to_linear(src[0]),
				config::to_linear(src[1]),
				config::to_linear(src[2]),
				f32::from(src[3]) / 255.0,
			]);
		}
		if settings.wallpaper_blur > 0.0 {
			linear = image::imageops::blur(&linear, settings.wallpaper_blur);
		}
		if settings.wallpaper_contrast_mask {
			crate::contrast::apply(
				&mut linear,
				settings.wallpaper_contrast_mask_size,
				settings.wallpaper_contrast_mask_strength,
				settings.wallpaper_contrast_mask_auto,
			);
		}
		for (dst, src) in img.pixels_mut().zip(linear.pixels()) {
			*dst = image::Rgba([
				config::from_linear_u8(src[0]),
				config::from_linear_u8(src[1]),
				config::from_linear_u8(src[2]),
				(src[3].clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
			]);
		}
	}
	// An image can carry its own layout, so a photo isn't squashed by a default
	// that suits gradients. Read straight from the file the pixels came from -
	// the embedded default wallpaper has no path, and keeps the configured fit.
	let mut fit = settings.wallpaper_default_fit;
	let mut anchor = [0.5, 0.5];
	if settings.wallpaper_honor_xmp {
		if let Some(path) = source {
			let tags = crate::xmp::read(path);
			if let Some(tagged) = tags.fit {
				fit = tagged;
			}
			if let Some(tagged) = tags.anchor {
				anchor = tagged;
			}
		}
	}
	Some(Prepared {
		rgba: img,
		opacity: settings.wallpaper_opacity,
		fit,
		anchor,
	})
}

fn builtin(settings: &Settings, folder_active: bool) -> Option<image::RgbaImage> {
	(settings.wallpaper_fallback_builtin && !folder_active)
		.then(|| image::load_from_memory(DEFAULT_BACKGROUND).ok())
		.flatten()
		.map(|img| img.to_rgba8())
}

// Every image in a rotation folder, in filename order.
fn list_folder_images(dir: &Path) -> Vec<PathBuf> {
	let Ok(entries) = std::fs::read_dir(dir) else {
		return Vec::new();
	};
	let mut images: Vec<PathBuf> = entries
		.flatten()
		.map(|e| e.path())
		.filter(|p| p.is_file() && config::is_image_file(p))
		.collect();
	images.sort();
	images
}

// The recently-shown list, newest first. Stored as filenames, not indices, so
// adding or removing images doesn't shift what "recent" means.
fn load_history() -> Vec<String> {
	let Some(path) = config::wallpaper_history_path() else {
		return Vec::new();
	};
	let Ok(text) = std::fs::read_to_string(path) else {
		return Vec::new();
	};
	text.lines()
		.map(str::trim)
		.filter(|line| !line.is_empty())
		.map(String::from)
		.take(WP_AVOID_MAX)
		.collect()
}

fn write_history(recent: &[String]) {
	let Some(path) = config::wallpaper_history_path() else {
		return;
	};
	let mut text = recent.join("\n");
	text.push('\n');
	let _ = std::fs::write(path, text);
}

// Cheap non-crypto entropy for random rotation, from the wall clock. Not used
// for anything security-sensitive - just to vary which image comes up next.
fn time_entropy() -> u64 {
	std::time::SystemTime::now()
		.duration_since(std::time::UNIX_EPOCH)
		.map_or(0, |d| d.as_nanos() as u64)
}

// Next image index in filename order, wrapping.
fn next_wallpaper_index(len: usize, current: usize) -> usize {
	if len < 2 {
		return 0;
	}
	(current + 1) % len
}

// Pick the next image the way a music player shuffles: at random, but never one
// of the last few shown. Straight uniform draws repeat often enough that people
// read them as broken, so holding back roughly half the folder (capped) buys the
// feel of randomness while staying a shuffle rather than a fixed cycle.
// `recent` is newest-first; entries past the hold-back window are ignored.
fn shuffle_pick(len: usize, recent: &[usize], entropy: u64) -> usize {
	if len < 2 {
		return 0;
	}
	let hold = (len / 2).clamp(1, WP_AVOID_MAX).min(len - 1);
	let avoid = &recent[..recent.len().min(hold)];
	// hold <= len-1, so at least one index always survives
	let candidates: Vec<usize> = (0..len).filter(|i| !avoid.contains(i)).collect();
	candidates[(entropy % candidates.len() as u64) as usize]
}

#[cfg(test)]
mod tests {
	use super::{
		Prepared, WP_AVOID_MAX, list_folder_images, next_wallpaper_index, prepare, shuffle_pick,
	};
	use crate::config::{Fit, Settings};

	// The blur and the contrast mask are the slow half and neither is under test
	// here; skipping them keeps these fast.
	fn flat_settings() -> Settings {
		Settings {
			wallpaper_blur: 0.0,
			wallpaper_contrast_mask: false,
			..Settings::default()
		}
	}

	#[test]
	fn wallpaper_order_wraps() {
		assert_eq!(next_wallpaper_index(3, 0), 1);
		assert_eq!(next_wallpaper_index(3, 2), 0); // wraps
		assert_eq!(next_wallpaper_index(1, 0), 0); // single image: stays put
		assert_eq!(next_wallpaper_index(0, 0), 0); // empty: safe
	}

	#[test]
	fn shuffle_never_repeats_a_recent_image() {
		// whatever the entropy, the pick avoids the held-back window and stays in range
		for entropy in 0..200u64 {
			for recent in [
				vec![],
				vec![0],
				vec![3, 1],
				vec![4, 2, 0], // deeper than the window: extra entries are ignored
			] {
				let next = shuffle_pick(5, &recent, entropy);
				assert!(next < 5);
				// 5 images hold back 2, so the two newest must not come back
				for held in recent.iter().take(2) {
					assert_ne!(next, *held, "shuffle repeated a recent image");
				}
			}
		}
	}

	#[test]
	fn shuffle_survives_tiny_folders() {
		// two images alternate; one (or none) has nowhere else to go
		for entropy in 0..20u64 {
			assert_eq!(shuffle_pick(2, &[0], entropy), 1);
			assert_eq!(shuffle_pick(2, &[1], entropy), 0);
			assert_eq!(shuffle_pick(1, &[0], entropy), 0);
			assert_eq!(shuffle_pick(0, &[], entropy), 0);
		}
	}

	#[test]
	fn shuffle_still_reaches_every_image() {
		// holding back recent picks must not strand any image permanently
		let mut seen = std::collections::HashSet::new();
		let mut recent: Vec<usize> = Vec::new();
		for entropy in 0..500u64 {
			let next = shuffle_pick(6, &recent, entropy);
			seen.insert(next);
			recent.insert(0, next);
			recent.truncate(WP_AVOID_MAX);
		}
		assert_eq!(seen.len(), 6, "some image was never picked");
	}

	#[test]
	fn folder_scan_filters_and_sorts() {
		let dir = std::env::temp_dir().join(format!("silkterm_wp_scan_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		for name in ["b.png", "a.JPG", "c.jpeg", "notes.txt", "c.gif", ".hidden"] {
			std::fs::write(dir.join(name), b"x").unwrap();
		}
		std::fs::create_dir_all(dir.join("d.png")).unwrap(); // a dir named like an image
		let imgs = list_folder_images(&dir);
		let names: Vec<String> = imgs
			.iter()
			.map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
			.collect();
		// only decodable image files, case-insensitive ext, sorted; the .txt, the dir
		// and the .gif (no decoder for it) all kept out
		assert_eq!(names, vec!["a.JPG", "b.png", "c.jpeg"]);
		let _ = std::fs::remove_dir_all(&dir);
	}

	// The path is no longer stat'd before the worker sees it (that check used to
	// run on the startup thread), so an unreadable one must land on the built-in
	// rather than on nothing.
	#[test]
	fn an_unreadable_image_still_lands_on_the_builtin() {
		let mut s = flat_settings();
		let missing = std::env::temp_dir().join("silkterm_no_such_wallpaper.png");
		let _ = std::fs::remove_file(&missing);
		assert!(prepare(&s, Some(&missing), false).is_some());
		// ... unless the user opted out, or a rotation folder is supplying images
		s.wallpaper_fallback_builtin = false;
		assert!(prepare(&s, Some(&missing), false).is_none());
		s.wallpaper_fallback_builtin = true;
		assert!(prepare(&s, Some(&missing), true).is_none());
	}

	// An empty rotation folder reports no rotation, and the built-in fills in -
	// the folder's emptiness used to be tested during config resolve, so the
	// suppression has to key on what the scan found, not on the folder existing.
	#[test]
	fn an_empty_rotation_folder_falls_back_to_the_builtin() {
		let mut s = flat_settings();
		let dir = std::env::temp_dir().join(format!("silkterm_wp_empty_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		s.wallpaper_folder = Some(dir.clone());
		s.wallpaper_folder_auto = true; // auto-detected: nothing to report
		assert!(super::rotate(&s, None).is_none());
		assert!(prepare(&s, None, false).is_some());
		let _ = std::fs::remove_dir_all(&dir);
	}

	// The image's own tags win over the configured default, and are read from the
	// file the pixels actually came from (which for rotation is the picked image,
	// not whatever `wallpaper` happened to name).
	#[test]
	fn the_builtin_keeps_the_configured_fit() {
		let s = Settings {
			wallpaper_default_fit: Fit::Zoom,
			..flat_settings()
		};
		let Some(Prepared { fit, anchor, .. }) = prepare(&s, None, false) else {
			panic!("built-in wallpaper failed to decode");
		};
		assert_eq!(fit, Fit::Zoom);
		assert_eq!(anchor, [0.5, 0.5]);
	}
}
