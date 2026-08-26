// The build number: whole minutes since 2000 began, written in Crockford base 32.
// Five characters until 2063, it sorts in the order the builds were made, and it
// decodes back to the minute one was built - which is what a copy's file date
// could never be trusted for, since every producer stamps that differently.
//
// build.rs include!s this file, so the number baked into the binary and the tests
// below are the same code. That is why nothing here has a `use` line and why the
// module is only compiled into the crate under cfg(test): at run time the answer
// is already a string in the environment (config::BUILD_ID).

// Crockford's alphabet, lowercase. No i, l, o or u, so nothing in a build number
// copied out of a bug report can be read back as a digit or as another letter.
const CROCKFORD_LOWER: &[u8; 32] = b"0123456789abcdefghjkmnpqrstvwxyz";

// 2000-01-01T00:00:00Z, as unix time.
const EPOCH_2000_UNIX: u64 = 946_684_800;

fn crockford32(value: u64) -> String {
	if value == 0 {
		return "0".to_string();
	}
	let mut digits = Vec::new();
	let mut left = value;
	while left > 0 {
		digits.push(CROCKFORD_LOWER[(left % 32) as usize] as char);
		left /= 32;
	}
	digits.iter().rev().collect()
}

// Whole elapsed minutes, so the number only ever goes up. A clock set before 2000
// gives 0 instead of wrapping.
fn minutes_since_2000(unix_secs: u64) -> u64 {
	unix_secs.saturating_sub(EPOCH_2000_UNIX) / 60
}

#[cfg(test)]
mod tests {
	use super::*;

	fn build_number_at(unix_secs: u64) -> String {
		crockford32(minutes_since_2000(unix_secs))
	}

	#[test]
	fn the_alphabet_is_crockfords_with_the_ambiguous_letters_left_out() {
		let alphabet = std::str::from_utf8(CROCKFORD_LOWER).unwrap();
		assert_eq!(alphabet, "0123456789abcdefghjkmnpqrstvwxyz");
		for skipped in ['i', 'l', 'o', 'u'] {
			assert!(!alphabet.contains(skipped), "{skipped} is ambiguous");
		}
		assert_eq!(alphabet.len(), 32);
	}

	#[test]
	fn small_values_encode_digit_by_digit() {
		assert_eq!(crockford32(0), "0");
		assert_eq!(crockford32(9), "9");
		// 10 is where the letters start, and h is 17 - one past the skipped i.
		assert_eq!(crockford32(10), "a");
		assert_eq!(crockford32(17), "h");
		assert_eq!(crockford32(18), "j");
		assert_eq!(crockford32(31), "z");
		assert_eq!(crockford32(32), "10");
		assert_eq!(crockford32(33), "11");
		assert_eq!(crockford32(1024), "100");
	}

	#[test]
	fn a_build_number_decodes_back_to_the_minute_it_was_built() {
		// Round-trip every digit position through a hand-rolled decode, so an
		// encoder that quietly reversed itself would not pass.
		let decode = |text: &str| -> u64 {
			text.bytes().fold(0u64, |sum, byte| {
				let digit = CROCKFORD_LOWER.iter().position(|c| *c == byte).unwrap();
				sum * 32 + digit as u64
			})
		};
		for minutes in [0, 1, 31, 32, 1_000, 14_000_000, u32::MAX as u64] {
			assert_eq!(decode(&crockford32(minutes)), minutes);
		}
	}

	#[test]
	fn the_epoch_is_the_start_of_2000_and_the_count_is_whole_minutes() {
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX), 0);
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX + 59), 0); // part of a minute doesn't count
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX + 60), 1);
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX + 61), 1);
		// A day, and a non-leap year.
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX + 86_400), 1_440);
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX + 365 * 86_400), 525_600);
	}

	#[test]
	fn a_clock_behind_the_epoch_gives_zero_rather_than_wrapping() {
		assert_eq!(minutes_since_2000(0), 0);
		assert_eq!(minutes_since_2000(EPOCH_2000_UNIX - 1), 0);
		assert_eq!(build_number_at(0), "0");
	}

	#[test]
	fn a_later_build_always_sorts_after_an_earlier_one() {
		// Same length means plain string order works; the length only grows, so
		// it holds across a rollover too.
		let earlier = build_number_at(EPOCH_2000_UNIX + 14_000_000 * 60);
		let later = build_number_at(EPOCH_2000_UNIX + 14_000_001 * 60);
		assert!(earlier < later, "{earlier} should sort before {later}");
		assert_eq!(earlier.len(), later.len());
		assert!(build_number_at(EPOCH_2000_UNIX) < build_number_at(EPOCH_2000_UNIX + 60));
	}

	#[test]
	fn the_number_stays_five_characters_for_the_life_of_this_program() {
		// 32^5 minutes past 2000 is partway through 2063; anything sooner is five
		// characters, which is what the About panel and the release notes assume.
		let year_2026 = EPOCH_2000_UNIX + 26 * 365 * 86_400;
		let year_2060 = EPOCH_2000_UNIX + 60 * 365 * 86_400;
		assert_eq!(build_number_at(year_2026).len(), 5);
		assert_eq!(build_number_at(year_2060).len(), 5);
	}
}
