# Encryption and keys

Encryption is optional. Without a key nothing is encrypted; with
`Builder::encryption_key(key)` every chunk and the index are encrypted with
**AES-256-GCM-SIV**. Headers stay readable (magic, version, a flag that the build
is encrypted, chunk size).

## The key

An `EncryptionKey` is **32 random bytes**.

```rust,ignore
use zenith_build::EncryptionKey;   // re-exported from zenith_archive

let key = EncryptionKey::generate()?;        // system random generator
let hex = key.to_hex();                      // Zeroizing<String>, 64 lowercase hex digits
let key = EncryptionKey::from_hex(&hex)?;    // exactly 64 hex digits, either case
let key = EncryptionKey::new([7u8; 32]);     // from bytes you already have
```

* `generate` uses the OS generator (`crypto.getRandomValues` in a browser).
* `from_hex` returns an error that says what is wrong (length, or "not a hex
  digit") but never what the text contained.
* `to_hex` returns a `Zeroizing<String>`: the text is wiped when dropped.
  `EncryptionKey` itself is wiped on drop and its `Debug` prints no key material.
* The keys derived from it (cipher, nonce and name keys) are wiped when the
  reader or writer is dropped. The HMAC computations that derive and use them are
  written out on top of `sha2` instead of using the `hmac` crate, because that
  crate cannot wipe its state: the pads and the intermediate digests are
  `Zeroizing`, and the hash states wipe themselves when dropped (the `zeroize`
  feature of `sha2`; the tests check at compile time that the types promise
  it). The AES key schedule is wiped by `aes` (its `zeroize` feature). There is
  no `unsafe` code in the three crates. What the operating system copies (swap,
  crash dumps) is out of reach for any of this.
* **Never derive a key from a password.** Every header carries a short check
  value, derived from the key, so that a wrong key can be told from damaged data.
  It also lets anyone who holds a build test candidate keys offline at full
  speed. A weak key, even a hashed password, can be brute-forced. Generate the
  key randomly and store it like a secret.

Three separate keys (encryption, nonces, file names) and the check value are
derived from the master key with HKDF-SHA256, each with its own label.

## Keep the key between releases (folder builds)

For `BuildFormat::Fs` the names of the data files and the nonce of every chunk
are derived from the key. **Use the same key for every update of a game.** A new
key renames and re-encrypts every file, so an update would have to rebuild and
re-download everything instead of the files that changed. A build into a folder
that holds a build made with another key is refused
(`PreviousBuildUnreadable`).

The nonces are deterministic on purpose (derived from the key, the path and the
chunk number), so an unchanged file produces identical bytes in every build.
This is safe with AES-GCM-SIV: reusing a nonce for different content reveals
only whether two messages are equal. **Do not replace the cipher with an AEAD
that lacks this property.**

## What an encrypted build hides

| Visible | Hidden |
|---|---|
| That the build is encrypted, the format version, the chunk size | File contents |
| Total size of the archive / sizes and count of the data files | The index: paths, sizes, compression settings, content hashes |
| In a folder build: that two builds share an unchanged file (same bytes) | Real file names: data files are named `ab/<30 hex>` from an HMAC of the path |

Headers, the chunk-size field and footers are authenticated as part of the data
(the exact header bytes, the entry path and the chunk number go into the AEAD
associated data), so a chunk cannot be moved to another file, to another
position, or into a build made with another key or chunk size, without failing
to authenticate.

The nonces are deterministic on purpose, so a chunk of the same file and
position from an *older build made with the same key* is a valid ciphertext
too. It is not accepted all the same: the authenticated index (and the chunk
table of a data file) lists the tag of every chunk it was built with, and an
older chunk has a different one. In a folder build the data file's table also
carries the content hash that the index lists for the file, so an older
whole data file does not fit either. Mixing versions of a file is therefore
detected, down to a single chunk.

What the files cannot tell by themselves: someone who puts back a
*complete* older build (all files, with the old index) gets a complete older
build, and every part of it is valid. Only knowing which build is the current
one helps, and that has to come from outside the build. Every build has a
**build id** for this (`BuildReport::build_id`, `Project::build_id()`): 32 bytes
that change with anything in the build. Keep the id of a release where the
player cannot change it (compiled into the program, or in a signed manifest)
and open the build with `OpenOptions::expected_build_id(id)`: any other build,
an older release included, is `BuildIdMismatch`. See [Reading builds](./reading.md).

## Web builds: better not to encrypt

For a game that runs in a browser, encryption buys little and costs speed:

* The key must be available to the client to decrypt anything, so anyone who
  can run the game can get it.
* Decryption in WebAssembly is slow. A measurement in Node (V8): about **45
  MB/s** on incompressible data and **77 to 162 MB/s** on text and binaries,
  against 167 to 1200 MB/s without encryption. Natively with AES-NI it costs
  little.

## Opening encrypted builds

```rust,ignore
let project = OpenOptions::new().encryption_key(key).open("dist/Game.zenithbuild")?;
```

* No key: `ArchiveEncryptedNoKey`. Wrong key: `ArchiveWrongKey`.
* A key given for a build that is not encrypted is **ignored** by `Project`.
  The lower-level `zenith_archive::ArchiveReader` / `FolderReader` are strict and
  return `ArchiveNotEncrypted` in that case.
* A damaged or swapped chunk fails with `ArchiveCorrupt` ("authentication
  failed").
