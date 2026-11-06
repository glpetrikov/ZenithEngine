# Trash, `.gitignore` and the file watcher

These exist on project folders only (feature `fs`); on builds they return
`Unsupported`.

## The trash

`project.remove(path)` never deletes. It moves the file or folder (whole, as one
rename) into `Trash/` and returns a `TrashId`:

```text
Trash/0001764547200123-a1b2c3d4/
  entry.toml
  files/ferris.png          the object, under its original name
```

```toml
# entry.toml
original = "Assets://tex/ferris.png"   # a ZPath, so the trash survives moving the project folder
deleted_at_ms = 1764547200123
kind = "file"                          # or "dir"
size = 18234                           # for a folder: sum of its files
ignored = false                        # the folder was listed in the managed .gitignore block
```

There is no index: the state of the trash is what is on disk, so it cannot go
out of sync.

* **`TrashId`**: 16 digits of Unix milliseconds, a dash, 8 hex digits. Plain
  string order is deletion order (ids never repeat within a process, even in
  the same millisecond).
* **Crash safety**: `remove` creates the entry folder and writes `entry.toml`
  first, then moves the object; `restore` moves the object back first, then
  deletes the entry folder. An entry without an object is skipped by
  `trash_list` and cleaned by `purge_all`.
* Scheme roots (`Assets://`) cannot be removed (`Unsupported`).

```rust,ignore
let id = project.remove(&ZPath::parse("Assets://tex/ferris.png")?)?;

project.trash_list()?;                               // newest first
project.trash_find(&ZPath::parse("Assets://tex/ferris.png")?)?;
project.restore(&id)?;                               // back to the original place
project.restore_last()?;                             // the newest entry

project.restore_with(&id, RestoreOptions {
    to: Some(ZPath::parse("Assets://other/ferris.png")?),   // None = original place
    on_conflict: OnConflict::KeepBoth,
})?;

project.purge(&id)?;       // delete one entry for good
project.purge_all()?;      // empty Trash/
```

`restore` creates missing parent folders. If the original scheme is no longer
registered it fails with `SchemeNotRegistered` (use `restore_with` and `to`).
`OnConflict`: `Fail` (default), `KeepBoth` (`ferris-1.png`, `ferris-2.png`, ...)
or `Overwrite` (the existing object goes to the trash first, so nothing is lost).
`purge` and `purge_all` are the only ways to delete anything for good.
`purge_all` deletes every entry, damaged ones included (a folder named like an
entry), and nothing else: a file that someone keeps in `Trash/` under another
name is left alone.

`Trash://<id>/files/<name>` is readable but read-only (see
[Working with a project](./using.md#operations)).

## Output folders and `.gitignore`

`project.ensure_ignored_dir(path)` creates a folder (with parents). If the
folder is inside the project and the project has a `.gitignore`, it adds an
anchored entry such as `/Build/`. A folder outside the project is only created.
`zenith_build` calls it for the parent of every output (for the output itself if it is a folder build straight in the project root).

A new project's `.gitignore` starts with `Temp/`, `Trash/`, `.zenitheditor-backup/`
(where the editor puts changes that were not saved, see [the editor](../editor.md))
and `*.zenith-tmp` (the leftover of a write that was interrupted half-way: files are written to a
`.<name>.<random>.zenith-tmp` next to the target and renamed). Older projects
can add the last line themselves.

Zenith keeps its entries in one block at the end of `.gitignore`:

```gitignore
Temp/
Trash/
.zenitheditor-backup/
*.zenith-tmp
# zenith:begin
/Build/
/dist/
# zenith:end
```

* Lines outside the block are never touched. An entry that already exists
  outside the block (or a parent that is ignored) is not duplicated.
* If there is no `.gitignore`, none is created and nothing is added.
* Line endings (LF/CRLF) and a missing final newline are kept.
* `project.sync_gitignore()` removes block entries whose folder no longer exists
  and removes the block when it becomes empty. It runs after `remove` and
  `move_to`, when the watcher starts or sees a listed folder disappear, and on
  demand. `Project::open` does not run it: opening only reads.
* A damaged block (a begin without an end, or the other way round, or more than
  one block) is `InvalidGitignore` for `ensure_ignored_dir` and an explicit
  `sync_gitignore`, and the file is left unchanged. Everywhere it runs
  implicitly it only becomes a [warning](./using.md#warnings).
* Removing a folder that was in the block records `ignored = true` in
  `entry.toml`; restoring it adds the entry again. If the block is damaged by
  then, the restore still succeeds (the folder is back, the trash entry is
  gone) and the problem is recorded as a [warning](./using.md#warnings).

## The file watcher

```rust,ignore
let watcher = project.watch(&WatchConfig::File)?;      // rules from Settings/Build.toml
let watcher = project.watch(&WatchConfig::Manual(vec![WatchRule {
    name: "scripts".into(),
    patterns: vec!["Assets/Scripts/**/*.rs".into()],
    exclude: vec![],
}]))?;

for event in watcher.events().try_iter() {
    match event {
        ProjectEvent::Change(change) => { /* change.kind, change.path (a ZPath), change.rules */ }
        ProjectEvent::Rescan => { /* events were lost: treat everything as changed */ }
    }
}
```

`Settings/Build.toml` holds `[[watch]]` tables (other tools keep their own
sections in the same file, so unknown keys are ignored):

```toml
[[watch]]
name = "scripts"
patterns = ["Assets/Scripts/**/*.rs"]   # globs relative to the project root, `/` separators
exclude = ["**/generated/**"]           # optional
```

If the file does not exist there are no rules, the watcher reports nothing, and
the file is not created.

* One recursive OS watcher on the project root (`notify` with
  `notify-debouncer-full`, 200 ms window). The thread belongs to the debouncer;
  dropping the `Watcher` stops it.
* Always ignored, whatever the rules say: `Temp/`, `Trash/`, `Build/`, `.git/`
  and every folder in the managed `.gitignore` block.
* An event is delivered if at least one rule matches; `change.rules` lists the
  names. `ChangeKind` is `Created`, `Modified`, `Removed` or
  `Renamed { from }`. A rename the OS did not pair is a `Removed` plus a
  `Created`. Moving a file *into* an ignored folder (what `remove` does) is a
  `Removed`; moving it *out* (`restore`) is a `Created`.
* Paths outside every scheme (the manifest, `.gitignore`) are not reported.
* Changes you make through `Project` arrive like any other, unmarked, and can
  repeat. `Modified`, `Removed` and `Renamed` also drop the cache entry (the
  `(mtime, size)` check keeps the cache correct without the watcher).
* The queue is bounded (1024). When it overflows, it is cleared and a single
  `Rescan` is queued; nothing else is queued until you take the `Rescan`.
* `events()` returns an `EventReceiver`: `try_recv`, `try_iter` (never block)
  and `recv_timeout`.
