# Building a project

`zenith_build::Builder` turns a project folder into a build. The project on
disk is neither copied nor changed (the only exception is adding the output
folder to `.gitignore`).

```toml
[dependencies]
zenith_build = { path = "crates/zenith_build" }
```

```rust,ignore
use zenith_build::{BuildFormat, Builder, EncryptionKey, ZPath};

let report = Builder::new()
    .path("Game")                          // the project folder (a ready path, `~` is not expanded)
    .output("dist/Game.zenithbuild")       // optional
    .format(BuildFormat::Archive)          // or BuildFormat::Fs
    .zstd_compression(19)                  // 1..=22, default 9
    .threads(8)                            // optional: how many threads compress (default: automatic)
    .hash_cache(true)                      // optional: remember file hashes between folder builds (default true)
    .encryption_key(key)                   // optional, see "Encryption and keys"
    .exclude("Assets/Scripts/**")          // rules, see "Build rules"
    .include("Docs/**")
    .add_file(dest, src)
    .add_bytes(dest, bytes)
    .build()?;

report.output;      // where it was written
report.entries;     // files in the build
report.compressed;  // files that were compressed
report.reused;      // files a folder build took over from the previous build
report.build_id;    // the identity of the build, see "Pinning a release"
report.warnings;    // problems after the build was complete (see "Folder builds")
```

These are all the methods. `path` is the only required one. Mistakes in options
(a level outside 1..=22, a bad glob, a missing path) are reported by `build()`,
not by the chain.

## The two formats

| | `BuildFormat::Archive` (default) | `BuildFormat::Fs` |
|---|---|---|
| Result | one file | a folder: `build.index` plus one file per project file |
| Default `output` | `<project>/Build/game.zenithbuild` | `<project>/Build/game/` |
| Good for | the web, simple distribution | Steam and other updaters: an update only changes the files that changed |
| Rebuild | always writes everything | skips what did not change (see below) |

Both are **reproducible**: entries are sorted by path, no modification times
are stored, and compression and encryption are deterministic, so the same
project, rules, settings and key give byte-identical output. For `Fs` builds
that is what keeps the files of an unchanged game file identical from one
release to the next.

### Which name does the runtime open?

The default name is the same for every project: `Build/game.zenithbuild` for an
archive and `Build/game/` for a folder (the constants are
`layout::DEFAULT_BUILD_NAME` and `layout::BUILD_FILE_EXTENSION` in
`zenith_filesystem`). The project's own name is not part of the file name; it is
read from the manifest inside the build. So a game can always do:

```rust,ignore
let game = Project::open("game.zenithbuild")?;   // or Project::open("game")? for a folder build
```

and a release script only has to copy `Build/game.zenithbuild` (or the folder)
next to the executable. The file name is not stored inside the build, so you
are free to rename the result. Pass `output(...)` to choose another name or place.

## What goes into a build

The base list is the manifest plus everything in `Assets/`, `Settings/` and
`Packages/` (defined in `zenith_filesystem::layout`). `Temp/`, `Trash/`,
`Build/`, `.gitignore` and `.v1` are not included unless a rule asks for them.
Rules change that: see [Build rules](./rules.md).

Scripts are compiled **before** the build, into `Temp/Build/scripts.wasm`. The
builder knows nothing about the compiler or the settings format; you swap the
sources for the compiled file and pass changed settings as bytes:

```rust,ignore
Builder::new()
    .path("Game")
    .exclude("Assets/Scripts/**")
    .add_file(ZPath::parse("Assets://Scripts/scripts.wasm")?, ZPath::parse("Temp://Build/scripts.wasm")?)
    .add_bytes(ZPath::parse("Settings://Scripts.toml")?, patched_settings)
    .build()?;
```

## Where the output may go

`output` is an ordinary path (not a `ZPath`), a file for `Archive` and a
folder for `Fs`. It may be anywhere **except**:

* inside `Assets/`, `Settings/` or `Packages/`, inside `.git/` or `Trash/`, or
  one of the files that make the folder a project (`.v1`, `.gitignore`);
* the manifest, or any file the build reads (for example with `include("Build/**")`
  the build would read its own output);
* the project folder itself or a folder that contains it
  (`InvalidBuildOutput`);
* an existing folder for an `Archive`, or an existing file for an `Fs` build
  (`InvalidBuildOutput` / `NotABuildDir`).

Before writing, the builder calls `ensure_ignored_dir` for the **parent** of the
output. A default build therefore adds `/Build/` to the project's managed
`.gitignore` block; an output outside the project only gets its folder created.
A folder build (`Fs`) that sits straight in the project root (`Game/dist`) is its
own folder, so that folder is ignored instead. An archive written straight into
the project root is not ignored: the managed block lists folders only.

## Archive builds

The archive is written to `<output>.zenith-tmp`, flushed to disk and renamed into
place, so a failed build (or a power loss) never leaves half an archive under
the real name.

It is compressed on several threads and comes out byte for byte the same as
with one. Small files (up to 8 MiB) are compressed whole, a window of them
(64 MiB at most) at a time, one file per thread; a bigger file is cut into
chunks that are compressed side by side. Entries are appended in path order.
`threads(n)` sets the number of threads; the default is one per processor, fewer
if the memory zstd needs at this level and chunk size (estimated by
`WriterOptions::thread_memory`) would not fit in two gigabytes.

## Folder builds

`Fs` is where rebuilding gets cheap. Build into the same folder again and again:

1. Every source file is hashed (BLAKE3), unless the hash cache has it: a file
   whose size and modification time are what they were when its hash was
   recorded, and which was already two seconds old then, keeps its hash
   (`Temp/Build/hash-cache.txt`, never part of a build). A project that has not
   changed is then not read at all. The cache trusts size and modification time
   as `make` does, so a file whose content changed while both were put back is
   not noticed: turn it off with `hash_cache(false)` for builds from tools that
   do that. If the previous build's `build.index`
   lists the same path with the same content hash, the same compression level
   and chunk size, and its data file is still there and intact (same length, valid
   header, footer and chunk table; the chunks themselves are not read), the file
   is **neither compressed nor written** (`report.reused`). A data file that was
   damaged since is made again.
2. The others are compressed, `threads(n)` files at a time (the default is
   chosen as for archives), into temporary files named `<data file>.zenith-tmp`;
   an entry whose own name ends in `.zenith-tmp`, or is
   `build.index.next` or `build.index.prev`, is refused (`InvalidBuildOutput`)
   for that reason. If any file fails, the temporary
   files are deleted and the folder stays as it was.
3. The new `build.index` is encoded. If that fails, the folder stays as it was.
4. The commit. A folder of many files cannot change all at once, so the order
   is chosen so that an interruption (a crash, a power loss) is *visible and
   repairable* instead of leaving an old index over a mixture of old and new
   files:
   1. the new index is written, and flushed to disk, as `build.index.next`;
   2. the old index steps aside as `build.index.prev`: from now on the folder
      has no `build.index`, so it is plainly not a finished build (opening it is
      `UnknownProjectFormat`);
   3. the staged files replace the data files (a file whose new bytes equal the
      old ones is not rewritten, so its modification time stays);
   4. `build.index.next` becomes `build.index`, and `build.index.prev` is
      deleted.

   If step 2 or 3 fails before any data file was replaced, the old index is put
   back and nothing has changed. If it fails later the build returns
   `BuildIncomplete` and leaves the folder without an index; the temporary files
   that are left are deleted either way.
5. Only now are data files that the old index listed and the new one does not
   deleted (and the folders they leave empty). One that cannot be deleted (the
   game is running from it) is listed in `report.warnings`; the build is complete
   and valid. **Other files in the folder are never deleted.**

The output folder must be empty, not exist, or hold a previous build:

| Situation | Result |
|---|---|
| Does not exist / is empty | created / used |
| Holds a build (`build.index`) made with the same key mode | incremental rebuild |
| Non-empty, no `build.index`, and no `build.index.next` or `build.index.prev` | `NotABuildDir`; nothing is changed |
| No `build.index`, but `build.index.next` and/or `.prev` (a build that was interrupted while it was replacing files) | repaired: the files named by those indexes are this build's own and are overwritten, none is reused, everything is compressed again, and the folder ends up exactly as a build from scratch |
| `build.index` and a leftover `.next` or `.prev` | the good index is the previous build; the leftovers are replaced and deleted |
| `build.index` cannot be read: another key, encrypted versus not, damaged | `PreviousBuildUnreadable`; nothing is changed (the folder is *not* cleaned) |
| A foreign file already has the name of a data file this build needs | `InvalidBuildOutput`; nothing is changed |

Consequences worth knowing:

* Changing the **compression level** recompresses every file (the level is
  recorded per entry). Changing the **key** is refused for an existing folder:
  build into a new folder, or delete the old one.
* The result of an incremental build is identical to a build from scratch.
  The one exception: data files that were *reused* keep the bytes the compressor
  produced at the time, so after upgrading the zstd library (a new
  `Cargo.lock`) do a clean build into an empty folder before you rely on
  byte-for-byte reproducibility.
* Keep the same output folder (or the previous release's files) between
  releases, and the same key: that is what turns an update into "download the
  files that changed".

## Shipping the engine with the build

`engine_exe` copies the engine's executable into the folder that holds the
output (next to the archive, or next to the folder of an `Fs` build), so the
result can be run or shipped as it is:

```rust,ignore
Builder::new()
    .path("Game")
    .output("dist/Game.zenithbuild")
    .engine_exe("C:/Users/me/AppData/Local/ZenithEngine/Versions/1/zenith_engine.exe")
    .rename_exe(RenameExe::ProjectName)      // dist/Game.exe
    .build()?;
```

* The path is used as given (`~` is not expanded), and a missing file is an
  `InvalidBuildOption` before anything is written.
* Without `rename_exe` the copy keeps its name. `RenameExe::ProjectName` uses
  the name from the manifest, `RenameExe::Name("My Game".into())` any name
  without an extension: the extension of the original is kept (`.exe` stays
  `.exe`). Names with slashes, `: * ? " < > |`, control characters or a
  trailing dot or space are refused.
* The copy goes through a temporary file, an existing copy is replaced, and the
  executable bit is kept. Replacing an executable that is running fails on
  Windows; that is reported as an I/O error.
* `BuildReport::engine_exe` holds the path of the copy.

### The browser runtime

`web_runtime` does the same for the web: it copies the output of
`wasm-pack build --target web` next to the build, into `pkg/` (the `.js`, the
`.wasm` and `snippets/`), and writes an `index.html` that starts it on a
`<canvas id="zenith-canvas">`. `web_index` uses a page of your own instead; it
can load `./pkg/<name>.js`.

```rust,ignore
Builder::new()
    .path("Game")
    .format(BuildFormat::Archive)
    .output("dist/Web/game.zenithbuild")
    .web_runtime("bins/zenith_runtime/pkg")
    .web_index("bins/zenith_runtime/index.html")
    .build()?;
```

The executable and the browser files are copied byte for byte: they are never
compressed or encrypted, whatever the build is. `BuildReport::web_files` lists
what was written.

## Pinning a release

`report.build_id` is a `BuildId`: 32 bytes (`to_hex()` gives 64 hex digits) made
from the build's header and its index, which lists every file with what pins its
exact bytes. The same project, rules, settings and key give the same id, and
anything else gives another. It is what tells a *complete older release* from the
current one, which the files of a build cannot do themselves:

```rust,ignore
// At release time: keep the id where the player cannot change it, for example
// compiled into the game.
let id = report.build_id;

// In the game:
let project = OpenOptions::new()
    .expected_build_id(id)
    .open("game.zenithbuild")?;   // any other build: BuildIdMismatch
```

The build tool reads the finished output back to get the id, so a build that
cannot be opened with its key fails the build instead of shipping.

## Compression level

The level (default 9) trades build time and size. It barely matters for how
fast the game *reads* the data: decompression speed of zstd does not depend
much on the level, and our reader (`ruzstd`) decodes 200 to 440 MB/s per thread
on compressible data. Use 19 for release builds if you can wait, a low level
for quick test builds.

### Without compression

`compress(false)` (default `true`) stores files as they are. The build is
bigger and the level is ignored, but a file can be found in a hex dump or a
text editor, which is handy in tests:

```rust,ignore
Builder::new().path("Game").format(BuildFormat::Fs).compress(false).build()?;
```

Each file is still inside the container (header, chunk table, footer), so it is
not a plain copy: open the build with `Project::open` to read it normally.
With an encryption key the content is encrypted whether it is compressed or
not. Switching `compress` makes a folder build write every file again once.
The mode is recorded in the build (the compression id of the header and level
`0` in the index of a folder build), so readers need no setting; builds made
before this option are read as before.

## Errors specific to building

`InvalidBuildOption`, `InvalidBuildOutput`, `NotABuildDir`,
`PreviousBuildUnreadable`, `BuildIncomplete`, `SymlinkInBuild`; plus the project errors
(`ProjectNotFound` for a missing `add_file` source, `SchemeNotRegistered`
for an unknown scheme) and archive errors.
