# Editor themes: what a theme has to set

This is the list of settings a theme file needs so that every part of the
editor (menus, popups, panels, tabs, the graph editor, the gizmos) can be
coloured. The editor draws with Dear ImGui, so most of it is ImGui's style;
the rest are colours that the editor and the renderer own.

Today `bins/zenith_editor/src/theme.rs` sets about 30 of these in code. The
goal is to move all of them into a theme file (see "File format").

## 1. Palette (the few colours a theme author really picks)

A theme starts from a small palette; everything else is derived from it
unless the theme overrides it.

| Name | Used for |
|---|---|
| `bg_primary` | panels, windows, child areas |
| `bg_secondary` | popups, menus, input fields |
| `bg_tertiary` | hovered fields, grips, scrollbar grab |
| `text` / `text_disabled` | normal and greyed text |
| `accent_darkest`, `accent_muted`, `accent_muted_hover`, `accent`, `accent_bright` | buttons, selection, tabs, check marks, sliders |
| `error`, `warning`, `info`, `success` | log lines, graph diagnostics, status colours |

## 2. ImGui colours (every `StyleColor`)

Each one is one `[r, g, b, a]` value. Grouped by what the user sees.

- **Text:** `Text`, `TextDisabled`, `TextSelectedBg`, `TextLink`.
- **Windows:** `WindowBg`, `ChildBg`, `Border`, `BorderShadow`.
- **Menus and popups:** `PopupBg`, `MenuBarBg`, and the hover and selection of
  menu items (`Header`, `HeaderHovered`, `HeaderActive`).
- **Modal dialogs:** `ModalWindowDimBg` (the dark veil behind a dialog).
  Without it a dialog can not be made lighter or darker.
- **Title bars:** `TitleBg`, `TitleBgActive`, `TitleBgCollapsed`.
- **Fields:** `FrameBg`, `FrameBgHovered`, `FrameBgActive`.
- **Buttons:** `Button`, `ButtonHovered`, `ButtonActive`.
- **Check marks, radio buttons, sliders:** `CheckMark`, `SliderGrab`,
  `SliderGrabActive`.
- **Lists and trees:** `Header*` (selected row), `TableHeaderBg`,
  `TableBorderStrong`, `TableBorderLight`, `TableRowBg`, `TableRowBgAlt`.
- **Tabs:** `Tab`, `TabHovered`, `TabSelected`, `TabSelectedOverline`,
  `TabDimmed`, `TabDimmedSelected`, `TabDimmedSelectedOverline`.
- **Docking:** `DockingPreview` (where a panel will land), `DockingEmptyBg`.
- **Separators and grips:** `Separator*`, `ResizeGrip*`.
- **Scrollbars:** `ScrollbarBg`, `ScrollbarGrab`, `ScrollbarGrabHovered`,
  `ScrollbarGrabActive`.
- **Plots:** `PlotLines`, `PlotLinesHovered`, `PlotHistogram`,
  `PlotHistogramHovered` (for the profiler and graphs later).
- **Keyboard and mouse feedback:** `NavCursor`, `NavWindowingHighlight`,
  `NavWindowingDimBg`, `DragDropTarget`.

## 3. ImGui style values (shape, not colour)

- Rounding: `window_rounding`, `child_rounding`, `frame_rounding`,
  `popup_rounding`, `scrollbar_rounding`, `grab_rounding`, `tab_rounding`.
- Sizes: `window_padding`, `frame_padding`, `item_spacing`,
  `item_inner_spacing`, `indent_spacing`, `scrollbar_size`, `grab_min_size`.
- Borders: `window_border_size`, `child_border_size`, `popup_border_size`,
  `frame_border_size`, `tab_border_size`.
- Alignment: `window_title_align`, `button_text_align`,
  `selectable_text_align`.
- Font: file and size (`Inter-Regular.ttf`, 15 px now), and a scale for HiDPI.

## 4. Colours that ImGui does not know about

These are drawn by the editor itself, so a theme has to name them too.

- **Graph editor** (`graph_edit.rs`): one colour per slot kind
  (`kind_color`: Color, Depth, Buffer, Scene data, ...), one per pass
  category (`category_color`: Input, Geometry, Lighting, Post, Output,
  Debug), plus the imnodes colours: `NodeBackground`, `NodeBackgroundHovered`,
  `NodeBackgroundSelected`, `NodeOutline`, `TitleBar*`, `Link`, `LinkHovered`,
  `LinkSelected`, `Pin`, `PinHovered`, `BoxSelector`, `BoxSelectorOutline`,
  `GridBackground`, `GridLine`, `GridLinePrimary`, `MiniMap*`.
- **Gizmos** (the transform gizmo of ImGuizmo): the axis colours X, Y, Z, the
  plane colours, the selection colour, the hatched-axis colour, the text
  colour and the line thickness.
- **Viewport overlays** (the renderer, `passes/editor.rs`): grid minor line,
  grid major line, X axis line (red), Z axis line (blue), the outline of the
  selected entity, the wire shapes of lights and cameras, the viewport clear
  colour behind the world.
- **Status and log:** the colour of an error, a warning, an info line and a
  success line; the colour of a modified (`*`) item.

## 5. Things a theme should NOT decide

Keep these out of the theme so that a theme can not break the editor:
widget sizes that change the layout of the dock, the keyboard shortcuts, and
the language of the text.

## 6. Per-panel overrides

A global theme is not enough: the Viewport wants a darker frame than the
Hierarchy, the Statusbar a different colour from the menu bar, a dialog its
own title colour. So **every panel (and every kind of popup) can override any
colour or style value of section 2 and 3 for itself only.**

### How it works

Before a panel builds its window, the editor pushes that panel's overrides
onto the ImGui style stack (`push_style_color`, `push_style_var`) and pops
them when the window ends. The widgets inside inherit them, so a panel
needs no extra code per widget:

```text
resolve(panel, key) = panel override
                   -> panel group override   (e.g. "all dialogs")
                   -> theme global value     (section 2 and 3)
                   -> base theme -> palette
```

One helper does it for all panels, so no panel pushes colours by hand:

```text
themed(ui, theme, PanelId::Viewport, || { ui.window(..).build(..) })
```

Popups that open from a panel (a context menu, a combo list) are separate
ImGui windows and do not see the style of the panel that opened them. They
are themed by their own entries (`menu`, `popup`, `combo`) or by the panel
group, not by the panel.

### Panels and groups that can be themed

| Id | What it is |
|---|---|
| `menu_bar` | the main menu bar (top) |
| `menu` | the drop-down menus and their items |
| `toolbar` | the row of buttons over the Viewport (Move, Rotate, Scale) |
| `statusbar` | the bar at the bottom |
| `dockspace` | the background behind the docked panels, tabs, splitters |
| `viewport` | the Viewport panel (the frame around the picture, its toolbar) |
| `hierarchy` | the entity tree |
| `details` | the component editor of the selected entity (was "Inspector") |
| `worlds` | the list of worlds |
| `file_explorer` | the file manager (favourites, folders, files) |
| `input` | the input debug panel |
| `render_graph` | the graph editor: the panel and its nodes (see below) |
| `dialogs` | the group of all modal dialogs; `dialog.<name>` for one of them |
| `tooltip` | tooltips |
| `console` / `log` | the log panel, when it exists |

Groups work like panels: `panels` (all docked panels), `dialogs`, `menus`.
A panel override wins over its group.

### What a panel override can hold

- Any `StyleColor` of section 2 (for example `WindowBg`, `TitleBgActive`,
  `Header*`, `Tab*`, `FrameBg*`, `Text`).
- Any style value of section 3 (rounding, padding, spacing, border size).
- A font (file and size) for that panel only.
- The panel's own colours of section 4 (for `render_graph`: the node, pin and
  link colours; for `viewport`: the overlay colours).
- A `title_bar` switch (shown or hidden) is NOT part of a theme: it is layout.

### Example

```toml
[panels.viewport]
WindowBg = "#101010"
frame_padding = [4.0, 2.0]

[panels.hierarchy]
Header = "#2c3a52"          # selected row
HeaderHovered = "#3d4f71"

[panels.statusbar]
WindowBg = "#1a1a1a"
Text = "#8a8a8a"

[dialogs]                   # all dialogs
TitleBgActive = "#3d4f71"
ModalWindowDimBg = "#000000aa"

[dialogs.welcome]           # one dialog
WindowBg = "#202833"

[panels.render_graph]
GridBackground = "#14141c"
[panels.render_graph.slot_kinds]
Color = "#59a6f2"
```

### Rules

- An override only changes what it names; everything else is inherited.
- Overrides are looked up once when a theme loads and stored per panel id, so
  drawing a frame does not search strings.
- A panel that is not listed uses the global theme. A new panel gets an id
  in the table above and is themable at once, without a theme change.
- Switching the theme at run time is allowed: the editor rebuilds the table
  and the next frame uses it.

## File format

A theme is one `.toml` file in a package (`Themes/` folder) with a name, an
author and a `base` theme to inherit from. Only the values that differ from
the base have to be written:

```toml
name = "Midnight"
base = "Zenith Dark"

[palette]
bg_primary = "#1f1f1f"
accent = "#7989a6"

[imgui.colors]
ModalWindowDimBg = "#00000066"
TabSelected = "#3d4f71"

[imgui.style]
window_rounding = 8.0
frame_padding = [6.0, 4.0]

[graph.slot_kinds]
Color = "#59a6f2"

[gizmo]
axis_x = "#e04a4a"
```

Colours are `#rrggbb` or `#rrggbbaa`. A missing value falls back to the base
theme, then to the palette. A theme with an unknown key still loads, and the
editor logs a warning for that key.

## What is missing in the code today

- `MenuBarBg`, `ModalWindowDimBg`, `TableRowBg*`, `NavCursor`, `DragDropTarget`,
  `Plot*` are not set; the ImGui dark preset shows through.
- The graph editor and the gizmo colours are constants in the code.
- The viewport overlay colours are constants in the renderer; the editor
  would pass them in as a resource of the world.
- There is no loader yet: `zenith_theme()` builds the one theme in code.
