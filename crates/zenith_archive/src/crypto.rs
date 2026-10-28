//! Key derivation, deterministic nonces and chunk encryption.
//!
//! AES-256-GCM-SIV is used on purpose: nonces are derived from the path and
//! chunk number so an unchanged file produces identical bytes in every build
//! (small Steam updates). Reusing a nonce for changed content is acceptable
//! only because SIV reveals nothing beyond "these two messages are equal".
//! Do not swap in an AEAD without that property.

use aes_gcm_siv::{
	Aes256GcmSiv, KeyInit, Nonce,
	aead::{Aead, Payload},
};
use sha2::{Digest, Sha256};
use zenith_error::{ZResult, ZenithError};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::corrupt;

const HKDF_SALT: &[u8] = b"zenith-archive-v1";

/// Master key of an encrypted build: 32 random bytes.
///
/// The key must be generated randomly and must **not** be derived from a
/// password. The `key_check` value stored in every header lets anyone holding
/// a build test candidate keys offline at full speed, so a weak key (a
/// password, even after hashing) can be brute-forced.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey([u8; 32]);

impl EncryptionKey {
	#[must_use]
	pub const fn new(bytes: [u8; 32]) -> Self {
		Self(bytes)
	}

	/// Generates a key from the operating system's random generator
	/// (`crypto.getRandomValues` in browsers).
	///
	/// # Errors
	/// Fails if the system generator is unavailable.
	pub fn generate() -> ZResult<Self> {
		let mut bytes = Zeroizing::new([0u8; 32]);
		getrandom::getrandom(bytes.as_mut_slice())
			.map_err(|_| ZenithError::ArchiveInvalidKey("the system random generator failed".into()))?;
		Ok(Self(*bytes))
	}

	/// Parses a key from exactly 64 hex characters (either case).
	///
	/// # Errors
	/// Fails for any other length or a non-hex character. The error says what
	/// is wrong, never what the text contained.
	pub fn from_hex(text: &str) -> ZResult<Self> {
		let digits = text.as_bytes();
		if digits.len() != 64 {
			return Err(ZenithError::ArchiveInvalidKey(format!(
				"expected 64 hex characters, got {} bytes",
				digits.len()
			)));
		}
		let mut bytes = Zeroizing::new([0u8; 32]);
		let (pairs, _) = digits.as_chunks::<2>();
		for (slot, [hi, lo]) in bytes.iter_mut().zip(pairs) {
			match (hex_value(*hi), hex_value(*lo)) {
				(Some(hi), Some(lo)) => *slot = hi << 4 | lo,
				_ => {
					return Err(ZenithError::ArchiveInvalidKey(
						"the key contains a character that is not a hex digit".into(),
					));
				}
			}
		}
		Ok(Self(*bytes))
	}

	/// Lowercase hex of the key. The returned string is wiped when dropped.
	#[must_use]
	pub fn to_hex(&self) -> Zeroizing<String> {
		// Capacity is reserved up front so the secret never sits in a buffer
		// that a reallocation would leave behind.
		let mut text = Zeroizing::new(String::with_capacity(64));
		for byte in self.0 {
			for nibble in [byte >> 4, byte & 0x0f] {
				text.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
			}
		}
		text
	}
}

fn hex_value(digit: u8) -> Option<u8> {
	char::from(digit).to_digit(16).and_then(|v| u8::try_from(v).ok())
}

impl From<[u8; 32]> for EncryptionKey {
	fn from(bytes: [u8; 32]) -> Self {
		Self(bytes)
	}
}

impl std::fmt::Debug for EncryptionKey {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("EncryptionKey(..)")
	}
}

/// HMAC-SHA256 (RFC 2104) over `parts`, as if they were one message, with every
/// buffer that held key-derived bytes wiped afterwards.
///
/// The `hmac` crate keeps the key in a form that is as good as the key (the
/// padded key block and the hash states that started from it) and cannot wipe
/// it, so the algorithm is written out here on top of `sha2`: the pads and the
/// intermediate digests are `Zeroizing`, and the hashers wipe themselves when
/// dropped (`sha2`'s `zeroize` feature). The tests check it against the `hmac`
/// crate for many key and message lengths, and against the RFC 4231 vector.
fn hmac(key: &[u8], parts: &[&[u8]]) -> Zeroizing<[u8; 32]> {
	const BLOCK: usize = 64;

	// A key longer than a block is replaced by its hash; a shorter one is
	// padded with zeros.
	let mut block = Zeroizing::new([0u8; BLOCK]);
	if key.len() > BLOCK {
		let mut hashed = Sha256::digest(key);
		for (b, k) in block.iter_mut().zip(hashed.iter()) {
			*b = *k;
		}
		hashed.as_mut_slice().zeroize();
	} else {
		for (b, k) in block.iter_mut().zip(key) {
			*b = *k;
		}
	}
	let mut ipad = Zeroizing::new([0u8; BLOCK]);
	let mut opad = Zeroizing::new([0u8; BLOCK]);
	for ((i, o), k) in ipad.iter_mut().zip(opad.iter_mut()).zip(block.iter()) {
		*i = k ^ 0x36;
		*o = k ^ 0x5c;
	}

	let mut inner = Sha256::new();
	inner.update(ipad.as_slice());
	for part in parts {
		inner.update(part);
	}
	let mut inner_digest = inner.finalize();

	let mut outer = Sha256::new();
	outer.update(opad.as_slice());
	outer.update(inner_digest.as_slice());
	inner_digest.as_mut_slice().zeroize();
	let mut digest = outer.finalize();

	let mut out = Zeroizing::new([0u8; 32]);
	for (o, d) in out.iter_mut().zip(digest.iter()) {
		*o = *d;
	}
	digest.as_mut_slice().zeroize();
	out
}

/// Keys derived from the master key (HKDF-SHA256, one label per purpose).
///
/// HKDF is two HMACs here: the extract step makes a pseudorandom key from the
/// master key, and every key we need is one block of the expand step (they are
/// all at most 32 bytes). Doing it by hand instead of with the `hkdf` crate
/// keeps the intermediate key out of structures that cannot be wiped; the tests
/// check it against that crate and against the RFC vectors.
pub struct Crypto {
	cipher: Aes256GcmSiv,
	nonce_key: Zeroizing<[u8; 32]>,
	names_key: Zeroizing<[u8; 32]>,
	key_check: [u8; 16],
}

fn key_error() -> ZenithError {
	ZenithError::ArchiveWrite("invalid derived key length".into())
}

/// AAD: container header bytes, then the length-prefixed identifier (entry path
/// or a reserved label), then the chunk number. The length prefix keeps the
/// concatenation unambiguous.
fn aad(header: &[u8], ident: &[u8], idx: u64) -> ZResult<Vec<u8>> {
	let len = u16::try_from(ident.len()).map_err(|_| corrupt("identifier too long"))?;
	let mut out = Vec::with_capacity(header.len().saturating_add(ident.len()).saturating_add(10));
	out.extend_from_slice(header);
	out.extend_from_slice(&len.to_le_bytes());
	out.extend_from_slice(ident);
	out.extend_from_slice(&idx.to_le_bytes());
	Ok(out)
}

impl Crypto {
	pub fn new(key: &EncryptionKey) -> ZResult<Self> {
		// HKDF-Extract, then the first (and only needed) block of HKDF-Expand
		// for each label. Every intermediate is wiped when it goes out of
		// scope.
		let prk = hmac(HKDF_SALT, &[&key.0]);
		let expand = |label: &[u8]| hmac(prk.as_slice(), &[label, &[1]]);
		let enc = expand(b"zenith/v1/enc");
		let nonce_key = expand(b"zenith/v1/nonce");
		let names_key = expand(b"zenith/v1/names");
		let check = expand(b"zenith/v1/check");
		let key_check: [u8; 16] = check.first_chunk().copied().ok_or_else(key_error)?;
		Ok(Self {
			cipher: Aes256GcmSiv::new_from_slice(enc.as_slice()).map_err(|_| key_error())?,
			nonce_key,
			names_key,
			key_check,
		})
	}

	pub const fn key_check(&self) -> [u8; 16] {
		self.key_check
	}

	/// First 12 bytes of `HMAC-SHA256(nonce_key, ident || 0x00 || idx_le)`.
	fn nonce(&self, ident: &[u8], idx: u64) -> ZResult<Nonce> {
		let digest = hmac(self.nonce_key.as_slice(), &[ident, &[0], &idx.to_le_bytes()]);
		let bytes: [u8; 12] = digest.first_chunk().copied().ok_or_else(key_error)?;
		Ok(Nonce::from(bytes))
	}

	#[cfg(feature = "write")]
	pub fn seal(&self, plain: &[u8], header: &[u8], ident: &[u8], idx: u64) -> ZResult<Vec<u8>> {
		let aad = aad(header, ident, idx)?;
		self.cipher
			.encrypt(&self.nonce(ident, idx)?, Payload { msg: plain, aad: &aad })
			.map_err(|_| ZenithError::ArchiveWrite("encryption failed".into()))
	}

	pub fn open(&self, stored: &[u8], header: &[u8], ident: &[u8], idx: u64) -> ZResult<Vec<u8>> {
		let aad = aad(header, ident, idx)?;
		self.cipher
			.decrypt(&self.nonce(ident, idx)?, Payload { msg: stored, aad: &aad })
			.map_err(|_| corrupt("authentication failed (damaged, tampered or swapped data)"))
	}

	/// Stable data-file name for an entry path: `ab/<30 hex>` where the 32 hex
	/// digits are the first 16 bytes of `HMAC-SHA256(names_key, path)`.
	pub fn data_file_name(&self, path: &str) -> String {
		let digest = hmac(self.names_key.as_slice(), &[path.as_bytes()]);
		let mut name = String::with_capacity(33);
		for (i, byte) in digest.iter().take(16).enumerate() {
			if i == 1 {
				name.push('/');
			}
			for nibble in [byte >> 4, byte & 0x0f] {
				name.push(char::from_digit(u32::from(nibble), 16).unwrap_or('0'));
			}
		}
		name
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	const HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

	#[test]
	fn hex_roundtrip() -> ZResult<()> {
		let key = EncryptionKey::from_hex(HEX)?;
		assert_eq!(key.0.first(), Some(&0), "first byte");
		assert_eq!(key.0.last(), Some(&0x1f), "last byte");
		assert_eq!(key.to_hex().as_str(), HEX, "to_hex inverts from_hex");

		let upper = EncryptionKey::from_hex(&HEX.to_uppercase())?;
		assert_eq!(upper.0, key.0, "uppercase digits are accepted");

		let generated = EncryptionKey::generate()?;
		let again = EncryptionKey::from_hex(&generated.to_hex())?;
		assert_eq!(again.0, generated.0, "a generated key survives the hex round trip");
		Ok(())
	}

	#[test]
	fn generated_keys_differ() -> ZResult<()> {
		let (a, b) = (EncryptionKey::generate()?, EncryptionKey::generate()?);
		assert_ne!(a.0, b.0, "two generated keys must differ");
		assert_ne!(a.0, [0u8; 32], "a generated key is not all zeros");
		Ok(())
	}

	#[test]
	fn wrong_length_is_rejected_without_leaking() {
		for len in [0usize, 1, 62, 63, 65, 66, 128] {
			let text = "ab".repeat(len).chars().take(len).collect::<String>();
			let err = EncryptionKey::from_hex(&text).err().map(|e| e.to_string());
			let message = err.unwrap_or_default();
			assert!(
				message.contains("64 hex characters"),
				"length {len} must be rejected: {message:?}"
			);
			if len > 4 {
				assert!(!message.contains(&text), "the error must not echo the input");
			}
		}
	}

	#[test]
	fn non_hex_is_rejected_without_leaking() {
		let secret_like = format!("{}{}", &HEX[..60], "zzzz");
		let cases = [
			secret_like,
			format!("{}g", &HEX[..63]),
			format!("{} ", &HEX[..63]),
			format!("0x{}", &HEX[..62]),
			// 62 hex digits + a two-byte character: 64 bytes but not 64 hex digits,
			// and slicing text bytes naively would panic on the char boundary.
			format!("{}é", &HEX[..62]),
		];
		for text in cases {
			assert_eq!(text.len(), 64, "test input must be 64 bytes long");
			let message = EncryptionKey::from_hex(&text)
				.err()
				.map(|e| e.to_string())
				.unwrap_or_default();
			assert!(message.contains("not a hex digit"), "{message:?}");
			assert!(!message.contains(&text[..20]), "the error must not echo the input");
		}
		assert!(
			EncryptionKey::from_hex("ééé").is_err(),
			"short non-ASCII input is an error, not a panic"
		);
	}

	#[test]
	fn debug_hides_the_key() -> ZResult<()> {
		let key = EncryptionKey::from_hex(HEX)?;
		let shown = format!("{key:?}");
		assert_eq!(shown, "EncryptionKey(..)", "Debug prints no key material");
		Ok(())
	}

	fn hex(bytes: &[u8]) -> String {
		use std::fmt::Write;
		bytes.iter().fold(String::new(), |mut text, byte| {
			let _ = write!(text, "{byte:02x}");
			text
		})
	}

	fn reference_hmac(key: &[u8], data: &[u8]) -> [u8; 32] {
		use hmac::{Hmac, Mac};
		use sha2_reference::Sha256;
		let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).unwrap_or_else(|_| unreachable!());
		mac.update(data);
		mac.finalize().into_bytes().into()
	}

	#[test]
	fn hmac_matches_the_reference_crate_for_all_sizes_around_the_block() {
		for key_len in [0usize, 1, 17, 32, 63, 64, 65, 100, 200] {
			let key: Vec<u8> = (0..key_len)
				.map(|i| u8::try_from((i * 7 + key_len) % 251).unwrap_or(0))
				.collect();
			for msg_len in [0usize, 1, 55, 56, 63, 64, 65, 119, 200] {
				let msg: Vec<u8> = (0..msg_len)
					.map(|i| u8::try_from((i * 13 + msg_len) % 241).unwrap_or(0))
					.collect();
				let parts: Vec<&[u8]> = msg.chunks(msg_len / 3 + 1).collect();
				assert_eq!(
					*hmac(&key, &parts),
					reference_hmac(&key, &msg),
					"key {key_len} bytes, message {msg_len} bytes"
				);
			}
		}
	}

	#[test]
	fn hmac_matches_rfc_4231_test_case_1() {
		assert_eq!(
			hex(hmac(&[0x0b; 20], &[b"Hi ", b"There"]).as_slice()),
			"b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7",
			"key 0x0b x20, data \"Hi There\""
		);
	}

	#[test]
	fn key_derivation_matches_rfc_5869_and_the_hkdf_crate() {
		// RFC 5869, appendix A.1 (SHA-256): the extract step and the first
		// block of the expand step, which is all this code uses.
		let ikm = [0x0b; 22];
		let salt: Vec<u8> = (0..=0x0c).collect();
		let info: Vec<u8> = (0xf0..=0xf9).collect();
		let prk = hmac(&salt, &[&ikm]);
		assert_eq!(
			hex(prk.as_slice()),
			"077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5",
			"PRK"
		);
		assert_eq!(
			hex(hmac(prk.as_slice(), &[&info, &[1]]).as_slice()),
			"3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf",
			"first block of OKM"
		);

		// The same steps as `Crypto::new` takes, against the crate it replaced.
		for seed in 0u8..16 {
			let master: Vec<u8> = (0u8..32).map(|i| i.wrapping_mul(seed).wrapping_add(seed)).collect();
			let reference = hkdf::Hkdf::<sha2_reference::Sha256>::new(Some(HKDF_SALT), &master);
			let prk = hmac(HKDF_SALT, &[&master]);
			for label in [
				&b"zenith/v1/enc"[..],
				b"zenith/v1/nonce",
				b"zenith/v1/names",
				b"zenith/v1/check",
			] {
				let mut want = [0u8; 32];
				assert!(reference.expand(label, &mut want).is_ok(), "reference expand");
				assert_eq!(*hmac(prk.as_slice(), &[label, &[1]]), want, "seed {seed}");
				let mut short = [0u8; 16];
				assert!(
					reference.expand(label, &mut short).is_ok(),
					"reference expand, 16 bytes"
				);
				assert_eq!(
					want.get(..16),
					Some(&short[..]),
					"a shorter output is the start of the block"
				);
			}
		}
	}

	#[test]
	fn the_hashers_and_keys_wipe_themselves_when_dropped() {
		fn wipes_on_drop<T: ZeroizeOnDrop>() {}
		wipes_on_drop::<Sha256>();
		wipes_on_drop::<EncryptionKey>();
		// The AES key schedule inside the cipher (the cipher itself is a
		// wrapper).
		wipes_on_drop::<aes::Aes256>();
	}
}
