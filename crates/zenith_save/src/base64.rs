//! Standard base64 with padding, for the bytes inside a TOML section.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(data: &[u8]) -> String {
	let mut out = String::with_capacity(data.len().div_ceil(3).saturating_mul(4));
	for chunk in data.chunks(3) {
		let mut group = [0u8; 3];
		for (slot, byte) in group.iter_mut().zip(chunk) {
			*slot = *byte;
		}
		let n = (u32::from(group[0]) << 16) | (u32::from(group[1]) << 8) | u32::from(group[2]);
		for shift in [18u32, 12, 6, 0].iter().take(chunk.len() + 1) {
			out.push(char::from(sextet(n >> shift)));
		}
		for _ in chunk.len()..3 {
			out.push('=');
		}
	}
	out
}

fn sextet(bits: u32) -> u8 {
	ALPHABET.get((bits & 0x3f) as usize).copied().unwrap_or(b'=')
}

/// `None` if `text` is not base64 with padding.
pub fn decode(text: &str) -> Option<Vec<u8>> {
	let bytes = text.as_bytes();
	if !bytes.len().is_multiple_of(4) {
		return None;
	}
	let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
	let groups = bytes.chunks(4).count();
	for (index, group) in bytes.chunks(4).enumerate() {
		let padding = group.iter().rev().take_while(|b| **b == b'=').count();
		// Padding only at the very end, and at most two characters of it.
		if padding > 2 || (padding > 0 && index + 1 != groups) {
			return None;
		}
		let mut n = 0u32;
		for byte in group.iter().take(4 - padding) {
			let value = ALPHABET.iter().position(|a| a == byte)?;
			n = (n << 6) | u32::try_from(value).ok()?;
		}
		n <<= 6 * u32::try_from(padding).ok()?;
		let produced = 3 - padding;
		for shift in [16u32, 8, 0].iter().take(produced) {
			out.push(u8::try_from((n >> shift) & 0xff).ok()?);
		}
	}
	Some(out)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn matches_the_known_vectors() {
		for (plain, coded) in [
			("", ""),
			("f", "Zg=="),
			("fo", "Zm8="),
			("foo", "Zm9v"),
			("foob", "Zm9vYg=="),
			("fooba", "Zm9vYmE="),
			("foobar", "Zm9vYmFy"),
		] {
			assert_eq!(encode(plain.as_bytes()), coded, "encode {plain:?}");
			assert_eq!(decode(coded).as_deref(), Some(plain.as_bytes()), "decode {coded:?}");
		}
	}

	#[test]
	fn every_byte_round_trips() {
		let data: Vec<u8> = (0..=255).collect();
		assert_eq!(decode(&encode(&data)), Some(data));
	}

	#[test]
	fn refuses_what_is_not_base64() {
		for bad in ["Zg", "Z===", "Zm9v!A==", "Zg==Zg==", "=Zg=", "Zm 9"] {
			assert!(decode(bad).is_none(), "{bad:?}");
		}
	}
}
