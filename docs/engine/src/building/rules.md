# Build rules

A build is a virtual list of entries. Rules edit that list, **in the order you
call them**, and for each path **the last rule that applies to it wins**.

| Call | Effect |
|---|---|
| `exclude(glob)` | Removes the matching entries. |
| `include(glob)` | Adds the project files matching the glob: files removed earlier and files outside the base list too. Only files that exist in the project are added. |
| `add_file(dest, src)` | Puts the project file `src` into the build at `dest` (replacing an entry already there). |
| `add_bytes(dest, bytes)` | Puts `bytes` into the build at `dest` (replacing an entry already there). |

The starting list is the manifest plus `Assets/**`, `Settings/**` and
`Packages/**`.

## Globs

Globs are relative to the **project root** and use `/` as the separator
(`Assets/Scripts/**`, `Docs/*.md`, `Assets/**/*.png`). They are `globset`
patterns with one setting: `*` and `?` do not cross `/`, `**` does. Braces
(`{a,b}`) and classes (`[ab]`) work. A bad glob is `InvalidBuildOption`.

A glob matches files only, and only files the walk can see. If no `include` rule
is present the builder does not even walk `Temp/` or `Trash/`. The `.git/`
folder is never walked: no glob can reach into the repository.

## Order matters

```rust,ignore
// Everything in Assets except Worlds is gone, Worlds comes back:
.exclude("Assets/**").include("Assets/Worlds/**")

// The other way round, the world is excluded again:
.include("Assets/Worlds/**").exclude("Assets/**")

// An added entry can be removed again by a later exclude:
.add_bytes(settings, bytes).exclude("Settings/Game.toml")
```

## `add_file` and `add_bytes`

Both paths are `ZPath`s resolved through the project's **built-in** schemes
(the builder opens the project itself, so custom schemes you registered on
another handle do not exist here). `dest` decides the path inside the build:
`Assets://Scripts/scripts.wasm` is `Assets/Scripts/scripts.wasm`.

* `src` must exist, must be a file and must not be a symlink.
* `dest` may not be in the trash.
* The same `dest` twice: the later call wins.
* The files on disk are never changed: patched settings are only in the build.

## Symlinks

A symlink among the entries is `SymlinkInBuild`; there is no silent skipping.
Exclude it, or replace it with `add_file`/`add_bytes`. (On Windows junctions are
links too.)

## The entry set is validated

Before anything is written the final list must be valid: paths relative, with
`/`, no `.`/`..`/empty segments, no `\`, `:` or control characters, each at most
255 bytes per segment and 1024 in total; no segment that ends in a dot or a
space or is a Windows device name (`CON`, `NUL`, `COM1`, ..., with any
extension); no two paths that differ only by letter case (they would be one file
on Windows and macOS); no path that is both a file and a folder; **exactly one
`*.zenithproject` in the root** (so excluding the manifest is an error). A plain
`Fs` build also refuses a root file named `build.index`.
