# The editor

`zenith_editor` is a window of its own (winit, wgpu and Dear `ImGui`; the loop is
in `app.rs`, not a framework's). It works on a folder project.

## The project and its worlds

The editor opens the project it had open last time (the path is kept in
`Editor.toml` in the `ZenithEngine` folder of the data folder of the user,
`dirs::data_local_dir()`). It reads every `Worlds/**/*.zenith` into memory. A
world is named by its path below `Worlds/` without the extension (`Level1`,
`Caves/Deep`), and each has a `dirty` flag: it has changes that are not in its
file. A world that cannot be read is told to the user in a dialog, and the
others load.

The menu of projects (New, Open, recent ones) is not made yet: the editor has
`Editor::open_project(path)` and the memory of the last project, nothing more.

## Saving: Ctrl+S

**Ctrl+S** (or File → Save All) writes **every world that has changes**, and only
those. Each file is written atomically (a file is the old one or the new one),
but the files are not atomic between themselves: one that cannot be written does
not stop the others, it stays dirty, and a dialog lists it.

The editor does not change worlds yet, so Debug → Mark all worlds changed makes
them dirty to try the saving.

## Closing

Closing the window, File → Quit and Escape (when the Viewport has no focus and
no dialog is open) ask what to do if some world has changes: *Save all and quit*,
*Quit without saving* or *Cancel*. Nothing to lose, no question.

## The backup: Ctrl+C

When the editor is stopped from outside (Ctrl+C, SIGTERM, closing the console)
it does not ask: the worlds with changes are written to `.zenitheditor-backup/`
in the folder of the project, in the layout of the project
(`.zenitheditor-backup/Worlds/Caves/Deep.zenith`), with `Backup.toml` that lists
them and says when. The files of the project are not touched. `Backup.toml` is
written last, so a backup that has it is complete; a backup already there keeps
the worlds the new one does not replace.

The next time the project is opened the editor offers the backup:

* **Restore** writes the worlds into the project (a world that was only in memory
  gets its file) and deletes the backup. One that cannot be restored stays in
  the backup and is told to the user.
* **Discard** deletes the backup.
* **Later** keeps it for the next time.

`.zenitheditor-backup/` is in the `.gitignore` of a new project, and the watcher
never reports it. Names in `Backup.toml` are checked like the names of worlds, so
it cannot point outside its folder.

## File → New World

A popup with the **name** (letters, digits, spaces, `_`, `-`, up to 64) and an
optional **folder** below `Worlds/` (`Caves` or `Caves/Deep`). The world is made
empty, in memory and dirty: its file appears on Ctrl+S (or in the backup). A
name that is bad, or taken by a world in memory or a file on disk, is told in
the popup.

## Panels

* **Viewport.** Where the render will go. It is the only panel that gets the
  keyboard, the mouse and the real gamepads, and only while it has the focus;
  losing the focus lets go of every key.
* **Input.** What the input sees: the keys, the mouse movement, the gamepads. An
  *emulated gamepad* made of widgets (hold a button to press it; sliders for the
  sticks and triggers) can be plugged in and works whether or not the Viewport
  has the focus.
* **Worlds.** The worlds of the project; `*` marks the ones with changes.

The arrangement of the panels is kept in `EditorLayout.ini` next to
`Editor.toml`.

## Ending

Every way out (the window, Escape, Ctrl+C) ends in the same place: the event
loop calls `exiting`, which lets go of the UI and the GPU in order while the
window still exists.
