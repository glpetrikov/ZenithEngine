# Working with a project

```toml
[dependencies]
zenith_filesystem = { path = "crates/zenith_filesystem" }   # feature `fs` is on by default
zenith_error = { path = "crates/zenith_error" }
```

`Project` is `Send + Sync`, every method takes `&self`, and the API is
synchronous. One handle can be shared between threads.

## Creating and opening

```rust,ignore
use zenith_filesystem::{CacheConfig, CacheSettings, OpenOptions, Project};

// New project folder with the standard layout. The folder name is the project name.
let project = Project::create("Game")?;

// Open anything: a project folder, a build file, a build folder.
let project = Project::open("Game")?;

// With options (they chain, then `.open(path)`):
let project = OpenOptions::new()
    .cache_config(CacheConfig::Manual(CacheSettings::default()))
    .encryption_key(key)            // only used for encrypted builds
    .max_read_size(256 * 1024 * 1024) // optional: the biggest file read_bytes loads
    .ignore_engine_version()          // optional: open a project of another engine version
    .expected_build_id(id)            // optional: refuse every build but this one
    .open("dist/Game.zenithbuild")?;

// Web: the bytes of a build that was downloaded.
let project = Project::open_archive_bytes(bytes /* Arc<[u8]> */, &OpenOptions::new())?;
```

Other constructors: `Project::open_archive(src, &opts)` for any
`zenith_archive::ReadAt` source, and `Project::open_build_dir(source, &opts)`
for a folder build whose files come from your own `DirSource` (for example files
fetched in a browser; `MemDir` is an in-memory one).

`Project::open` only reads: it does not edit the `.gitignore` (a tool that opens
a project, such as a build on a CI machine, must not change the working tree).
It reads the managed block; a damaged block never makes `open` fail, it only
produces a [warning](#warnings). Stale entries are removed by `sync_gitignore`,
which `remove`, `move_to` and the watcher call.

A project whose manifest is for another engine version does not open
(`EngineVersionMismatch`), neither does a build; `OpenOptions::ignore_engine_version()`
opens it anyway, for a tool that has to read an old project to convert it.

An encryption key that a build does not need is **ignored** by `Project` (it
reads the encryption flag from the header first). A missing or wrong key for an
encrypted build is an error (`ArchiveEncryptedNoKey` / `ArchiveWrongKey`).

## Operations

| Method                                                              | Notes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| ------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `exists`, `list`, `metadata`, `is_symlink`                          | `list` is sorted by name; entries are `File`, `Dir` or `Symlink`. `metadata` follows links. A file whose name is not valid UTF-8 is not listed (a `ZPath` cannot name it); `copy` and the walk that `zenith_build` does fail on it instead of leaving it out.                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `read_bytes` → `Arc<[u8]>`, `read_string`                           | Go through the [cache](#cache). `read_string` fails with `InvalidUtf8` for binary data. A file bigger than `OpenOptions::max_read_size` (default 1 GiB) is `FileTooLarge` before anything is read; this is what stops a hostile build whose index states an absurd size. Use `open_read` for bigger files.                                                                                                                                                                                                                                                                                                                                                                                                          |
| `open_read` → `Read + Seek + Send`                                  | Streaming, never cached.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `write(path, &[u8])`                                                | Atomic: written to a temporary file next to the target, flushed to disk, then renamed. Missing parent folders are created. Writing onto a folder is `ProjectWrongKind`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             |
| `create_dir`                                                        | Creates parents too.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `copy`, `move_to`                                                   | Files and folders, within one project, also across schemes. The destination must not exist (`ProjectAlreadyExists`); a folder cannot go into itself. `copy` may read from `Trash://`. A folder that cannot be copied completely (it holds a link, or a file whose name is not valid UTF-8) is not copied at all: what was made is removed again. A destination that another process creates while the copy or move runs is never replaced: a file is put in place with a hard link, which fails if the name is taken (on a file system without hard links, after a last look), a folder with `create_dir`; the one thing a folder move can replace is an empty folder. (`move` is a Rust keyword, hence `move_to`.) |
| `remove` → `TrashId`                                                | Moves to the trash; nothing is deleted for good. See [the trash](./trash-gitignore-watch.md).                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `register_scheme`, `manifest`, `kind`, `warnings`, `cache_settings` |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `build_id`                                                          | The identity of a build; `None` for a project folder.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `save_cache_config`                                                 | Writes the settings in effect to `Settings/CacheConfig.toml`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                       |
| `walk_project`, `open_project_file`, `project_path`                 | For tools that treat the whole folder as input (this is what `zenith_build` uses): list everything under the project root including places no scheme covers, read by project-relative path, turn a `ZPath` into a project-relative string.                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

On builds (`BuildArchive`, `BuildFs`) every changing method returns
`ReadOnly`, and the trash, `.gitignore` and watcher methods return
`Unsupported`. Anything under `Trash://` is read-only: `exists`, `list`,
`metadata`, `read_*`, `open_read` and `copy` *out of it* work, everything else is
`ReadOnlyScheme`.

## Symlinks and the project boundary

The project root is opened once as a `cap_std::fs::Dir` and every operation
goes through that handle. Consequences:

* `..`, absolute paths and symlinks that lead outside the project cannot be
  followed: the operation fails (`PathEscapesRoot`), whatever the path says.
* Symlinks **inside** the project work, if their target is a *relative* path.
  A link with an absolute target is refused even when it points inside the
  project.
* Reading and writing go through a link (writing to `Assets://alias.txt` changes
  the file it points to and keeps the link). `list` shows links as `Symlink`;
  `is_symlink` tells for one path. `copy` refuses to copy a link.
* The plain `PathBuf` of the project root is kept only for the file watcher
  (the OS API needs it); no file is ever opened through it.

On Windows, junctions count as links.

## Cache

`read_bytes` and `read_string` are cached; `open_read` and `Trash://` are not.

* The key is the backend plus the **resolved** path, so `Assets://Worlds/A.zenith`
  and `Worlds://A.zenith` share one entry.
* A file enters the cache on its **second** read (the first one only remembers
  the key, at most `max_tracked_keys` of them), so a file that is read once never
  pushes useful files out. Eviction is LRU.
* On the `Fs` backend a hit is checked against the file's `(modification time,
  size)` and re-read if it changed; `write`, `move_to`, `remove` and `restore`
  drop the entry, and so do watcher events. Builds are immutable, so no check is
  needed.

Settings:

| Field                    | Default |
| ------------------------ | ------- |
| `min_reads_before_cache` | 2       |
| `max_file_size`          | 4 MiB   |
| `total_budget`           | 256 MiB |
| `max_tracked_keys`       | 4096    |

Where they come from is chosen at open time: `CacheConfig::File` (default) reads
`Settings/CacheConfig.toml`, and uses the defaults, without creating the file,
if it does not exist; `CacheConfig::Manual(settings)` uses your values and never
touches the disk. On a build, `File` reads the file from inside the build if the
build contains it.

```toml
# Settings/CacheConfig.toml: all keys optional, unknown keys are an error
min_reads_before_cache = 2
max_file_size = 4194304
total_budget = 268435456
max_tracked_keys = 4096
```

## Warnings

Things that went wrong without failing an operation are collected and logged
(through `zenith_log`); `project.warnings()` returns them, oldest first. The
same warning is kept once, however often the operation that causes it runs.
Today that is:

* a damaged managed `.gitignore` block (`WarningKind::DamagedGitignore`) or an
  unreadable `.gitignore` (`WarningKind::GitignoreIo`), also when it happens
  during `restore`, after the folder has been put back;

## Errors

Everything returns `zenith_error::ZResult<T>`, i.e. `ZenithError`. The ones you
will meet:

| Variant                                                                                        | When                                                                                                |
| ---------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------- |
| `InvalidZPath`, `InvalidScheme`, `SchemeNotRegistered`, `SchemeAlreadyRegistered`              | Names and schemes.                                                                                  |
| `ProjectNotFound`, `ProjectAlreadyExists`, `ProjectWrongKind`                                  | Missing, existing, or file-versus-folder mix-ups.                                                   |
| `PathEscapesRoot`                                                                              | A link or path that leaves the project.                                                             |
| `EngineVersionMismatch`                                                                        | Opening a project or build whose manifest is for another engine version.                            |
| `BuildIdMismatch`                                                                              | Opening a build other than `OpenOptions::expected_build_id` (an older release, say).                |
| `ReadOnly`, `ReadOnlyScheme`, `Unsupported`                                                    | Changing a build; writing to `Trash://`; asking a build for the trash or the watcher.               |
| `UnknownProjectFormat`, `UnsupportedProjectVersion`, `InvalidProject`, `BadProjectName`        | Opening and creating.                                                                               |
| `InvalidTrashId`, `TrashEntryNotFound`                                                         | Trash.                                                                                              |
| `InvalidGitignore`, `InvalidSettings`, `Watch`                                                 | `.gitignore` block, `CacheConfig.toml` / `Build.toml`, the OS watcher.                              |
| `FileTooLarge`                                                                                 | `read_bytes` / `read_string` of a file over `max_read_size`; a settings file over 1 MiB in a build. |
| `ArchiveCorrupt`, `UnsupportedArchiveVersion`, `ArchiveEncryptedNoKey`, `ArchiveWrongKey`, ... | Builds.                                                                                             |

## Features and the web target

`zenith_filesystem` has one feature, `fs` (on by default). It gates everything
that needs a real filesystem: the `Fs` backend (`cap-std`), the trash,
`.gitignore` handling and the watcher (`notify`, `globset`). Without it only the
read-only build backends remain, which is what a browser needs:

```toml
zenith_filesystem = { path = "crates/zenith_filesystem", default-features = false }
```

```bash
cargo check -p zenith_archive -p zenith_filesystem --no-default-features --target wasm32-unknown-unknown
```

`zenith_archive` has its own feature, `write` (writers; needs a C compiler for
the zstd library). The readers never need it; reading uses the pure-Rust
`ruzstd`, so no C compiler is needed for wasm.

## Saving a world

A world is turned into the bytes of a file with `World::snapshot_bytes` (YAML,
UTF-8) and comes back with `World::restore_bytes`; the project only sees bytes:

```rust,ignore
project.write(&ZPath::new("Worlds/world.zenith")?, &world.snapshot_bytes()?)?;

let bytes = project.read_bytes(&ZPath::new("Worlds/world.zenith")?)?;
world.restore_bytes(&bytes)?;
```

Entities with no saved components are not written (the engine keeps some of
its own), so a save followed by a load never adds entities.
