# Saves and the data of the player

A game has two kinds of data. The first is the author's: the project, or the
build made from it. The player must not change it, and it may sit where nothing
can be written (an install folder that an update or "verify files" wipes). The
second is the player's: the controls they changed, their saves. It has a place
of its own, reached with the `Data://` scheme of any project or build.

```rust,ignore
project.write(&ZPath::new("Data/Saves/slot1.bin")?, &bytes)?;   // works for a build too
let bytes = project.read_bytes(&ZPath::new("Data://Saves/slot1.bin")?)?;
project.delete(&ZPath::new("Data://Saves/slot1.bin")?)?;        // for good: no trash here
```

`Data://` takes `read_bytes`, `read_string`, `open_read`, `exists`, `list`,
`metadata`, `write`, `create_dir` and `delete`. It is not part of the project:
`remove`, `copy` and `move_to` refuse it (`Unsupported`), `Data` cannot be
registered as a scheme of your own, and it is never part of a build. Names are
checked like the paths inside a build (no `..`, nothing Windows cannot name,
nothing ending in `.zenith-tmp`). A file is read whole and obeys
`OpenOptions::max_read_size`.

## Where it is kept

| Where      | Kept in                                                                                      |
| ---------- | -------------------------------------------------------------------------------------------- |
| a computer | the data folder of the system for the user (`dirs::data_dir()`), then the folder of the game |
| a browser  | IndexedDB, in a database called `ZenithEngine <folder of the game>`                          |
| tests      | memory                                                                                       |

The folder of the game is `user_data` of the project's
[manifest](./projects/layout.md#the-manifest) (`Studio/Game`), or the project
name when the manifest does not say. Nothing in the engine names a place on a
particular machine: the data folder is whatever the system says it is for the
user who runs the game.

All three are a `UserStore`: whole files under `/`-separated paths, replaced
atomically, so a file is the old one or the new one and never half of each. On a
computer a write goes to a temporary file next to the target, is flushed to disk
and renamed over it. Nothing is created until something is written: opening a
game leaves no empty folders.

You can choose another place with `OpenOptions::user_data(store)` when opening,
or `project.set_user_data(store)` later (a browser has to: the database is named
after the project, which is known only once it has been read). `DirStore::new(path)`
is a folder (a portable game keeps it next to the program), `MemStore` is
memory.

### In a browser

IndexedDB can only be asked asynchronously and the project is read
synchronously, so `IdbStore::open(name).await` reads everything into memory
first (it is small: settings and saves). After that reads and writes use the
copy at once, and every write is also started in the background, in parallel
with the game. A write that has not finished is remembered, and when the page is
hidden or closed they are all started again on the spot, since nothing can be
awaited then. `store.flush_async().await` waits until everything is safe;
`project.flush_data()` only starts what is left. Without IndexedDB (a private
window) the data stays in memory and is gone with the page.

## Slots: `zenith_save`

`Saves` keeps the progress of the player in named slots, one file each,
`Data://Saves/<slot>.zenithsave`:

```rust,ignore
let saves = Saves::new(&project);                      // .encryption_key(key), .compress(false)

let save = Save::new(SaveMeta::new("Level 3, before the boss", WORLD_VERSION).with_extra("level", "3"))
    .with_entry(WORLD_ENTRY, world.snapshot_bytes()?);   // any named entries
saves.save("slot1", &save)?;

for slot in saves.slots()?.saves {                      // newest first
    println!("{}: {}", slot.slot, slot.meta.title);
}
let loaded = saves.load("slot1")?;                      // an error if the save is damaged
world.restore_bytes(loaded.entry(WORLD_ENTRY).unwrap_or_default())?;
saves.delete("slot1")?;
```

* **Header.** `SaveMeta` has the title to show, the `data_version` (yours: the
  version of your data, such as `WORLD_VERSION`; the engine does not look at it,
  compare it yourself before restoring), `extra` for anything else to show
  without opening the save, and, filled in when saving, the version of the
  engine and the time.
* **Container.** A save is a container of the same format as a build:
  compressed (zstd, level 3 by default), encrypted if you give a key. Its
  header is in the root file `Save.zenithproject`, which the format asks for;
  your entries may have any other name. A key stored in a game that anyone can
  download stops nobody who looks for it, but it keeps a save from being edited
  with a text editor.
* **Safe writing.** The save is built and read back in memory first, so one that
  cannot be read never takes the place of one that can. It then replaces the file
  atomically, and the save that was there, if it was good, becomes
  `<slot>.zenithsave.bak`.
* **A damaged save is an error.** `load` and `info` give `SaveDamaged` (the
  message names the slot and says what is wrong) and `SaveNotFound` for an empty
  slot, whether or not a backup exists: the engine never quietly hands out older
  progress. The backup is for you to ask for: `load_backup(slot)` (it sets
  `from_backup`), `restore_backup(slot)` (makes it the save again, keeping the
  backup), or `load_or_backup(slot)` (the save, else the backup, and a line in
  the log) for a game that would rather not bother the player. A damaged save
  is never copied over a good backup.
* **The list.** `slots()` gives the readable saves (newest first, each with
  `has_backup`) and, apart, the `damaged` slots with what is wrong and whether
  their backup can bring them back (`has_backup`), a slot that has only a
  backup included. Offer "restore the previous save" there.
* **Not guessed at.** A save of a newer format is `SaveVersionMismatch`, and
  the older backup is not quietly put in its place. A key problem
  (`ArchiveEncryptedNoKey`, `ArchiveWrongKey`) is not taken for damage either.
* **Slot names** are letters, digits, spaces, `_`, `-` and `.`, up to 64, not
  beginning or ending with a space or a dot (`InvalidSaveSlot` otherwise).

## Sections: values by name

Most saves are numbers and text, not a snapshot. `Saver` and `Loader` keep them
in named sections, with no generics, so a script can call them too:

```rust,ignore
let mut saver = saves.saver("slot1", SaveMeta::new("Level 3", WORLD_VERSION));
saver.begin("player")?;                 // version 1
saver.save_i32("score", 32)?;
saver.save_string("name", &name)?;
saver.end()?;
saver.begin_versioned("inventory", 2)?; // the version of its data
saver.save("items", &items)?;           // anything serde can write
saver.end()?;
saver.entry(WORLD_ENTRY, world.snapshot_bytes()?)?; // a file that is not a section
saver.finish()?;                        // only now anything is written

let loader = saves.loader("slot1")?;
let player = loader.section("player")?;
let score = player.load_i32("score")?;
let lives = player.load_i32_or("lives", 3)?;     // a default for a missing value
let inventory = loader.section("inventory")?;
if inventory.get_version() < 2 { /* move the old data to the new form */ }
```

* **Types.** `bool`, `i32`, `u32`, `i64`, `u64`, `f32`, `f64`, text and bytes,
  each with `save_*` and `load_*` (and `load_*_or`). `save::<T>(key, &value)`,
  `load::<T>(key)`, `save_section(name, version, &value)` and `load_all::<T>()`
  take any `serde` type, for Rust code.
* **A section is a file.** It is the TOML file `Sections/<name>.toml` in the
  container, with its `[section] version` and its `[values]`. Names of sections
  and values are 1 to 64 of ASCII letters, digits, `_` and `-`. Without a key the
  file reads as text; with one, the whole container is encrypted.
* **Limits of TOML.** A `u64` above `i64::MAX` is refused when saving
  (`InvalidSaveData`). Bytes are kept as base64 text. A float also reads an
  integer (`speed = 1` written by hand). The names of sections and of values
  are restricted as above; the fields inside a value saved with `serde` are not
  (a map may have any keys, TOML quotes them). `sections()` lists them in
  alphabetical order.
* **All or nothing.** `begin`, `end` and `save_*` only fill memory. A forgotten
  `end`, a repeated name or a value that does not fit is an error
  (`InvalidSaveData`), and `finish` writes the slot as [`Saves::save`] does
  (atomically, the old one becomes the backup). A save that fails on the way
  leaves the slot as it was.
* **A section is read alone.** `loader` reads the header only; a section is read
  when asked for. A damaged one is `SaveDamaged` with its name, and the others can
  still be read. `Saves::load` (the whole save) still refuses a save with any
  damage. `loader_backup` opens the backup.
* **Reading errors.** `SaveSectionNotFound`, `SaveKeyNotFound` (`player.score`)
  and `SaveWrongType` (also for an integer that does not fit: `i32` from 5 billion).
  `load_*_or` fills in only a missing value, never a wrong one.

In a browser the container is compressed by a pure Rust encoder with one fast
level (the zstd C library does not build there); the level setting is ignored.
Everything it writes is plain zstd, and a build or a save made on a computer
reads in a browser and the other way round.

## What the runtime keeps there

The runtime reads the controls of the author from `Settings://Input.toml`, then
the changes of the player from `Data://Input.toml`, and writes the changes back
(every couple of seconds, and when the game ends) when they differ from what is
there, or removes the file when nothing differs. See [Input](./input.md). The
runtime does not save the world on its own: what the player did belongs in a
save slot, not in the `world.zenith` of the project.
