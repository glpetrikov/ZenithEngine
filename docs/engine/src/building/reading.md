# Reading builds, on desktop and on the web

A build opens like a project: `Project::open(path)` (see
[Working with a project](../projects/using.md)). It is read-only, and `ZPath`s
resolve exactly as in the project folder, so game code does not care whether it
runs from a project or from a build.

```rust,ignore
let game = Project::open("dist/Game.zenithbuild")?;           // a file: BuildArchive
let game = Project::open("dist/GameFolder")?;                 // a folder with build.index: BuildFs
let level = game.read_bytes(&ZPath::parse("Worlds://Main.zenith")?)?;
let mut stream = game.open_read(&ZPath::parse("Assets://music/theme.ogg")?)?;   // Read + Seek
```

## What the readers guarantee

* Reading one file decompresses only that file's chunks; a file is cut into
  independent chunks (256 KiB by default), each a separate zstd frame, so
  `Seek` into the middle of a file decodes one chunk, not the file.
* Builds are **untrusted input**. Every size and offset is checked with checked
  arithmetic and against limits before memory is allocated; duplicate or
  malicious paths (`..`, absolute, `\`, `:`) and file/folder conflicts in the
  index are rejected; a decompressed chunk may not be larger than its declared
  size (the decoder is limited to one chunk plus one byte). Nothing panics on a
  malformed file; the tests flip every byte and cut every prefix of sample
  builds and expect an error each time.
* A file is never loaded into memory beyond `OpenOptions::max_read_size`
  (default 1 GiB). The index of a hostile build can state any size, and a tiny
  compressed chunk can decode to a whole chunk, so `read_bytes` checks the
  stated size first (`FileTooLarge`). Streaming with `open_read` decodes one
  chunk at a time and is not affected.
* A build must contain exactly one `*.zenithproject` in its root.
* Every build has an identity, `Project::build_id()` (`None` for a project
  folder). `OpenOptions::expected_build_id(id)` refuses any other build,
  including a complete, valid, older release, which nothing inside the files can
  tell from the current one. The id of a release is `BuildReport::build_id`; keep
  it where the player cannot change it. It is SHA-256 over the container header
  and the index as stored, so it changes with anything in the build.

## On the web

Without the `fs` feature `zenith_filesystem` has only the read-only backends,
and `zenith_archive` without `write` has no C code, so both build for
`wasm32-unknown-unknown`. The game downloads the build and hands over the
bytes:

```rust,ignore
let bytes: Arc<[u8]> = Arc::from(downloaded);                       // e.g. from fetch()
let project = Project::open_archive_bytes(bytes, &OpenOptions::new())?;
```

Fetching a build in parts with range requests is not supported; the whole file
has to be in memory. A folder build can be used in a browser too, through
`Project::open_build_dir` with your own `DirSource` that returns the fetched
files.

`EncryptionKey::generate` compiles for wasm (it uses `crypto.getRandomValues`
through `wasm-bindgen`). As explained in
[Encryption and keys](./encryption.md#web-builds-better-not-to-encrypt),
leaving the build unencrypted is the better choice for the web.

## Speed

Measured reading a whole file (256 KiB chunks, level 9, one thread, MB/s of
uncompressed data):

| Data | Native | Native + AES-SIV | WebAssembly (Node) | Wasm + AES-SIV |
|---|---|---|---|---|
| Text | 291 | 319 | 309 | 162 |
| Binary | 209 | 194 | 167 | 77 |
| Incompressible | 2265 | 921 | 1202 | 45 |

Decompression is `ruzstd` (pure Rust) everywhere, about three to four times
slower than the C library but fast enough for game assets and free of a C
dependency on the web. The cache (second read onwards) removes repeated
decompression.

## Using the container crates directly

`zenith_archive` does not depend on the project crates:

```rust,ignore
use zenith_archive::{ArchiveReader, FileSource};

let reader = ArchiveReader::open(FileSource::open("Game.zenithbuild")?, None)?;
for entry in reader.entries() { println!("{} {}", entry.path, entry.size); }
let data = reader.read_all("Assets/Worlds/Main.zenith")?;
let mut stream = reader.open_entry("Assets/tex/a.png")?;   // Read + Seek
```

Sources implement `ReadAt` (positional reads): `FileSource` for files and
`Arc<[u8]>` for memory. `FolderReader::open(dir_source, key)` does the same for
folder builds. The writers (`ArchiveWriter`, `FolderWriter`) are behind the
`write` feature; most code should use `zenith_build::Builder` instead.
