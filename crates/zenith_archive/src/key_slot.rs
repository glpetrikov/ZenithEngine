//! The key of a build, carried by the program that plays it.
//!
//! The engine's executable, and its `.wasm` for the browser, hold a slot of
//! [`SLOT_LEN`] bytes: a marker, a flag and room for a key. A build made with a
//! key writes the key into the slot of its copy of the program
//! ([`write_key`]); the program reads it back when it starts
//! ([`embedded_key`]).
//!
//! This keeps the key out of sight, nothing more. The program is in the hands
//! of the player, and whoever looks for the key finds it. An encrypted build
//! cannot be opened by a tool that happens to read archives; it is not safe
//! from a player who wants its files.

use zenith_error::{ZResult, ZenithError};
use zeroize::Zeroizing;

use crate::crypto::EncryptionKey;

/// Bytes nothing else in a program is likely to hold, to find the slot by.
const MARKER: [u8; MARKER_LEN] = [
	0x5f, 0x7c, 0x0e, 0x78, 0x66, 0x28, 0x88, 0xfc, 0xac, 0xf0, 0x73, 0x9c, 0x43, 0x8d, 0x78, 0x2b,
];
const MARKER_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// The marker, a flag and the key.
pub const SLOT_LEN: usize = MARKER_LEN + 1 + KEY_LEN;
const HAS_KEY: u8 = 1;
const NO_KEY: u8 = 0xcc;
/// What fills the room of the key while there is none. Not zeros: the
/// optimizer of a `.wasm` drops the zeros at the end of a block of data
/// (memory starts zeroed), and the room would not be in the file to write the
/// key into.
const NO_KEY_FILLER: [u8; KEY_LEN] = [
	0x7e, 0xa2, 0x84, 0x0a, 0x6c, 0xd4, 0x71, 0x7f, 0x7e, 0x7f, 0x81, 0x31, 0x92, 0x8f, 0x46, 0x21, 0xbc, 0x77, 0x9b,
	0x60, 0x87, 0xe2, 0x54, 0x21, 0x51, 0xb9, 0x8e, 0x57, 0x48, 0x85, 0x42, 0xcc,
];

/// The slot as it is compiled: the marker, then no key.
static SLOT: [u8; SLOT_LEN] = compiled_slot();

const fn compiled_slot() -> [u8; SLOT_LEN] {
	let mut slot = [NO_KEY; SLOT_LEN];
	let (marker, rest) = slot.split_at_mut(MARKER_LEN);
	marker.copy_from_slice(&MARKER);
	let (_, room) = rest.split_at_mut(1);
	room.copy_from_slice(&NO_KEY_FILLER);
	slot
}

/// The slot as this program holds it. Read through a volatile read: the
/// compiler sees an immutable static and would otherwise put its compiled
/// contents (no key) wherever it is read, and a key written into the file
/// later would never be seen. The volatile read also keeps the static in the
/// program, and keeps [`MARKER`] from being copied anywhere else in it, where a
/// search for the slot would find it twice.
fn current() -> Zeroizing<[u8; SLOT_LEN]> {
	let start = (&raw const SLOT).cast::<u8>();
	let mut slot = Zeroizing::new([0; SLOT_LEN]);
	for (at, byte) in slot.iter_mut().enumerate() {
		// SAFETY: `SLOT` is a static of `SLOT_LEN` bytes and `at` is below
		// `SLOT_LEN`: the pointer is in it, aligned (a byte) and initialized
		// for the whole life of the program.
		*byte = unsafe { start.add(at).read_volatile() };
	}
	slot
}

/// The key the build wrote into this program, if it was made with one.
pub fn embedded_key() -> Option<EncryptionKey> {
	let slot = current();
	if slot.get(MARKER_LEN) != Some(&HAS_KEY) {
		return None;
	}
	let key: [u8; KEY_LEN] = slot.get(MARKER_LEN + 1..)?.try_into().ok()?;
	Some(EncryptionKey::new(key))
}

/// The slot as a program holds it before a key is written: what to put into a
/// pretend program, to test a tool that writes keys.
pub fn blank_slot() -> [u8; SLOT_LEN] {
	let mut slot = *current();
	if let Some(rest) = slot.get_mut(MARKER_LEN..) {
		write_blank(rest);
	}
	slot
}

/// The flag and the room of a slot without a key.
fn write_blank(flag_and_room: &mut [u8]) {
	let (flag, room) = flag_and_room.split_at_mut(1);
	flag.fill(NO_KEY);
	room.copy_from_slice(&NO_KEY_FILLER);
}

/// The marker, from the slot of this program. Not from [`MARKER`]: a copy of
/// it in the code would be a second marker in the program.
fn marker() -> [u8; MARKER_LEN] {
	let slot = current();
	let mut marker = [0; MARKER_LEN];
	if let Some(bytes) = slot.get(..MARKER_LEN) {
		marker.copy_from_slice(bytes);
	}
	marker
}

/// Where the flag and the room of the slot are in `bytes` (the whole file of a
/// program), if it has a slot.
fn find_slot(bytes: &[u8]) -> ZResult<Option<std::ops::Range<usize>>> {
	let damaged = || ZenithError::ArchiveInvalidKey("the key slot of the program is damaged".into());
	let marker = marker();
	let mut found = bytes
		.windows(MARKER_LEN)
		.enumerate()
		.filter(|(_, window)| *window == marker)
		.map(|(at, _)| at);
	let Some(at) = found.next() else {
		return Ok(None);
	};
	if found.next().is_some() {
		return Err(ZenithError::ArchiveInvalidKey(
			"the program has more than one key slot".into(),
		));
	}
	let range = at
		.checked_add(MARKER_LEN)
		.and_then(|start| Some(start..start.checked_add(1 + KEY_LEN)?))
		.ok_or_else(damaged)?;
	// What follows the marker must be a slot, with a key or without: if the
	// program was changed after it was compiled (an optimizer that dropped
	// the room of the key), writing there would break it.
	let rest = bytes.get(range.clone()).ok_or_else(damaged)?;
	let (flag, room) = rest.split_at(1);
	let blank = flag == [NO_KEY] && room == NO_KEY_FILLER;
	if !blank && flag != [HAS_KEY] {
		return Err(damaged());
	}
	Ok(Some(range))
}

/// Does the program `bytes` have a slot for a key?
///
/// # Errors
/// `ArchiveInvalidKey` if it holds the marker more than once, or what follows
/// the marker is not a slot.
pub fn has_slot(bytes: &[u8]) -> ZResult<bool> {
	find_slot(bytes).map(|range| range.is_some())
}

/// Writes `key` into the slot of a program, `bytes` being the whole file of
/// the program; `None` empties the slot. Returns `false` if the program has no
/// slot (it was made by an engine older than the slot).
///
/// # Errors
/// `ArchiveInvalidKey` if the program holds the marker more than once (which
/// one is the slot cannot be told), or what follows the marker is not a slot.
pub fn write_key(bytes: &mut [u8], key: Option<&EncryptionKey>) -> ZResult<bool> {
	let Some(range) = find_slot(bytes)? else {
		return Ok(false);
	};
	let place = bytes
		.get_mut(range)
		.ok_or_else(|| ZenithError::ArchiveInvalidKey("the key slot of the program is damaged".into()))?;
	if let Some(key) = key {
		let (flag, room) = place.split_at_mut(1);
		flag.fill(HAS_KEY);
		room.copy_from_slice(key.bytes());
	} else {
		write_blank(place);
	}
	Ok(true)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A pretend program: some bytes, the slot as compiled, more bytes.
	fn program() -> Vec<u8> {
		let mut bytes = vec![0xAB; 100];
		bytes.extend_from_slice(&blank_slot());
		bytes.extend_from_slice(&[0xCD; 100]);
		bytes
	}

	#[test]
	fn a_slot_is_found_once() -> ZResult<()> {
		assert!(has_slot(&program())?);
		assert!(!has_slot(&[0; 300])?);
		Ok(())
	}

	fn slot_of(bytes: &[u8]) -> &[u8] {
		bytes.get(100..100 + SLOT_LEN).unwrap_or_default()
	}

	#[test]
	fn a_program_that_was_not_given_a_key_has_none() {
		assert!(embedded_key().is_none(), "the slot is empty as compiled");
	}

	#[test]
	fn the_key_is_written_into_the_slot_and_taken_out_again() -> ZResult<()> {
		let mut bytes = program();
		let key = EncryptionKey::new([7; 32]);
		assert!(write_key(&mut bytes, Some(&key))?, "the slot is found");

		let slot = slot_of(&bytes);
		assert_eq!(slot.get(..MARKER_LEN), Some(MARKER.as_slice()), "the marker stays");
		assert_eq!(slot.get(MARKER_LEN), Some(&HAS_KEY));
		assert_eq!(slot.get(MARKER_LEN + 1..), Some([7; 32].as_slice()));
		assert_eq!(bytes.get(..100), Some([0xAB; 100].as_slice()), "nothing else changes");

		assert!(write_key(&mut bytes, None)?, "the slot is found again");
		assert_eq!(slot_of(&bytes), blank_slot().as_slice(), "and emptied");
		Ok(())
	}

	#[test]
	fn a_program_without_a_slot_is_told_apart() -> ZResult<()> {
		let mut bytes = vec![0; 1000];
		assert!(!write_key(&mut bytes, Some(&EncryptionKey::new([1; 32])))?);
		assert_eq!(bytes, vec![0; 1000], "and left alone");
		Ok(())
	}

	#[test]
	fn two_slots_are_refused() {
		let mut bytes = program();
		bytes.extend_from_slice(&blank_slot());
		assert!(write_key(&mut bytes, Some(&EncryptionKey::new([1; 32]))).is_err());
		assert!(has_slot(&bytes).is_err());
	}

	#[test]
	fn a_slot_at_the_very_end_that_is_cut_short_is_refused() {
		let mut bytes = vec![0; 10];
		bytes.extend_from_slice(&MARKER);
		assert!(write_key(&mut bytes, None).is_err());
	}

	/// What the optimizer of a `.wasm` would make of a slot of zeros: the
	/// marker, and something else where the room was.
	#[test]
	fn a_marker_that_is_not_followed_by_a_slot_is_left_alone() {
		let mut bytes = MARKER.to_vec();
		bytes.extend_from_slice(&[0x64; 100]);
		let before = bytes.clone();
		assert!(write_key(&mut bytes, Some(&EncryptionKey::new([1; 32]))).is_err());
		assert_eq!(bytes, before, "nothing was written");
	}

	#[test]
	fn the_empty_slot_has_no_zeros_to_drop() {
		assert!(compiled_slot().iter().all(|&byte| byte != 0));
		assert_eq!(blank_slot(), compiled_slot());
	}

	#[test]
	fn a_key_can_be_written_over_another() -> ZResult<()> {
		let mut bytes = program();
		write_key(&mut bytes, Some(&EncryptionKey::new([1; 32])))?;
		assert!(write_key(&mut bytes, Some(&EncryptionKey::new([2; 32])))?);
		assert_eq!(slot_of(&bytes).get(MARKER_LEN + 1..), Some([2; 32].as_slice()));
		Ok(())
	}
}
