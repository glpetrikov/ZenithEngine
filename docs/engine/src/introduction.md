# Introduction

A Zenith **project** is a folder on disk that the editor edits. A **build** is
that project turned into something a game can ship: one file, or a folder of
files. Three crates deal with this:

| Crate               | What it does                                                                                                                           |
| ------------------- | -------------------------------------------------------------------------------------------------------------------------------------- |
| `zenith_filesystem` | Opens projects and builds, resolves `scheme://path` names, reads and writes files, keeps the trash, `.gitignore` and the file watcher. |
| `zenith_archive`    | The two build containers (a single file and a folder of files), the chunk codec, encryption. It knows nothing about projects.          |
| `zenith_build`      | `Builder`: decides which files go into a build and writes it through `zenith_archive`.                                                 |

```text
zenith_error ◄── zenith_archive
zenith_error ◄── zenith_filesystem ──► zenith_archive
zenith_build ──► zenith_filesystem, zenith_archive (feature `write`)
```

`zenith_archive` does not depend on the other two; `zenith_filesystem` only
needs the *reading* half of `zenith_archive`; only `zenith_build` needs the
writers.

## Quick start

Create a project, write a file, build it, open the build:

```rust,ignore
use zenith_build::{BuildFormat, Builder};
use zenith_filesystem::{OpenOptions, Project, ZPath};

// 1. Create the project (the folder name is the project name).
let project = Project::create("Game")?;

// 2. Write and read files through schemes.
project.write(&ZPath::parse("Worlds://Main.zenith")?, b"...")?;
let bytes = project.read_bytes(&ZPath::parse("Assets://Worlds/Main.zenith")?)?;

// 3. Build it: by default into Build/game.zenithbuild inside the project
//    (the same name for every project; see "Which name does the runtime open?").
let report = Builder::new().path("Game").build()?;
println!("wrote {} ({} files)", report.output.display(), report.entries);

// 4. Open the build exactly like a project (read-only).
let game = Project::open(&report.output)?;
let same = game.read_bytes(&ZPath::parse("Worlds://Main.zenith")?)?;
```

The sections that follow cover each part in detail. Every `ZenithError`
variant mentioned is a variant of `zenith_error::ZenithError`.

## Where the code lives

* Project API: [`crates/zenith_filesystem`](https://github.com/glpetrikov/ZenithEngine/tree/main/crates/zenith_filesystem)
* Containers and codec: `crates/zenith_archive`
* Builder: `crates/zenith_build`

Run `cargo doc --open -p zenith_filesystem -p zenith_build -p zenith_archive`
for the API reference; every public function documents its errors.
