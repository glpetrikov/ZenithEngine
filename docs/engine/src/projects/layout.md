# Project folder and manifest

## Layout on disk

`Project::create("Game")` makes this (the layout is defined once, as constants
in `zenith_filesystem::layout`; other crates take it from there):

```text
Game/
  Assets/
    Worlds/
  Packages/
  Settings/
  Temp/
  Trash/
  .gitignore            Temp/, Trash/, .zenitheditor-backup/ and *.zenith-tmp (one per line)
  .v1                   empty flag file: folder structure version 1
  Game.zenithproject    the manifest
```

* The manifest is named after the folder: `Game/Game.zenithproject`. Opening a
  folder whose manifest has another name, or whose manifest says another name,
  is an error.
* `.v1` is the **folder structure** version. It is separate from the `version`
  number *inside* the manifest, which is the version of the manifest format.
  A folder without `.v1` is not a project; a folder with another `.vN` flag is
  reported as `UnsupportedProjectVersion`. There are no migrations.
* `Build/` is not created with the project. It appears when a build asks for it
  (see [Building a project](../building/building.md)).
* `Temp/` and `Trash/` are meant to stay out of version control; the
  generated `.gitignore` says so.

`Project::create` refuses a folder that exists and is not empty
(`ProjectAlreadyExists`) and names that cannot be a folder and file name
(`BadProjectName`: empty, over 128 bytes, starting with a dot, ending with a dot
or space, a Windows device name such as `CON`, `NUL` or `COM1`-`COM9` (with any
extension), or containing `/ \ : * ? " < > |` or control characters).

Only the folders above are created. Folders such as `Assets/Scripts` are made
by whoever needs them; the engine does not create them.

The project name is the name of the folder. If the folder is reached through a
symlink or junction with another name, the name of the link is tried when the
real folder's name has no manifest.

## The manifest

```toml
[project]
name = "Game"
project_version = "0.1.0"
engine_version = "^0.1.0"
version = 1
user_data = "Studio/Game"   # optional
```

| Field             | Meaning                                                                                                                                                                                                                                                                                         |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `name`            | Project name; must equal the folder name.                                                                                                                                                                                                                                                       |
| `project_version` | The game's own version (semver).                                                                                                                                                                                                                                                                |
| `engine_version`  | Which engine versions the project is for (a semver requirement). A project for another engine version does not open: `EngineVersionMismatch`. (Migrations will convert old projects; until then `OpenOptions::ignore_engine_version()` lets a tool read one anyway.) The same holds for builds. |
| `version`         | Manifest format. Only `1` exists; anything else is `UnsupportedProjectVersion`.                                                                                                                                                                                                                 |

`user_data` is optional: the folder of the data of the player (`Data://`, see
[Saves](../saves.md)) below the data folder of the system (`dirs::data_dir()`:
`%APPDATA%` on Windows, `~/.local/share` on Linux, `~/Library/Application Support`
on macOS), a name or a path of names such as `Studio/Game`. Without it the folder
is called like the project. Choose it once and keep it: the saves of the players
stay where they are if the game is renamed. A name with `..`, a drive or
anything a platform cannot name is refused when the project is opened.

`Project::manifest()` returns it as `zenith_filesystem::Manifest`
(`manifest.user_data_folder()` is the folder in effect).

## Which kind of project is it?

`Project::open(path)` looks at the path and picks the backend:

| What is at the path                    | Backend                                 | Writable |
| -------------------------------------- | --------------------------------------- | -------- |
| A folder with `.v1`                    | `Fs`: the project folder                | yes      |
| A folder with `build.index`            | `BuildFs`: a build as a folder of files | no       |
| A file starting with the archive magic | `BuildArchive`: a build as one file     | no       |
| Anything else                          | `UnknownProjectFormat`                  |          |

A folder with both `.v1` and `build.index` is ambiguous and is rejected as
`UnknownProjectFormat`. `Project::kind()` reports the backend
(`BackendKind::{Fs, BuildArchive, BuildFs}`).
