# Build formats (byte layout)

Both containers share one codec. All integers are little-endian. The format has
version 1 and is not released yet, so it may still change without a version bump;
readers reject any other version.

## Common pieces

### Header (48 bytes at offset 0 of every container file)

| Offset | Size | Field | Rule |
|---|---|---|---|
| 0 | 8 | magic | per container, see below |
| 8 | 2 | format version | must be 1 |
| 10 | 2 | flags | bit 0 = encrypted; other bits must be 0 |
| 12 | 1 | AEAD id | 0 = none, 1 = AES-256-GCM-SIV; 1 exactly when the flag is set |
| 13 | 1 | compression id | 1 = zstd |
| 14 | 1 | chunk size, log2 | 12..=24 (default 18 = 256 KiB) |
| 15 | 1 | reserved | 0 |
| 16 | 16 | key check | zero when not encrypted |
| 32 | 12 | reserved | zeros |
| 44 | 4 | CRC32 of bytes 0..44 | |

Magics (8 bytes): archive `89 "ZARC" 0D 0A 1A`, folder index `89 "ZFIX" 0D 0A 1A`,
data file `89 "ZDAT" 0D 0A 1A`. The header contains nothing that varies between
builds, which keeps output reproducible.

### Footer (32 bytes at the end of the file)

| Offset | Size | Field |
|---|---|---|
| 0 | 8 | end magic: `89 "ZARCEND"`, `89 "ZFIXEND"` or `89 "ZDATEND"` |
| 8 | 8 | body offset |
| 16 | 8 | body length (as stored) |
| 24 | 4 | CRC32 of the stored body (0 when encrypted) |
| 28 | 4 | CRC32 of footer bytes 0..28 |

The body is the index (archive, folder index) or the chunk table (data file).
It must end exactly where the footer starts; its length is limited to 256 MiB
and checked before anything is allocated. Encrypted bodies are authenticated by
the AEAD tag instead of the CRC.

### Chunk

Data is cut into chunks of `2^chunk_size_log2` bytes (the last may be shorter).
Each chunk is an independent zstd frame that declares its content size
(which must equal the expected size) and uses no dictionary, no content
checksum (the chunk table's CRC32 or the AEAD tag covers the chunk) and a window
no larger than a chunk; then, if encrypted, AES-256-GCM-SIV (16 more bytes of
tag).
Reading a chunk never needs its neighbours.

Chunk table record (8 bytes): `stored length u32`, `check u32`. Without a key
`check` is the CRC32 of the stored chunk. With a key it is the first four bytes
of the chunk's AEAD tag (the last 16 bytes of the stored chunk). The index and
the chunk tables are authenticated, so they pin the exact chunk of the exact
build: a chunk of the same file and position from another build made with the
same key has a valid tag of its own, but not this one.

## Single-file archive

```text
[Header][chunks of file 1][chunks of file 2]...[Index body][Footer]
```

Entries are stored in strictly ascending byte order of their path and their
data is contiguous, starting at offset 48. Index body:

```text
entry_count u32
repeated: path_len u16, path (UTF-8), size u64, data_offset u64,
          chunk_count u32, chunk table (chunk_count * 8 bytes)
```

An empty file has no chunks. The index is not compressed. Writing needs one
pass, no seeking and no sizes known in advance.

## Folder build

```text
build.index            [Header][Index body][Footer]   (body offset is 48)
<data file>...         one per entry
```

Index body:

```text
entry_count u32
repeated: path_len u16, path, name_len u16, name,
          size u64, data_len u64,
          content_hash [32], compression_level u8, chunk_size_log2 u8
```

* `name` is the data file's path relative to the build folder. Without a key it
  equals `path`. With a key it is `ab/<30 hex>`, the hex of the first 16 bytes of
  `HMAC-SHA256(names_key, path)` split after two characters. The reader
  recomputes it and rejects an index that disagrees, so a hostile index cannot
  point at another file.
* `data_len` is the size of the data file on disk.
* `content_hash` is BLAKE3 of the source content, chosen by the build tool,
  which also uses it to skip unchanged files. The same value is stored in the
  data file's table, and the two must be equal, so a data file of another
  version of the file does not fit the index.
  `compression_level` is 1..=22 and `chunk_size_log2` must equal the header's.
  (These three fields were added to the version 1 layout before release.)

Data file: `[Header][chunks][Table][Footer]`, where the table is
`uncompressed size u64`, `chunk_count u32`, `content_hash [32]` and the chunk
table. A data file must have the same flags, chunk size and key check as the
index, and the `content_hash` of the index entry.

## Build id

`BuildId` is `SHA-256("zenith-build-id-v1" || header (48 bytes) || index as
stored)`, where the index is the archive's index body or `build.index`'s body
exactly as it is in the file (encrypted if the build is). It is not stored in the
build; a reader works it out when it opens the container. The index pins every
file: sizes, and for each chunk its CRC32 or, with a key, its tag, and in a
folder build each data file's content hash and length. The same project,
settings and key give the same id, and any change to a file gives another.

## Keys and nonces

From the 32-byte master key, HKDF-SHA256 (salt `"zenith-archive-v1"`) derives,
each with its own label (the implementation is written out on top of `sha2` so
that every key-derived buffer can be wiped; it is tested against the `hkdf` and
`hmac` crates and the RFC 4231 and 5869 vectors): the encryption key (`zenith/v1/enc`), the nonce key
(`zenith/v1/nonce`), the names key (`zenith/v1/names`) and the 16-byte key check
(`zenith/v1/check`).

* Nonce (12 bytes): the first 12 bytes of
  `HMAC-SHA256(nonce_key, ident || 0x00 || chunk_number u64)`.
* Associated data: the 48 header bytes, `ident` length (u16) and bytes, and the
  chunk number (u64).
* `ident` is the entry path for chunks, `FF "index"` for the index and
  `FF "table"` followed by the path for a data file's table. `FF` never starts a
  valid UTF-8 path, so these cannot collide with a path.

Because the path is part of the associated data, a data file or chunk moved to
another path fails to authenticate. A chunk or a data file of another *version*
of the same file does not fit either: the chunk's tag and the data file's
`content_hash` are listed in the (authenticated) index and chunk table. (In an
*unencrypted* folder build nothing binds a data file to its path except the size
and content hash checks against the index, which protect against mix-ups, not
against an attacker.)

## Limits and rules checked on read

Path at most 1024 bytes and each segment at most 255, not ending in a dot or a
space and not a Windows device name (`CON`, `NUL`, `COM1`, ...); no two paths
that differ only by letter case; at most 2^22 entries;
entry size at most 2^48; stored chunk length at most chunk size + chunk size/128
+ 512 + 16; the entry set must be strictly sorted (so unique), have no file/folder
conflicts and have exactly one `*.zenithproject` in the root; offsets are
contiguous and checked with checked arithmetic before use.
