# Renderer plan: `crates/zenith-renderer`

Status: a plan, no code yet. Date: 2026-11-10.

The old `crates/zenith_renderer` is only a reference for wgpu calls (device
setup, surface format, HDR, shaders). Its structure is not carried over.

Marks in the text: **[decided]** was chosen by Gleb, **[proposed]** is a
proposal that can still be argued with, **[verify]** has to be confirmed
against the documentation before it is built.

## 1. Goals

- Native and Web (WebGPU only, no WebGL2).
- Several cameras, each of which may have its own graph.
- Rendering into a texture for the editor viewport (ImGui shows the texture).
- 2D and 3D in one graph.
- One shared cache of GPU resources.
- The render graph is data: JSON, edited in the editor with imnodes.
- Assets are referenced by UUID.
- The shaders are files of the project, the engine's own and the game's
  (section 19); later, render passes written in scripts (section 20).

Not in the first version: MSAA, frustum culling, asynchronous loading, shadow
cascades, clustered lighting, cooked assets in a build.

## 2. Who writes what

Claude writes the renderer, stage by stage (section 16); Gleb works on
scripting. (At first Gleb was to write the core and Claude the passes; that
changed so the engine moves faster.) The pass API of section 9 and the
contract of section 10 still mark the line inside the crate: passes only use
what the contract gives.

## 3. Decisions

| Question | Decision | |
| --- | --- | --- |
| Where extract copies data | Resources in the main `World`, plain data only | decided |
| Several cameras | `Camera.graph: Option<GraphId>`, `None` is the main graph | decided |
| Pass scope | No `Frame` scope: everything runs per camera, shadows too | decided |
| Cache | One `GpuCache` for all graphs and cameras | decided |
| Graph | Data-driven from the start, JSON, a registry of pass types | decided |
| Assets | `AssetId(Uuid)`, one file per asset, no `.meta` files | decided |
| Where the asset registry lives | The `assets` module of this crate, with no wgpu in it | decided |
| Crate names | `-` instead of `_`; this package is `zenith-renderer` | decided |
| What a pass uses | Everything comes through slots: input nodes give the camera, lights, environment, meshes and sprites | decided |
| Links in the graph | Explicit slot to slot links (like imnodes pins) | proposed |
| Transforms | No node of their own: they are the instance buffer of meshes and sprites | proposed |
| Where input nodes get data | `&World`, read only; the other passes read their slots | proposed |
| Render schedule | The host's, not the world's: the runtime (the editor) keeps it and runs it on the world | decided |
| Settings | `RenderSettings`, a resource apart from the graph, plain data that can be saved | decided |
| Precision | Positions stay f64 until prepare; the GPU gets camera-relative f32 | proposed |
| Windows | No winit in the crate; the host gives sizes and the surface target | proposed |
| Cache eviction | By time unused, not by reference count | proposed |
| WebGL2 | Not supported; drop the `webgl` feature of wgpu | proposed |
| Shaders | Files of the project (`Shaders://`), loaded from there; the engine writes its own into it | decided |
| Shader language | WESL, compiled to WGSL by the `wesl` crate | decided |
| Custom shaders | A material names its shader; the shader follows the contract of section 19 | decided |
| Scriptable render passes | Later, after the graph editor, through the contract of section 10 | decided |

## 4. The whole frame

```
host (runtime or editor)
 ├─ world.update_systems(dt)          the game schedule (may stand still in the editor)
 └─ render.run(world.world_mut())     the render schedule, kept by the host:
      1. Assets    apply file watcher events (editor)
      2. Extract   extract_cameras, extract_models, extract_sprites, extract_lights
      3. Prepare   prepare_assets, prepare_views, prepare_draw_lists
      4. Render    render: run the graph of every camera, submit, present
      5. Cleanup   evict from the cache, return transient textures to the pool
```

The render has a schedule of its own because the editor draws the world while
the game systems stand still (edit mode). It belongs to the host, not to the
world: `zenith_renderer::schedule()` makes it (label `Render`, the systems in
`RenderSet`s), `add_systems` adds them to a schedule the host already has. The
host decides when and how often it runs, and can put systems of its own in it.

### What `zenith_world` has to gain

- `World::keep_resource::<T>()`: the resource outlives `clear_world()`.
  `Input` and `Time` are kept this way; the renderer keeps `Renderer`,
  `RenderSettings`, `ClearColor`, and later `GpuCache`, `AssetRegistry` and
  `RenderTargets`.
- `clear_world()` empties the world in place (every entity, every resource
  that is not kept or built into bevy) instead of making a new `BevyWorld`.
  The systems of a schedule belong to the world they first ran on, and
  running them on another one panics. (Done in stage 1; before it, a level
  restored with `restore_snapshot` after the game schedule had run would
  have panicked the same way.)
- The editor has several worlds open, but one cache and one set of graphs.
  Only the world being shown holds the render resources:
  `zenith_renderer::detach(world)` returns them as a bundle,
  `attach(world, state)` puts them into another world. Two worlds in two
  viewports at once are not in the first version.

## 5. ECS resources

| Resource | Holds |
| --- | --- |
| `Renderer` | `device`, `queue`, adapter info, `surface: Option<SurfaceState>`, the output format, the settings it works by, `graphs: GraphRegistry`, `passes: PassRegistry`, the frame number |
| `RenderSettings` | how the GPU is opened (power, API, fallback adapter, memory, validation: at the next start) and how frames are shown (present mode, alpha mode, frame latency: at once). Saved later in `Settings://Render.toml` with the player's changes on top, like the input. The main graph goes there too |
| `GpuCache` | pipelines, shaders, layouts, samplers, GPU meshes, textures, materials, fallback resources |
| `AssetRegistry` | UUID to path, path to UUID, the list of problems (duplicates) |
| `RenderTargets` | offscreen textures by `RenderTargetId` (the editor viewport) |
| `ExtractedViews`, `ExtractedMeshes`, `ExtractedSprites`, `ExtractedLights` | the data of the frame, copied from components |
| `ExtraViews` | views the host added without an entity (the editor camera) |
| `ViewDrawLists` | per view: sorted batches and ranges of the instance buffer |
| `PreparedViews` | per view: the camera buffer, the lights buffer, the environment |
| `ProjectRes` | `Arc<Project>`, to read asset files |

`GpuCache` is apart from `Renderer` so that a pass can hold `&Renderer` and
`&mut GpuCache` at once.

### `Renderer`

- `Renderer::new(target, size, options).await` makes the instance, adapter,
  device, queue and surface. `target` is
  `impl Into<wgpu::SurfaceTarget<'static>>`, so winit is not a dependency.
- `Renderer::from_device(device, queue, output_format)` has no surface: for
  the editor (`Gfx` makes the device, ImGui shares it) and for tests.
- Setup is asynchronous only. On the web the host inserts the resource when it
  is ready; the render systems run under `resource_exists::<Renderer>`.
- Surface: the host calls `resize_surface(w, h)` on the window event. A zero
  size marks the surface as not configured. The handling of
  `get_current_texture()` (`Timeout`, `Occluded`, `Outdated`, `Lost`) is taken
  from the old `viewport.rs`.
- `CameraTarget::Window` means "the main output". In the runtime that is the
  surface; in the editor it is redirected into the texture of the Game panel
  (`renderer.redirect_window(Some(target_id))`).

## 6. Components

They live in `zenith-renderer/src/components.rs`, are serialized like the
others (`PascalCase`) and are registered with
`zenith_renderer::register_components(&mut ComponentRegistry)`.

```rust
pub struct Model  { pub mesh: AssetId, pub material: AssetId }
pub struct Sprite { pub texture: AssetId, pub size: Vec2, pub color: [f32; 4],
                    pub uv_rect: Option<[f32; 4]>, pub order: i32 }
pub struct Camera {
    pub projection: Projection,       // Perspective { fov_y, near, far } | Orthographic { height, near, far }
    pub target: CameraTarget,         // Window | Texture { id: AssetId, size: [u32; 2] }
    pub viewport: Option<[f32; 4]>,   // a part of the target, in 0..1; None is all of it
    pub order: i32,                   // the order of cameras on one target
    pub clear: ClearMode,             // Color(..) | None (draw over the camera before)
    pub graph: Option<GraphId>,       // None is the main graph
}
pub struct DirectionalLight { pub color: [f32; 3], pub illuminance: f32, pub shadows: bool }
pub struct PointLight       { pub color: [f32; 3], pub intensity: f32, pub range: f32 }
pub struct Environment      { pub sky: AssetId, pub intensity: f32 }   // on the camera's entity
```

A limit of the first version: a `Model` has one material for the whole mesh.
Several materials for sub-meshes come later (`materials: Vec<AssetId>`).

A 2D camera is a camera with `Orthographic`. There is no type of its own.

## 7. Extract and prepare

### Extract

Every system clears its `Vec` (the capacity stays) and copies from its query,
skipping entities that are not active (`IsActive`).

```rust
pub struct ExtractedView {
    pub source: ViewSource,          // Entity(..) | Host(u32)
    pub position: DVec3, pub rotation: Quat,
    pub projection: Projection,
    pub target: ViewTarget,          // Window | Target(RenderTargetId)
    pub viewport: Option<[f32; 4]>, pub order: i32, pub clear: ClearMode,
    pub graph: Option<GraphId>,
    pub environment: Option<(AssetId, f32)>,   // the sky and its intensity
}
pub struct ExtractedMesh   { pub entity: Entity, pub mesh: AssetId, pub material: AssetId,
                             pub position: DVec3, pub rotation: Quat, pub scale: Vec3 }
pub struct ExtractedSprite { /* texture, position, rotation, size, color, uv_rect, order */ }
```

The rule: `Extracted*` hold no wgpu types and no references into the world.
Later these resources can then be taken with `mem::take` and handed to a
render thread with nothing rewritten. The editor camera is not an entity: the
editor puts a view into `ExtraViews`, and `extract_cameras` appends it to
`ExtractedViews`.

### Prepare

- `prepare_assets`: for every `AssetId` that extract found it looks into
  `GpuCache`, loads what is missing (section 11) and marks the entry as used
  this frame.
- `prepare_views`: fills `PreparedViews`: the view and projection matrices and
  the uniform buffer of every view, its lights, its environment.
- `prepare_draw_lists`: sorts and batches per view. Opaque by (pipeline,
  material, mesh) into instances, transparent from far to near, sprites by
  (order, texture).

`Transform.position` is a `DVec3`. The GPU gets `(position - view.position)`
as f32, and the view matrix has no translation. So the instance data is per
view.

A trap: `queue.write_buffer` takes effect before any command of the frame
runs. One uniform buffer for all views gives every view the data of the last
one. Each view gets its own buffer or its own range, never a shared one.

## 8. The render graph

### Description and execution are apart

- `GraphDesc`: plain data (nodes, links, parameters, positions in the editor).
  It is what JSON holds and what the editor edits: lists of nodes and links,
  with node ids that are never reused. (No `petgraph`: a topological sort
  with ties broken by id, and a depth-first search that names the nodes of a
  cycle, are some forty lines; `toposort` of petgraph does not promise the
  order of ties.) What the engine does not know
  (a field, a node of a pass that is not registered) is kept and written
  back.
- `CompiledGraph`: the pass instances, the order, the table of resources, the
  pool of transient textures. It comes from
  `compile(&GraphDesc, &PassRegistry)`.

An edit in the editor changes the `GraphDesc` and compiles it. If that fails,
the last `CompiledGraph` goes on working and the errors are shown on the
nodes.

### Slots: textures and data

A slot carries one of two things.

- A texture (`Color`, `Depth`): a GPU resource that the graph creates,
  versions and pools.
- Data (`Camera`, `Lights`, `Environment`, `Meshes`, `Sprites`): a reference
  to what prepare made for the view. The graph only hands it on: no versions,
  no lifetimes, no pool.

Everything a pass uses comes through its slots. Data comes into a graph
through input nodes. They are in the `PassRegistry` like any pass (category
`Input`), have outputs only and record no commands: in `prepare` they read
the resources of the world and publish a reference on their output. Input
nodes are the only nodes of the engine that read the world; `&World` stays in
the contexts for them, and for a pass of a game that needs what no node gives.

| `type_name` | Output | Carries | Parameters |
| --- | --- | --- | --- |
| `zenith.input_camera` | `camera` | the uniform buffer of the view (view and projection matrices and their inverses, position, the size of the viewport, near and far), and the same as a CPU struct | none |
| `zenith.input_lights` | `lights` | the buffer of the lights of the view (relative to the camera), their number, the ambient light; the list on the CPU, to fit shadows | none |
| `zenith.input_environment` | `environment` | the sky texture with its sampler, the intensity | none |
| `zenith.input_meshes` | `meshes` | batches (material, mesh, a range of instances) and the instance buffer | `phase` (`opaque`, `transparent`) |
| `zenith.input_sprites` | `sprites` | batches by texture and the instance buffer | none |

`input_camera` is the camera the graph is running for. The executor still
knows the view (its target, its viewport), but a pass takes the matrices from
the slot.

**Transforms** have no node of their own: they are the instance buffer of
`Meshes` and `Sprites` (per instance: the model matrix relative to the camera
and the normal matrix). A transform means nothing without the thing it
places, and a node of its own would give every geometry pass two inputs that
only work together. What becomes per instance later (the transform of the
frame before, an entity id, the joints of a skinned mesh) is added to the same
buffer or goes with the batch.

**A note on a texture.** A texture can carry a note from the pass that wrote
it: `ctx.set_note("shadow_map", ShadowInfo { .. })`, read with
`ctx.note::<ShadowInfo>("shadow_map")`. The shadow map carries the matrices
of its light this way, so one link is enough and the map and its matrices
cannot be mismatched.

**Optional inputs.** An optional input that is not linked gets a neutral
default (no lights, a black environment, a shadow map that shadows nothing),
so one shader serves every case.

One list can feed several passes (the opaque pass and the shadow pass), and a
filter added later (a layer mask) is a parameter of the input node, not a
change in every pass. A pass that is given an empty list returns `Skip`.

Inputs that come later, each with the first pass that needs it:

| Node | Gives | For |
| --- | --- | --- |
| `zenith.input_time` | seconds since the start, the delta, the frame number | animated materials, dithering, effects over time |
| `zenith.input_texture` | a texture asset as `Color` (needs an `Asset` type of parameter) | LUTs, masks, noise |
| `zenith.input_history` | the color and depth of the frame before, per view | TAA, reflections in screen space, motion blur |
| `zenith.input_lines` | debug lines and shapes | colliders, gizmos, debug drawing |
| `zenith.input_particles` | batches of particles | effects |
| `zenith.input_ui` | the draw data of the game's UI | UI over the frame |
| `editor.input_selection` | the selected entities | the outline pass, picking |

### Compilation

1. Make the passes by `type_name` through the `PassRegistry`. An unknown type:
   the node is skipped with a warning and its links are dropped.
2. Check the links: the slots exist, the kinds are the same on both ends, the
   required inputs are connected. A pass without a required input is dropped,
   and so on down the chain.
3. Resource versions, for texture slots. An output either creates a texture
   or passes an input on (pass-through: the same resource, a new version).
   When passes A read a version and pass B writes it, an implicit edge A to B
   is added.
4. Topological sort, ties broken by node id so the order is deterministic. A
   cycle is an error that names its nodes (`#1 → #2 → #1`, and
   `Diagnostic::related` lists them for the editor to mark), not what only
   comes after it. Two passes drawing into the same version of a texture
   are an error too: neither comes after the other.
5. Drop the passes the `Output` node does not depend on.
6. Lifetimes: resources with the same description and lifetimes that do not
   overlap share one physical texture.

If nothing reaches `Output` after steps 1, 2 and 5, the graph is broken.

### The fallback graph

- `builtin_graph.json` is built into the crate (`include_str!`) and compiled
  at start under the fixed `GraphId::BUILTIN`.
- The main graph of a project is named in `Settings://Render.toml`
  (`main_graph = "<uuid>"`). Without the setting, the main graph is the
  built-in one.
- A graph file that is missing, cannot be parsed or does not compile: a
  warning in the log, and the built-in graph takes its place.
- `Camera.graph = Some(id)` and no such graph: the main graph is used.
- The editor keeps unknown nodes as ghosts with their JSON as it was, and
  writes them back on save, so a graph with a pass of a plugin that is not
  loaded loses nothing.

### The file (`.zenithgraph`, JSON)

```json
{
  "uuid": "8f0c…",
  "type": "render_graph",
  "version": 1,
  "name": "Main",
  "nodes": [
    { "id": 1, "pass": "zenith.input_camera", "editor": { "pos": [-240, 0] } },
    { "id": 2, "pass": "zenith.input_meshes", "params": { "phase": "opaque" }, "editor": { "pos": [-240, 160] } },
    { "id": 3, "pass": "zenith.opaque",  "params": { "clear_color": [0.1, 0.2, 0.5, 1.0] }, "editor": { "pos": [120, 80] } },
    { "id": 4, "pass": "zenith.tonemap", "params": { "operator": "aces", "exposure": 1.0 }, "editor": { "pos": [520, 80] } },
    { "id": 5, "pass": "zenith.output",  "editor": { "pos": [860, 80] } }
  ],
  "links": [
    { "from": [1, "camera"], "to": [3, "camera"] },
    { "from": [2, "meshes"], "to": [3, "meshes"] },
    { "from": [3, "color"], "to": [4, "hdr"] },
    { "from": [4, "color"], "to": [5, "color"] }
  ]
}
```

A node `id` is stable and never reused. The `type_name` of a pass is the key
in the file, so it must never be renamed.

### Transient resources

- A pool of transient textures, `HashMap<TextureKey, Vec<PooledTexture>>`,
  the key being (format, size, samples, usage). One pool for all the graphs
  (the renderer owns it, stage 5): a view takes what it needs and gives it
  back when it is done, so the next view takes the same textures.
- The size is a rule: `View { scale }` (a share of the size of the view),
  `Absolute(w, h)` or `FromParam("name")`.
- The format: `Fixed(..)`, `Hdr` (`Rgba16Float`), `Depth` (`Depth32Float`) or
  `ViewTarget`.
- A resource of the `ViewTarget` format is the target of the camera itself;
  there is no copy. `Output` must be given such a resource, anything else is
  a compile error.
- Cameras of one graph with different sizes get different textures from the
  same pool. A texture not taken for N frames is freed.
- The graph decides the `LoadOp`, not the pass: the first to write a resource
  gets `Clear`, the rest `Load`. For the target of a camera: `Clear` if it is
  the first camera on that target this frame and `clear = Color`, else `Load`.
- The `viewport` of a camera is set automatically (viewport and scissor) when
  a pass draws into the target of the camera.
- A pass that returns `Skip` does not leave what it creates as garbage: the
  graph clears it (color to transparent black, depth to 1; the target of the
  view only if this camera clears it).

### Execution

```
render (an exclusive system, &mut World):
  frame += 1; take the surface texture, if there is a surface
  encoder = device.create_command_encoder()
  sort the views by (target, order)
  for every view:
      graph = graphs.resolve(view.graph)       // its own → main → built-in
      hand out transient textures for the size of the view
      prepare of every pass, in order          // may return Skip
      exec of every pass that was not skipped  // in a debug group named after the node
  queue.submit; present
```

One encoder per frame. An error of `exec` is logged once, the pass is skipped
and the frame goes on.

### The built-in graph

```
textures:
  shadow ──► opaque ──► sky ──► transparent ──► sprites ──► tonemap ──► output
  (opaque creates color and depth; they are passed through from then on)

data:
  input_camera                ──► shadow, opaque, sky, transparent, sprites
  input_lights                ──► shadow, opaque, transparent
  input_environment           ──► opaque, sky, transparent
  input_meshes (opaque)       ──► shadow, opaque
  input_meshes (transparent)  ──► transparent
  input_sprites               ──► sprites
```

2D and 3D in one graph: sprites are drawn into the same HDR color with the
same depth. With a 2D camera the 3D passes have nothing to draw and return
`Skip`. A camera that needs no shadows or post-processing is given a graph of
its own without those nodes.

## 9. `RenderPass`

```rust
pub trait RenderPass: Send + Sync + 'static {
    /// What the type is. Kept in the PassRegistry: the editor needs no instance.
    fn describe() -> PassDesc where Self: Sized;

    /// Parameters from JSON: on creation and on every edit in the editor.
    fn set_params(&mut self, params: &ParamMap) -> ZResult<()>;

    /// Once after creation and after the graph is compiled again: layouts, shaders.
    fn init(&mut self, ctx: &mut InitContext<'_>) -> ZResult<()>;

    /// Every frame for every view, before exec: ask for pipelines, update own buffers.
    fn prepare(&mut self, ctx: &mut PrepareContext<'_>) -> PassAction; // Run | Skip

    /// Records commands and nothing else. No resource is created here.
    fn exec(&self, ctx: &mut ExecContext<'_>) -> ZResult<()>;
}
```

```rust
pub struct PassDesc {
    pub type_name: &'static str,       // "zenith.opaque"
    pub display_name: &'static str,
    pub category: PassCategory,        // Input | Geometry | Lighting | Post | Output | Debug: the color of the node header
    pub inputs: Vec<InputDesc>,        // name, kind (SlotKind: a texture or a data kind), access (Read | Write), optional
    pub outputs: Vec<OutputDesc>,      // name + Create(TextureDesc) | PassThrough("input name") | Data(kind)
    pub params: Vec<ParamDesc>,        // name, type, default, min/max, tooltip
    pub tooltip: &'static str,
}
```

- Parameters are of simple types only: `Bool`, `Int`, `Float`, `Vec2/3/4`,
  `Color`, `Enum(variants)`, `String`. The editor builds widgets from
  `ParamDesc`.
- `PassRegistry`: `type_name` to (`PassDesc`, factory). The engine registers
  its passes, the editor adds its own (`editor.grid`, `editor.gizmos`,
  `editor.outline`).
- An input node is the same trait: `prepare` publishes its data with
  `ctx.set_data("slot", ..)`, and `exec` does nothing.
- There is no `resize` in the trait. The graph owns the textures and a pass
  sees the sizes of the frame in `prepare`. The graph caches the bind groups
  of input textures by the id of the physical texture and drops them when
  that texture is made again.
- One pass instance serves several views in a frame. State per view must not
  sit in the fields of the pass; `ctx.view_state::<T>()` is for that.

## 10. The contract between the core and the passes

What a pass is given. The names are a proposal; the shapes are what matters.

| From | A pass uses |
| --- | --- |
| `InitContext` | `device`, `queue`, `&mut GpuCache` |
| `PrepareContext` | the same, `&World`, the format and size of every texture slot, `data::<T>("slot")`, `set_data("slot", ..)`, `set_note("slot", ..)`, `bind_group(layout, entries)`, `view_state::<T>()` |
| `ExecContext` | `encoder`, `&World`, `&GpuCache`, `texture("slot")`, `data::<T>("slot")`, `note::<T>("slot")`, `color_attachment("slot")`, `depth_attachment("slot")`, `begin_render_pass(..)` |
| `GpuCache` | `pipeline(&PipelineKey) -> PipelineId`, `get_pipeline(id)`, `shader(source) -> ShaderId`, `layout(..)`, `sampler(..)`, `mesh(id)`, `texture(id)`, `material(id)` |
| `ViewDrawLists` | per view: `opaque`, `transparent`, `sprites`, each a list of `Batch { pipeline_key_part, material, mesh, instances: Range<u32> }`, and the instance buffer. Only the input nodes read it; the other passes get a list from a `Meshes` or `Sprites` slot |
| `PreparedViews` | per view: the camera and lights buffers, the environment. Only the input nodes read it |

Bind groups. WebGPU allows four, so an input cannot be a bind group of its
own: a data kind gives buffers and texture views, and the pass puts them
together.

- Group 0: what the pass was given (the camera buffer, the lights buffer, the
  shadow map, the environment). The pass owns the layout and builds the group
  with `ctx.bind_group(layout, entries)`; the graph caches it by what is in
  it.
- Group 1: the material. Its layout is defined once in the cache and the
  shaders follow it, so it is agreed before the first pass is written.
- Groups 2 and 3: free for the pass.
- Transforms are not a bind group: the instance buffer is a vertex buffer
  stepped per instance.

### The first passes

| `type_name` | Inputs | Outputs | Parameters |
| --- | --- | --- | --- |
| `zenith.input_camera` | none | `camera` (data) | none |
| `zenith.input_lights` | none | `lights` (data) | none |
| `zenith.input_environment` | none | `environment` (data) | none |
| `zenith.input_meshes` | none | `meshes` (data) | `phase` |
| `zenith.input_sprites` | none | `sprites` (data) | none |
| `zenith.shadow` | `camera`, `lights`, `meshes` | `shadow_map` (creates, `Depth`, `FromParam("size")`, with a `ShadowInfo` note) | `size`, `depth_bias` |
| `zenith.opaque` | `camera`, `meshes`, `lights` (optional), `environment` (optional), `shadow_map` (optional, read) | `color` (creates, `Hdr`), `depth` (creates) | `clear_color` |
| `zenith.sky` | `camera`, `environment`, `color` (write), `depth` (read) | `color` (pass-through) | `intensity` |
| `zenith.transparent` | `camera`, `meshes`, `lights` (optional), `environment` (optional), `color` (write), `depth` (read), `shadow_map` (optional) | `color` (pass-through) | none |
| `zenith.sprites` | `camera`, `sprites`, `color` (write), `depth` (write) | `color`, `depth` (pass-through) | `depth_test` |
| `zenith.tonemap` | `hdr` (read) | `color` (creates, `ViewTarget`) | `operator`, `exposure` |
| `zenith.output` | `color` (read) | none | none |

## 11. `GpuCache`

One resource for everything.

| What | Key | Evicted |
| --- | --- | --- |
| Shaders | `ShaderId` | no (dropped on hot reload) |
| Pipelines | `PipelineKey` | no |
| Bind group layouts, samplers | their description | no |
| Meshes | `AssetId` | yes |
| Textures | `AssetId` | yes |
| Materials (bind group) | `AssetId` | yes |

`PipelineKey`: shader, vertex layout, color formats, depth format, samples,
blend, cull, depth write and compare, topology. A pass asks for a pipeline in
`prepare`, so one material works in graphs with different formats.

### The state of an asset in the cache

`Loading` (not drawn), `Ready`, `Failed` (drawn with the fallback). A
material is `Ready` even when a texture of it failed: that texture's slot gets
the checker (a normal map, the flat normal).

Loading in the first version is synchronous, in `prepare_assets`, with a time
budget per frame. What did not fit stays `Loading` until the next frame. The
CPU parsing (the `assets` module) is apart from the upload to the GPU, so the
parsing can move to background threads later and the cache stays as it is.

### A lost asset

The `AssetId` is not in the registry, or the file cannot be read or parsed:

- a mesh becomes the built-in cube, a texture a magenta checker, a material
  the standard one;
- one warning in the log per `AssetId`, with the path hint from the scene;
- the `Failed` state is remembered; it is tried again only when the file
  changes or on a manual reload;
- nothing panics.

### Eviction

- An entry has `last_used_frame` and its size in bytes. `prepare_assets` marks
  what the frame needs. A material marks its textures too.
- A system in `Cleanup` removes the entries not used for longer than a
  threshold (30 seconds by default, longer in the editor) while the cache is
  over a soft memory budget.
- By hand: `cache.evict_unused()` (on a change of world) and `cache.pin(id)`
  (preloading).
- A reference count does not fit: an `AssetId` is a `Copy` value in
  components, and there is nothing to count its holders with.

### Invalidation

A file changed (the watcher in the editor, or "replace the source"):
`cache.invalidate(id)`. Through reverse dependencies the materials that use
that texture are dropped too. The next `prepare_assets` loads them again.

## 12. Assets (the `assets` module)

The boundary: the module imports neither wgpu nor anything of the rest of the
crate. It gives out `AssetId`, `AssetRegistry`, the reading of containers and
CPU structures (`MeshData`, `ImageData`, `MaterialData`, `GraphDesc`). Physics
and sound take it as it is later; moving it into a crate of its own is moving
the folder.

### `AssetId`

`AssetId(Uuid)`, `Copy`, a string in serde. Only the editor and the import
make new UUIDs; the runtime on the web does not.

The path in a scene as a hint: on saving a world the editor adds an optional
`AssetHints` section (`uuid` to path) for every UUID in the components that
the registry knows. Loading does not look assets up by it; it is only there
for the message about a lost asset.

### The binary container (models and textures)

One file per asset: `.zenithmodel` for models, and `.zenithtexture` is
proposed for textures. Numbers are little-endian.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 4 | magic: `ZMDL` or `ZTEX` |
| 4 | 4 | format version (u32) |
| 8 | 16 | UUID |
| 24 | 4 | length of the JSON (u32) |
| 28 | N | JSON with the import settings (UTF-8) |
| 28 + N | to the end | the raw bytes of the source |

- The header (28 bytes) is read through `Project::open_read`, without parsing
  the file.
- The UUID sits at a fixed offset: giving a copy a new UUID is writing 16
  bytes.
- The JSON of a model: the format of the source (`obj` or `glb`), scale,
  UV flip, the material of each mesh. The JSON of a texture: the format of
  the source, the use (`color` in sRGB, `normal`, `hdr`, `data`), mips,
  filter, wrap. Tangents are always worked out on import, the same way for
  every format (glTF tangents are not used).
- The source is always one file. On importing an `.obj`, its `.mtl` and
  textures become material and texture assets of their own; on importing a
  glTF, its materials and pictures do (pictures in the `.glb`, in `data:`
  URIs or in files beside the `.gltf`). A `.gltf` is kept as a `.glb`: every
  buffer goes into the binary chunk. The nodes of its scene place its meshes
  (their transforms are applied, mirroring ones turn the triangles back).
- A file that starts with `version https://git-lfs` is an LFS pointer: an
  error that says to run `git lfs pull`.

### Text assets (JSON)

Materials, prefabs, scenes, render graphs: plain JSON with the `uuid` inside.
Common fields: `uuid`, `type`, `version`. The extensions `.zenithmaterial` and
`.zenithgraph` are proposed.

```json
{
  "uuid": "3b1e…", "type": "material", "version": 1,
  "shader": "standard",
  "base_color": [1, 1, 1, 1],
  "base_color_texture": "c41a…",
  "normal_texture": null,
  "roughness": 0.5, "metallic": 0.0,
  "blend": "opaque", "double_sided": false
}
```

### The registry

- `AssetRegistry::scan(&Project)` walks the project, takes the files with
  known extensions and reads only the header of a container, or the `uuid`
  and `type` fields of a JSON file.
- Scanning an archive at start is slow in a build, on the web most of all.
  Later `zenith_build` writes a ready index into the build and the runtime
  reads it with `AssetRegistry::from_index`. Until then the runtime scans.
- In the editor the registry follows the events of `Project::watch`.

### Cases

**Duplicate UUIDs.**
- Copying in the editor goes through `assets::copy_asset`: the copy gets a new
  UUID at once.
- A copy made outside (the file manager, git): the scan finds two files with
  one UUID. The path the registry already knew wins; on a cold scan, the
  lexicographically smaller one. The other file is not registered, a warning
  with both paths goes to the log, and the editor lists it among the problems
  with an action "give it a new UUID". The editor does not rewrite files it
  did not copy.

**Moving a file in the editor.** `Project::move_to` and
`registry.rename(from, to)`. The UUID in the file does not change, so no
reference breaks. The path hints in scenes are stale until the next save,
which harms nothing. A move made outside: a "removed" event and a "created"
event with the same UUID are seen as a move.

**Replacing the source.** The "replace the source" button of a model or a
texture writes a new container with the same UUID and the same import
settings, then `cache.invalidate(id)`. References and materials are not
touched.

**A lost asset.** See section 11.

### Git LFS

A project does not get a `.gitattributes` of its own: the engine writes none.
The `.gitattributes` at the root of the repository puts `.zenithmodel` and
`.zenithtexture` (and the source formats) into LFS; a game in a repository
of its own sets up LFS the way it wants. JSON assets do not go to LFS: they
are text for the sake of diffs and merges.

## 13. The editor

### The viewport

1. `Gfx` makes the device and the renderer gets it through
   `Renderer::from_device`. The surface stays with the editor.
2. The editor makes a target in `RenderTargets` (`RENDER_ATTACHMENT |
   TEXTURE_BINDING`) and registers its view with `dear-imgui-wgpu`.
   **[verify]** the API for an external texture in 0.18.
3. Every frame: the size of the panel in pixels; if it changed,
   `targets.resize(..)` and a new registration. The size is at least 1 and at
   most `max_texture_dimension_2d`.
4. The editor puts the view of its camera into `ExtraViews` with
   `graph: Some(EDITOR_GRAPH)` (the main graph plus grid, gizmos and outline)
   and runs the render schedule.
5. The commands of the world are submitted before those of ImGui, so the
   panel shows the frame of now.

The Game panel: cameras with `CameraTarget::Window` are redirected into its
texture.

### The panels (UE-like layout)

The four panels that matter most, in this order of importance: **Details**,
**Hierarchy**, **File Explorer**, **Viewport**. The default layout is the one
of Unreal Engine, not of Unity: the Viewport in the middle and big, the
Hierarchy (what UE calls the Outliner) on the right at the top, the Details
(the same name as in UE) on the right below it, and the file panel along the whole
bottom edge. The docking layout is saved with the editor settings as it is
now (`EditorLayout.ini`).

- **Hierarchy.** The entities of the shown world as a tree with the name and
  an icon by what the entity has (model, light, camera, sprite). A search
  box, selecting with a click, Ctrl/Shift for several, rename, delete,
  duplicate, drag to make a child. Needs a parent/child component in
  `zenith_components` (**[verify]** it does not exist yet: today the world is
  flat). The selection is the same everywhere: the Viewport, the Hierarchy
  and the Details share it.
- **Details.** For the selected entity: the name and the active flag, then
  one collapsible block per component, with a widget per field, made from
  the registry of components (`zenith_registry`: the names and the types of
  the fields), so a component of a game shows up without code in the
  editor. `Add Component` with a search, remove a component, reset to
  default, a lock, multi-edit when several entities are selected. Every
  change marks the world as changed. Asset fields (a model, a material, a
  texture) take a drag from the File Explorer. Transform is the first block.
- **Viewport.** Besides what stage 11 did: **selection by a click in the
  picture** (a ray against the bounds of the models, then the nearest hit;
  an ID pass can replace it later), box selection by dragging, Ctrl/Shift to
  add, the outline (`editor.outline`) of everything selected, the gizmo on
  the selection, and a hover highlight.
- **File Explorer** (the name is a proposal, see below). Like the Content
  Browser of UE: a tree of folders on the left, the files of the chosen
  folder as tiles or a list on the right, a path bar with breadcrumbs, a
  search and a filter by the kind of asset, a size slider for the tiles.
  **Favorites** at the top of the tree: a folder or an asset is added with
  a right click and stays. **Opened folders are remembered**: which folders
  of the tree are expanded, which folder is open, the scroll, the view mode
  and the favorites are saved per project (in `Settings/`, not in the
  world) and come back when the editor or the project is opened again;
  closing a panel does not forget the open folder. Drag and drop to move
  (through `ops::move_asset`, so ids stay), rename, delete (to the trash of
  the project), import by dropping files in, a thumbnail for pictures and
  models, a context menu with Show on Disk, Replace Source, Export.

Names for the file panel: **Content Browser** (the UE name, the easiest to
recognise), **Asset Browser**, **Project Files**, **File Manager**, **Assets**.
The proposal is **Content Browser**, because the layout and the behaviour
copy UE; **Assets** is the short, neutral alternative.

### The graph editor (imnodes)

- A list to choose which graph is shown (from the `GraphRegistry`).
- Nodes and pins from `PassDesc`, the color of a pin by the kind of the slot,
  the color of a header by the category.
- Parameters: ImGui widgets from `ParamDesc`.
- Adding a node from the `PassRegistry`, making and removing links, compiling
  after every edit, errors on the nodes, saving to JSON.
- Nothing more.

**[verify]** that `dear-imnodes` works with `dear-imgui-rs` 0.18.

## 14. Web (WebGPU)

- Asynchronous setup only; no `pollster` in the crate.
- Bevy resources need `Send + Sync`. **[verify]** that the types of wgpu 30
  have them on wasm without atomics (the
  `fragile-send-sync-non-atomic-wasm` feature). The way out otherwise:
  non-send resources.
- Only what core WebGPU has: `Features::all_webgpu_mask() &
  adapter.features()`, default limits. It follows that there are at most 4
  bind groups, no push constants and no multi-draw indirect, shadows go
  through a comparison sampler, HDR is `Rgba16Float`.
- The surface format: a browser stores non-sRGB and gives an sRGB view
  through `view_formats`. The logic is in the old `pick_surface_format`.
- Shaders are WESL compiled to WGSL (section 19); the GPU only gets WGSL.
- No threads: everything is on one thread, and the time budget of asset
  loading matters here most. Time comes from `web-time`.
- Drop the `webgl` feature of wgpu.

## 15. The layout of the crate

```
crates/zenith-renderer/src/
  lib.rs            the public API, RenderSet, add_systems, attach/detach
  renderer.rs       Renderer: setup, the start and the end of a frame
  surface.rs        surface configuration, format choice, acquire/present
  targets.rs        RenderTargets, RenderTargetId
  components.rs     Camera, Model, Sprite, lights, Environment
  extract.rs        Extracted* and the extract_* systems
  prepare/          assets.rs, views.rs, draw_lists.rs
  render.rs         the render system
  graph/            desc.rs, compile.rs, exec.rs, pool.rs, pass.rs,
                    registry.rs, builtin_graph.json
  passes/           inputs.rs, shadow.rs, opaque.rs, sky.rs, transparent.rs,
                    sprites.rs, tonemap.rs, output.rs
  cache/            mod.rs, pipelines.rs, meshes.rs, textures.rs,
                    materials.rs, fallback.rs, evict.rs
  assets/           id.rs, container.rs, json.rs, registry.rs, load.rs, ops.rs,
                    import/ (mesh.rs, obj.rs, gltf.rs, image.rs)   // no wgpu
  shaders/          *.wesl: the engine's copies, written into a project
                    (section 19)
```

Dependencies: add `glam`, `bytemuck`, `uuid`, `serde_json`, `zenith_error`,
`zenith_log`, `zenith_filesystem` (without `fs`), `image`, `tobj`, `gltf`,
`half` (HDR textures as 16-bit floats), `web-time` (the clock of the cache;
`std` on native). Remove `winit`.

The name: the package is `zenith-renderer` (`zenith_renderer` in code); the
old one is `zenith_renderer_old` until it is removed. The other crates move
from `_` to `-` in a refactor of their own.

## 16. Stages

Every stage ends with something that can be checked.

1. **The frame.** *Done.* Dependencies, `Renderer` (native and web),
   `RenderSettings`, the render schedule kept by the host, `keep_resource` in
   `zenith_world`, `ClearColor` for a window no camera draws on. Check: the
   example `clear` clears a window through a world and its render schedule;
   headless tests; the crate is checked for wasm32 in CI. The runtime runs on
   the new crate (it shows the clear color until stage 6); its old camera
   code is gone.
2. **Assets.** *Done.* `AssetId`, the container, JSON (materials), the
   registry (duplicates, moves, refresh from the watcher), the CPU import
   (`.obj`, `.gltf` and `.glb` with tangents, pictures, HDR as floats),
   `load` by id, the operations (import a model with its materials and
   textures, import a texture, copy, move, new id, replace the source, export
   the source, import a model file put into `Assets/` by hand in place with
   the file going to the trash), `zenith-cli` on `clap` (`import`, `export`,
   `assets`, `new-id`; the project is the one the current folder is in unless
   `-p` gives it), and in the editor File → Import, Export and Import Loose
   Models, with the question on open that `Settings://Editor.toml`
   (`[assets] loose_models = "ask" | "always" | "never"`) turns off or makes
   automatic. Check: tests with
   no GPU, and the cube and
   the sky of the sample are `.zenithmodel` / `.zenithtexture` assets that a
   test reads. Not yet: the `AssetHints` of world files (with the editor), the
   watcher wired to the registry (the editor), the asset index in a build.
3. **`GpuCache`.** *Done.* Meshes (one vertex and one index buffer a
   model, the meshes as ranges), textures (sRGB colors, linear normal maps
   and data, HDR as `Rgba16Float`, mipmaps made on the GPU), materials (the
   layout of group 1 fixed in `cache/materials.rs`), shaders, layouts,
   samplers and pipelines by their key (a shader made again drops its
   pipelines), the stand-ins, loading within a time budget, invalidation
   through the materials that use a texture, eviction by time over a memory
   budget, `pin`, `evict_unused`. The cache is a resource kept like the
   renderer; the render schedule starts its frame and evicts at the end.
   Check: headless tests (the cube and the sky of the sample, a lost and a
   broken asset, invalidation, eviction, the budget, pipelines against the
   documented layouts, mipmaps read back). Not yet: `prepare_assets`, which
   asks for what extract found (stage 5).
4. **The graph core.** *Done.* `PassDesc` with its slots and parameters
   (names are `Cow<'static, str>`, for the script passes of section 20), the
   `RenderPass` trait, `PassRegistry` (types known at compile time and ones
   registered at run time), `GraphDesc` and its JSON with editing (add and
   remove a node, link and unlink), compilation in six steps, the pool,
   execution with the three contexts (data, notes, bind groups cached by
   what is in them, state per view, attachments whose load the graph
   decides, the viewport of the camera), `GraphRegistry` (the last version
   that compiled keeps running; own graph → main → built-in). `init` runs
   before the first frame of a compiled graph. Check: compile tests with no
   GPU (order with implicit edges, left-out nodes, dropped links, cycles, two
   writers, the output, aliasing, parameters), and run tests on a GPU with
   passes made for the test (the clear of the target by the first camera
   only, data and textures from pass to pass, a skipped pass). The contract
   of section 10 exists in code from here on. Not yet: the built-in graph
   compiles only once its passes exist (stage 6).
5. **ECS.** *Done.* The components (`Model`, `Sprite`, `Camera`,
   `DirectionalLight`, `PointLight`, `Environment`) with
   `register_components`; the `Extracted*` resources and `extract_*` systems
   (inactive entities and cameras into a texture are left out, the views of
   the host come from `ExtraViews`, the views are ordered); `RenderTargets`;
   `prepare_assets` (requests what the frame draws, loads within the budget),
   `prepare_views` (matrices relative to the camera, one uniform buffer per
   view), `prepare_draw_lists` (opaque by material and mesh, transparent far
   to near, all instances of the frame in one buffer); the `render` system,
   which runs the graph of every view and clears a target by its first camera.
   `Renderer` holds the graphs, the passes and the texture pool. Not yet:
   the lights buffer and the environment (stage 8), the sprite draw lists
   (stage 9), cameras into render textures (stage 7), and no graph runs until
   the first passes exist (stage 6): a view is only cleared.
6. **The first passes.** *Done.* `zenith.input_camera`, `zenith.input_meshes`
   (parameter `phase`), `zenith.opaque` (HDR color and depth, one fixed light
   in `standard.wgsl` until stage 8), `zenith.tonemap` (aces, reinhard, none;
   exposure) and `zenith.output`, with the built-in graph of the five
   compiled at the start of `Renderer`. The passes ask for their shaders by
   name (`Zenith/standard`, `Zenith/tonemap`) through `ShaderSources`
   (`GpuCache::named_shader`), which gives the engine's copies until stage
   10. A mesh or a material that is still loading is not drawn.
   `PrepareContext::texture` lets a pass make a bind group of a texture of
   the graph in `prepare`. The runtime registers the components and gives the
   world its project (`install_project`). Check: tests draw the sample cube
   into a texture on a real adapter (the background, the cube in the middle,
   a cube behind the camera not seen). A world needs a `Camera` entity to
   be drawn: the runtime draws nothing of a world without one.
7. **Cameras and targets.** *Done.* `order`, `viewport`, `clear`, `graph` and
   `RenderTargets` were made with stages 5 and 6; this stage added the camera
   that draws into a texture: `CameraTarget::Texture { id, size }`,
   `ViewTarget::Texture`, `RenderTargets::ensure_texture` / `texture_target`
   and the `prepare_targets` system (the texture is made on first use and
   remade when the size changes). Check: two cameras with viewports on one
   target, a third into a texture, on a real adapter.
8. **The rest of the 3D passes.** *Done.* `zenith.input_lights` (up to 4
   directional and 16 point lights, relative to the camera, an `ambient`
   parameter), `zenith.input_environment`, `zenith.sky`, `zenith.shadow` (one
   directional light, a box of `distance` around the camera, PCF, the matrices
   go as a `ShadowInfo` note on the map), `zenith.transparent` (blended, far to
   near, no depth write). `standard.wgsl` has Lambert plus Blinn-Phong from
   the roughness, normal maps (the frame from derivatives, so meshes need no
   tangents), the environment as an equirectangular picture (a high mip level
   for the ambient light, a level by roughness for the reflection; a texture
   imported without mipmaps gives a sharp ambient light). Group 0 is shared by
   `opaque` and `transparent` (`passes/scene.rs`), unlinked optional inputs
   get stand-ins. The built-in graph has all of it. Checks on a real adapter:
   a light lights a face, a light from behind does not, a point light, a
   cube's shadow on a wall, the sky where nothing else is. Not checked in a
   picture: the blended pass (the sample project has no blending material).
9. **2D.** *Done.* `zenith.input_sprites` and `zenith.sprites` are in the
   built-in graph, after `transparent`. A sprite is a quad (`sprite.wgsl`) that
   faces +Z of its entity, tinted by `color`, cut out of the texture by
   `uv_rect`. They are sorted by `order`, then far to near, then by texture
   (`prepare_sprite_lists`, a buffer of its own). They blend, do not write
   depth, and are hidden by what is nearer when `depth_test` is on. A 2D
   camera is a camera with `Orthographic`. Checks on a real adapter: where a
   sprite stands and where it does not, moving it, the order, `uv_rect`.
10. **Shaders in the project** *(done, see below)* (section 19): `Shaders://`, the engine's
    shaders written into new projects and into old ones on open, the version
    line and "the engine has a newer one", loading from the project, WESL
    compiled with `wesl` (imports through the project, conditions in the
    pipeline key), checking with `naga`, the stand-ins of a missing or broken
    shader, custom shaders named by materials. The engine's shaders written
    before this stage move to `.wesl` here. Check: a project whose
    `Zenith/standard.wesl` was changed draws with the change; a broken one
    draws with the engine's copy and a warning.
    *Done:* `Shaders://` (`Assets/Shaders`), the engine's shaders are `.wesl`
    with a `// zenith-shader: name version` line and a shared
    `Zenith/common`; `shaders::project` writes the missing ones
    (`install_engine_shaders`, an older copy is only reported,
    `restore_engine_shaders` writes all), and reads the project
    (`load_project_shaders`, `.wesl` wins over `.wgsl`); `shaders::compile`
    runs `wesl` over the project's files in memory and checks the WGSL with
    `naga`; `GpuCache::named_shader_with` takes WESL conditions (they are part
    of the key), keeps the last version that compiled, else stands in with the
    engine's copy, else `Zenith/magenta`. The `load_shaders` system reads the
    project once and when `ReloadShaders` is set. `wesl` builds for wasm32.
    Left out: (`mipmap` is WESL too now, `Zenith/mipmap`, compiled once inside the cache; it is not written into a project because a project would not change it); the runtime does not write shaders into a project
    (the editor does, stage 11); no compiled-WGSL builds yet.
11. **The editor: viewport.** The texture in ImGui, the editor camera, hot
    reload (of shaders too), replacing a source, the editor passes
    (`editor.grid`, `editor.gizmos`, `editor.outline`). Also ImGuizmo
    (the transform gizmo: move, rotate, scale of the selected entity, drawn
    by ImGui over the viewport texture, not a render pass; it needs the view
    and projection matrices of the editor camera, which the editor already
    has).

    *Done:* the renderer has `GraphId::EDITOR` (`editor_graph.json`: the
    built-in graph plus `editor.grid`, `editor.gizmos`, `editor.outline`, all
    drawing lines with `Zenith/lines` over the scene with a depth test). The
    grid is lines on `y = 0` that fade out; the gizmos are the lines of the
    `GizmoLines` resource (the editor puts the marks of lights and cameras
    there); the outline is the box of each model in `EditorSelection` (the
    bounds are kept in `GpuMesh`), not a silhouette. `uninstall` /
    `install_again` move the renderer with its cache from one world to
    another. In `zenith_editor`: `scene_view` draws the shown world into a
    texture through the shared device and registers it with `dear-imgui-wgpu`
    (resized to the panel); `camera` is the editor camera (right mouse button
    and WASD/QE to fly, wheel, `F` to focus); `viewport_panel` has the
    Viewport, the Hierarchy and the list of worlds, and the ImGuizmo gizmo
    (`dear-imguizmo` 0.18, `W`/`E`/`R`, local or world axes) whose result is
    written back into `Transform` and marks the world as changed. The editor
    writes the engine's shaders into a project once, and watches
    `Assets/` with `Project::watch`: a changed shader sets `ReloadShaders`, a
    changed asset is invalidated in the cache, a new or removed one makes the
    registry read again. File → Replace Source uses `ops::replace_source`;
    the watcher does the rest. Left out: picking by a click in the picture,
    the Game panel, a silhouette outline; the window was not run by hand
    (only the parts without a window are tested), and a project with no
    world shows nothing in the Viewport.
12. **The graph editor** with imnodes. *Done:* the panel Render Graph (Window menu): a list of the graphs of the renderer and of the project (`.zenithgraph` assets, loaded on a click), nodes and pins made from `PassDesc` (a pin is colored by what its slot carries, a title by the category of its pass), parameters as ImGui widgets made from `ParamDesc` right in the node, links by dragging (checked: output to input, the same kind of slot, not to itself; Ctrl-drag detaches), delete with the Delete key, adding a node from a menu on right click (by category), every edit goes into `Renderer::set_graph`, which compiles it, so the errors and warnings of the compiler are listed under the nodes and the nodes with errors are red; the last version that compiled keeps running. Save writes a project graph back, Save As puts a copy with a new id into `Assets/Graphs`; "Show in Viewport" draws the Viewport with the graph being edited; "Main graph" makes it the main one. Node positions come from the file, nodes without one are laid out in columns by depth, and the positions imnodes holds are written back. The pieces without a window (pins, links, layout, parameters, a UI run with no window) are tested; the panel was not run by hand. Left out: undo, copy and paste of nodes, a view of the textures of a graph (stage 15), renaming a graph.
13. **PBR: the surface model, as soon as possible.** Cook-Torrance (GGX, Smith visibility, Schlick Fresnel) with energy-conserving diffuse, so plastic is plastic (a dielectric: a dull base color and a small white highlight) and metal is metal (the highlight takes the color of the metal, no diffuse); roughness and metallic as numbers and as textures (the packed metallic-roughness texture of glTF), occlusion and emissive maps, a normal scale; image based lighting by the split-sum method (a GGX-prefiltered environment with roughness in the mips, irradiance as spherical harmonics, a BRDF lookup), specular occlusion and horizon fade. See section 24. Check: a row of spheres, roughness from 0 to 1 across and metallic 0 and 1 in two rows, under the sky and a sun, that looks like the reference pictures of the same test in other engines.
14. **PBR: detail from textures.** Texture slots and import settings per kind (sRGB for color, linear for data, channel packing into one texture), UV scale and offset per material, tiling detail maps, parallax occlusion mapping from a height map, triplanar mapping, vertex colors, and the extra layers: clear coat, anisotropy, sheen, transmission (with stage 20). Check: a brick wall with a height map that has depth when looked at from the side, and a floor that does not look tiled from afar.
15. **Line rendering and debug views.** A line renderer in the engine (not only for the editor): world-space lines with a thickness, a color and an optional depth test, batched, as an API for games and tools (`DebugDraw`: line, polyline, box, sphere, circle, arrow, frustum, grid) and as the `zenith.lines` pass; the editor passes (grid, gizmos, outline) move onto it. Debug views: a node `zenith.debug_view` that shows a buffer of the graph instead of the picture, with a mode and a palette: depth (a color ramp from near to far, linear or logarithmic, with the range as parameters), normals, roughness, velocity, entity id (a color per id), overdraw, lights per cluster (a heat map), shadow cascades (a color per cascade), mip level, wireframe. A Debug view menu in the Viewport chooses the mode; every texture slot of any graph can be viewed from the graph editor with a click on its pin. Check: the depth of the sample scene as colors that change with distance, and the lights-per-cluster heat map once stage 18 is there.
16. **The editor: panels.** Details, Hierarchy, the file panel and the
    selection in the Viewport, in the UE-like layout (see "The panels" in
    section 13). Order: the shared selection and the parent/child component,
    then the Hierarchy, the Details made from the component registry,
    selection by a click, and last the file panel with favorites and the
    remembered folders. Check: a model is dragged from the file panel into the
    world, picked in the Viewport, its material changed in the Details, the
    world saved, and after a restart the same folders are open.
17. **Scriptable render passes** (section 20), after the scripting, made by Gleb, can call
    into the engine. Check: a fullscreen effect written as a script and a
    WESL file in the project, put into the graph in the graph editor.
18. **Clustered forward and the buffers.** Compute passes and buffer slots in the graph, frame buffers with history, the depth pre-pass with the thin G-buffer (`depth`, `normal`, `velocity`, `entity_id`), `light_cull`, lights in storage buffers, `SpotLight`, MSAA. Check: 500 point lights in a scene give the same picture as the loop over all of them, and the frame is faster.
19. **Shadows and ambient light.** Cascaded maps, spot and point shadows in an atlas, soft edges, contact shadows, a better bias; prefiltered environment, spherical harmonics, BRDF lookup. Check: a shadow of a cube from each kind of light, no shimmering when the camera moves.
20. **Post-processing.** SSAO, fog, bloom, glare, lens flare, anamorphic streaks, ghost-type lens flare (section 23), vignette, automatic exposure, FXAA and TAA, more tone mappers, color grading, outline by `entity_id`, picking in the Viewport by the same buffer. Check: each as a node of the graph, off when left out, and the editor graph shows the silhouette of the selection.
21. **Atmosphere, volumetric fog and decals.** Atmospheric scattering (lookup tables), height fog, froxel volumetric fog with temporal reprojection, the decal pass. Check: a light shaft through a window, the cost shown in the frame time stays under a set budget.
22. **Software and Hardware graphs.** The `profile` and `platforms` of a graph, `requires` of a pass, the graph set of a project, `builtin_hardware`, ray traced shadows and reflections, the path tracer (Hardware, and the compute one as Software). Check: the same scene in both profiles, and a machine without ray tracing falls back with a log line.
23. **Later.** A sample scene to show the engine: a floor, a wall and a sun, a character that can walk around (like the Hazel one), with shadows and the sky, as the project the editor opens first and the check for the lighting stages.  The asset index in a build, asynchronous loading, culling
    (frustum first, then occlusion; see "Culling"), the parameters of custom shaders read by reflection
    (section 19), removing the old `zenith_renderer`.
24. **The big ones, in pieces** (each piece useful alone): virtual geometry
    (Nanite-like), global illumination (Lumen-like), ReSTIR for the path
    tracer, depth of field with bokeh, motion blur, Gaussian splatting.
    See their sections at the end of section 23.

## 17. Open questions

- Worlds are YAML today (`.zenith`, `serde-saphyr`), and by the new rule a
  scene is JSON with a `uuid`. When and how they move is outside this plan.
- The extensions `.zenithtexture`, `.zenithmaterial` and `.zenithgraph` are
  proposals waiting for a yes.
- Several materials for the sub-meshes of one model.
- A shader found by its name (a path in `Shaders://`) or by a UUID like the
  other assets (in a `// zenith-shader:` line, or a `.zenithshader` JSON
  beside the `.wesl`). The name is proposed: simpler, what shader tools leave
  alone, and the path WESL imports by anyway; the cost is that moving a
  shader breaks the materials and imports that name it, unless the editor
  rewrites them.
- The scripting language of the script passes, and how it calls into the
  engine (Gleb).
- Three things to **[verify]**: `Send + Sync` of wgpu on wasm, registering an
  external texture with `dear-imgui-wgpu`, `dear-imnodes` with
  `dear-imgui-rs` 0.18.

## 18. Later: finding an asset by the hash of its content

Not now; after the editor works with assets.

A reference to a model (or a texture) can also be a path with the hash of
what is expected there: `Assets://Models/Cube.zenithmodel` + hash. If the file
at the path is not that one (another file took its place, or there is
nothing), the asset is looked for by its hash wherever it is now.

- **The hash is stored, never worked out on load.** The container keeps it in
  its header (format version 2: 32 more bytes after the length of the
  settings), so the registry reads it with the id, from the header only. It is
  written when the asset is made (import, copy) and when its source is
  replaced. SHA-256 of the source bytes, from `sha2`, which the engine already
  has.
- **The registry keeps a second index**: hash to the assets that have it
  (copies of one source share a hash).
- **Lookup of a path + hash**, in this order:
  1. the file at the path has that hash: it is the one;
  2. another file has that hash: the asset was moved, use that file (the
     editor can offer to fix the reference);
  3. the file at the path is of the same kind with another hash: its source
     was replaced, use it and warn;
  4. nothing: the stand-in of a lost asset, as for an unknown `AssetId`.
- **With the UUID, not instead of it.** The id stays what components refer
  to; path + hash is for what has no id to hold: a path written in a script, a
  reference from another project, an asset imported again from its source
  (a new id, the same content).
- JSON assets (materials, graphs) are edited by hand and change all the time:
  they get no hash.

## 19. Shaders in the project

**[decided]** The shaders the renderer draws with are files of the project,
and it loads them from there, not from copies built into the engine. A game
can change how the engine draws, and can add shaders of its own.

### Where they are

- `Assets/Shaders/`, with a scheme of its own, `Shaders://` (like
  `Worlds://`, which is `Assets/Worlds`). **[proposed]**
- **[decided]** The shaders are written in **WESL** (WebGPU Extended Shading
  Language): WGSL with modules (`import`) and conditional compilation
  (`@if`). `.wesl` files, text, for diffs and merges. Nothing goes to LFS.
  The engine compiles them to WGSL with the `wesl` crate (0.6, MIT or
  Apache-2.0) when it loads them; wgpu only ever sees WGSL. Plain WGSL is
  valid WESL, so a `.wgsl` file in `Shaders://` is read too.
- The engine's shaders are under `Shaders://Zenith/` (`Zenith/standard.wesl`,
  `Zenith/sky.wesl`, `Zenith/tonemap.wesl`, `Zenith/mipmap.wesl`, ...); the
  game's own are anywhere else in `Shaders://`.
- A shader is found by its name: its path in `Shaders://` without the
  extension (`Zenith/standard`), the same path WESL imports it by
  (`import package::Zenith::common::Camera;`, `Shaders://` being the
  package root). Not by a UUID: a shader file has no place for one that the
  shader tools would leave alone, and a name is what a material, a pass and
  an `import` write. Moving a shader is renaming it. **[proposed]** (the open
  question is in section 17)

### How the engine's shaders get there

- The engine keeps its copies (`include_str!`, `src/shaders/`) only to write
  them. `Project::create` (through the renderer: `zenith_filesystem` knows
  nothing of shaders) writes them into `Shaders://Zenith/`.
- An older project without them: the editor writes the missing ones when it
  opens the project, and says so.
- Each copy starts with a line the engine reads:
  `// zenith-shader: Zenith/standard 3` (its name and version). A newer
  engine with a newer version does not overwrite the file: the project's copy
  may have been changed on purpose. The editor shows "the engine has a newer
  `Zenith/standard`" with a diff, and "Replace" / "Keep". "Restore the
  engine's shaders" writes them all again.
- A built game reads them from its archive, like any other asset (the web
  too).

### Loading

- `GpuCache::shader` already takes a name and a source; a source of shaders
  (`ShaderSources`) gives the text of a name. Until this stage it gives the
  engine's copies; from here on it reads the project. The passes ask by name
  from stage 6 on, so this stage changes where the text comes from, not the
  passes.
- Shared code is WESL modules: `Zenith/common` holds the structs of the
  camera, the lights and the material, and the shaders import what they use.
  The resolver of the `wesl` compiler reads the modules through the project
  (`Shaders://`), so it works in an archive and on the web.
- Variants are WESL conditions, not files: `@if(shadows)`, `@if(normal_map)`.
  The conditions a pipeline is compiled with are part of its key (the
  `ShaderId` stands for a name and a set of conditions), so one file gives
  the variants a pass needs.
- Compiling: WESL to WGSL with `wesl`, then the WGSL is checked with `naga`
  (`wgpu::naga`) before `create_shader_module`, so an error in a project file
  does not reach the device's error handler (which panics). The error, with
  its file and line in the `.wesl`, goes to the log and the editor.
- A built game may get the shaders already compiled to WGSL by
  `zenith_build`, so it carries no compiler (smaller, on the web most of
  all). Until then the runtime compiles them. **[verify]** that `wesl`
  builds for wasm32 and what it adds to its size.
- A shader that is missing or does not compile: the engine's built-in copy
  stands in, like a lost asset, with one warning; if it is not an engine
  shader, the magenta shader. A broken edit while the game runs keeps the
  last version that compiled.
- Hot reload: the watcher of the editor sees the file change, the cache makes
  the shader again under the same id and drops its pipelines (that part is
  done in stage 3); the passes ask for them again in `prepare`.

### Custom shaders

- A material names its shader: `"shader": "Zenith/standard"` (the default),
  or `"shader": "Effects/toon"`. The old `"standard"` is read as
  `Zenith/standard`.
- What a material shader must follow, the contract with the passes that draw
  meshes:
  - the entry points `vs_main` and `fs_main` (the shadow pass also looks for
    `vs_shadow`, the depth-only version);
  - the vertex inputs of `GpuMesh::LAYOUT` (`@location(0..4)`) and of the
    instance buffer (stage 5 fixes its locations);
  - group 0 is the pass's (the camera, the lights, the shadow map, the
    environment: their structs are in `Zenith/common`), group 1 the
    material's, groups 2 and 3 are not for a material shader.
- Group 1 of a custom shader: the layout of `cache/materials.rs` (a uniform
  and two textures) in the first version. Later the shader says what it
  takes: a `struct MaterialParams` and textures named in group 1, read from
  the compiled WGSL by `naga`'s reflection. The material file then has
  `"params": { "rim_color": [1, 0.5, 0, 1] }` and
  `"textures": { "ramp": "<uuid>" }`, and the material inspector of the editor
  builds its widgets from the same reflection. **[proposed]**
- The pipeline key has the shader in it, so the opaque and transparent passes
  sort by shader before material, and a shader that fails to compile falls
  back as above.

## 20. Later: scriptable render passes (SRP)

**[decided]** Later, after the scripting is done (stage 17 of section 16; Gleb writes the scripting and this part, not the renderer work): render passes written
in the scripting language of the engine (Gleb's part), not in Rust, that the
graph uses like any other pass.

- A script pass registers itself in the `PassRegistry`: its `PassDesc` (slots,
  parameters, category) comes from the script, so it is a node in the graph
  editor with pins and widgets like a built-in pass. For this `PassDesc` and
  the other descriptions must not hold `&'static str` only: they get owned
  names (`Cow<'static, str>`) when the graph core is written (stage 4), so
  nothing has to be rewritten later. The same goes for the entry points in
  `PipelineKey`.
- What a script pass can do is the contract of section 10 and nothing more:
  ask for a pipeline from a shader of the project and a key, read and write
  its slots, set its parameters, and record commands through a small API
  (`set_pipeline`, `bind`, `draw_list("meshes")`, `draw_fullscreen`,
  `dispatch` for compute later). No raw wgpu, no resource made in `exec`.
- The cost of a script is paid per pass, not per draw: `draw_list` runs the
  draws of a whole slot natively.
- Its shaders are custom shaders of section 19, so a script pass plus a WESL
  file in `Shaders://` is a new effect with no Rust and no rebuild of the
  engine.
- What is decided before it: the scripting language and how it calls into
  the engine (Gleb), and that the renderer runs its passes on the main thread
  (it does now).

## 21. Lighting, buffers and effects

### What is drawn today

| Effect | State today |
| --- | --- |
| Shading | PBR (base color, roughness, metallic, normal map, alpha cutoff), forward, one pass per phase (opaque, transparent) |
| Lights | up to 4 directional and 16 point, in one uniform buffer, every pixel loops over all of them |
| Shadows | one directional light, one 2048 orthographic map, hardware comparison sampler (2x2 PCF), a fixed distance and bias |
| Environment | an equirectangular picture: the sky, ambient light and reflections (no prefiltering yet, so the ambient light is sharp) |
| Transparency | sorted blending, no depth write |
| 2D | sprites with an order, a UV rectangle and a tint |
| Tone mapping | ACES, Reinhard, none; exposure |
| Editor | grid, gizmo lines, selection box |

Nothing else: no bloom, no ambient occlusion, no fog, no anti-aliasing, no
screen-space effects, no post-processing besides tone mapping.

### Forward+ (decided: it is built clustered)

**Decision:** the light grid is **clustered** from the start (a tile on the
screen times depth slices, 16 x 9 x 24 cells to begin with, the slices on an
exponential curve), because it fixes the transparency case below and the
volumetric fog and the decals reuse the same grid. "Tile" in the text that
follows reads as "cluster"; the depth range of a tile is then not needed, the
culling is a light against the cells.

The lighting becomes **Forward+** (tiled forward). Meshes are still shaded in
one pass, with the material they have, so MSAA, blending and custom material
shaders keep working. What changes is how a pixel finds its lights.

1. `zenith.depth_prepass`: draws the opaque and masked meshes into the depth
   buffer (and the thin G-buffer, below) first.
2. `zenith.light_cull`: a **compute** pass. It splits the screen into tiles
   (16 x 16 pixels), reads the smallest and the largest depth of each tile,
   and writes for each tile the list of lights that touch it (a storage
   buffer: a count and indices per tile).
3. `zenith.opaque` reads the list of its tile and loops over those lights
   only; the depth test is `Equal` with no depth write, so each pixel is
   shaded once.
4. `zenith.transparent` uses the same lists. The tile depth range comes from
   the opaque depth only, which is wrong for a surface in front of it, so the
   culling is done with the near plane for the far end: a little more lights
   per tile, no light missing. Clustered (depth slices as well) is the next
   step if that costs too much.

The lights move from the uniform into storage buffers, so the 4 and 16 limits
go: the number is the size of the buffer (a setting, 1024 point and spot
lights to start with). Directional lights are few and affect every pixel, so
they stay out of the culling and are looped over directly. New light type:
`SpotLight` (position, direction, range, inner and outer angle). Area lights
are later.

What it needs from the graph:

- **Compute passes.** `RenderPass` gets a `begin_compute_pass` in
  `ExecContext` next to `begin_render_pass`, and a `PassCategory::Compute` for
  the colors of the nodes. `PipelineKey` gets a compute variant. WebGPU has
  compute, storage buffers and `atomics`; the limits to respect are
  `maxStorageBuffersPerShaderStage` (8 by default) and
  `maxComputeWorkgroupSize` (256).
- **Buffer slots.** A slot kind for a storage buffer beside the textures and
  the data (`SlotKind::Buffer`), created by a pass with a size rule, so the
  light list is a resource of the graph and not a hidden one.
- The `lights` data slot stays as the CPU side of the lights; the new
  `light_grid` slot carries the tile lists to the passes that shade.

### G-buffers

Forward+ has no fat G-buffer for lighting. What the effects need (ambient
occlusion, reflections, decals, motion blur, picking, outlines) comes from a
**thin G-buffer** that the depth pre-pass writes as extra color targets, so
it costs one pass and no more geometry:

| Target | Format | What | Used by |
| --- | --- | --- | --- |
| `depth` | `Depth32Float` | scene depth | culling, AO, fog, DoF, soft particles, decals |
| `normal` | `Rgb10a2Unorm` (octahedral, roughness in the last bits) | normal and roughness | SSAO, SSR, decals |
| `velocity` | `Rg16Float` | motion of a pixel since the last frame | TAA, motion blur |
| `entity_id` | `R32Uint` | the entity of the pixel | picking in the Viewport, outline by silhouette |

Each is an output of the pre-pass in the graph and can be left out of a graph
that does not use it (an unlinked output is not allocated). The editor
graph keeps `entity_id`, which gives exact picking and a real silhouette
outline instead of a box. A fully deferred shading path is not planned.

### Frame buffers

The graph already makes transient textures from a pool. A **frame buffer**
is the named set of attachments a group of passes draws into, and it also
covers what must live longer than one frame:

- `FrameBuffer` in the description of a graph: a name, a list of color
  attachments, an optional depth, a size rule (`FromParam`, a fraction of the
  view, a fixed size) and a format rule. Passes name a frame buffer instead of
  each slot, so `depth_prepass` and `opaque` share one set.
- **History.** A texture can be marked `history`: the pass reads the one of
  the last frame and writes the new one; the two swap each frame, per view.
  This is what TAA, automatic exposure and temporal AO need.
- **Resolution scale.** The scene frame buffers can be a fraction of the
  target (a setting), and the last passes scale up; the UI stays sharp.
- A camera that draws into a render texture (`CameraTarget::Texture`) is a
  frame buffer an asset can read.

### Effects, one by one

State: **done**, **planned** (with the stage) or **later**.

**Shadows** (they are different kinds, one per light kind):

| Kind | What | State |
| --- | --- | --- |
| Directional, one map | orthographic, PCF | done |
| Cascaded shadow maps | 3 to 4 cascades fitted to the camera frustum, a blend between cascades, a texel-snapped matrix against shimmering; SDSM fitting as an option | planned, stage 19 |
| Spot light | one perspective map | planned, stage 19 |
| Point light | six faces, in an atlas (no cube arrays on the web) | planned, stage 19 |
| Soft edge | PCF with a rotated Poisson disk, and PCSS (the radius follows the distance to the blocker); the filter is a setting of the light: hardware PCF, Poisson PCF, PCSS | **decided**, stage 19 |
| Contact shadows | a short screen-space ray march toward the light, over small details | planned, stage 19 |
| Shadow bias | constant, slope-scaled and normal offset together, and receiver plane bias for the wide kernels; all are settings of the light | **decided**, stage 19 |
| Static shadow cache | maps of lights that did not move are kept | later |

The shadow maps are packed into one atlas with a list per light, so the
number of shadowed lights is a budget, not a limit of the shader.

**Ambient light and reflections**

| Effect | State |
| --- | --- |
| Image based lighting from the environment picture | done (diffuse and specular from the same picture) |
| Prefiltered specular (roughness in the mips), irradiance as spherical harmonics, BRDF lookup | planned, stage 19 |
| Reflection probes (box-projected) | later |
| SSAO, as GTAO (depth and normal) | planned, stage 20 |
| Screen-space reflections (hierarchical depth march) | later |
| Specular occlusion from AO | planned, stage 20 |

**Anti-aliasing**

| Effect | State |
| --- | --- |
| MSAA (2x, 4x, 8x on the forward passes) | **decided**, stage 18 (a setting, the depth and the buffers resolve) |
| SSAA (a render scale above 1) | **decided**, stage 18 (with the resolution scale of the frame buffers) |
| FXAA | **decided**, stage 20 (the cheap one for the web) |
| SMAA (1x and T2x) | **decided**, stage 20 |
| CMAA2 | **decided**, stage 20 |
| TAA (jitter, velocity, history, clamp and sharpen) | **decided**, stage 20 |

**Post-processing** (all as nodes of the graph, in this order in the default
graph):

| Effect | Notes | State |
| --- | --- | --- |
| Fog | exponential height fog, a color or the sky | planned, stage 20 |
| Bloom | downsample and upsample chain (no big blur), a threshold and an intensity | planned, stage 20 |
| Automatic exposure | a histogram of the scene luminance, eye adaptation over time | planned, stage 20 |
| Depth of field | circle of confusion from depth, a bokeh gather | later |
| Motion blur | from the velocity buffer | later |
| Tone mapping | ACES, Reinhard, none; AgX and Khronos PBR Neutral to add | done / planned, stage 20 |
| Color grading | a 3D lookup table and the usual sliders (contrast, saturation, white balance) | planned, stage 20 |
| Decals | boxes projected onto the depth and the normal | later |
| Outline | silhouette from `entity_id`, editor selection first | planned, stage 20 |

**Other**

| Effect | State |
| --- | --- |
| Atmospheric scattering (the sky, the haze with distance, the color of the sun), section 22 | planned, stage 21 |
| Volumetric fog and light shafts | froxels, section 22 | planned, stage 21 |
| Order independent transparency (weighted blended) | later, if sorting is not enough |
| Particles (compute, billboards) | later |
| Path tracing, ray traced shadows, reflections, GI | the Hardware profile and a slow Software path tracer, section 22 | planned, stage 22 |
| Global illumination | not planned; baked lightmaps or probes are a later question |

Every effect is a node with parameters in the graph, so it is off by being
left out, and the cost is paid only when it is in. Each needs the compute and
the frame-buffer parts above, which is why they come first.

## 22. Techniques, and graphs for Software and Hardware

### The kinds of forward and deferred

| Name | How a pixel finds its lights | Good | Bad |
| --- | --- | --- | --- |
| Forward | loops over all lights | simple, MSAA, blending | cost grows with lights |
| **Forward+** (tiled forward) | a list per 16 x 16 tile | many lights, MSAA, blending, any material | the depth range of a tile is wide at edges; transparent surfaces need care |
| **Clustered forward** | a list per 3D cell (tile and depth slice) | the lists are tight, transparency works, volumetric fog reuses it | more memory and more work in culling |
| Deferred | lights shade a G-buffer | cost does not depend on geometry | no MSAA, no blending, a fat G-buffer, bandwidth |
| Tiled / clustered deferred | the same lists over a G-buffer | many lights, cheap | the same limits as deferred |
| Light indexed deferred | lights drawn as volumes | few lights are cheap | overdraw of volumes |
| Visibility buffer | triangle ids stored, shaded later | the least bandwidth, material batching | needs bindless and a lot of work |

The plan is Forward+ first, with the light list built so that it can become
clustered (one more dimension) without touching the passes that read it.

### Shadow algorithms

**Making the map**
- Shadow mapping: depth from the light, compared in the shader. What we do.
- Cascaded shadow maps (CSM / PSSM): several maps along the view, the near
  ones sharp. Cascade splits are a mix of linear and logarithmic.
- SDSM: the splits fitted to the depth that is really on the screen (a
  depth reduction in compute finds the nearest and the farthest pixel, the
  cascades are made to fit that). Planned for stage 19 as an option over the
  plain CSM.
- Point lights: six faces (cube) or an atlas of six squares; the web has no
  cube arrays, so an atlas. Dual paraboloid is cheaper but has artefacts.
- Virtual shadow maps (UE5): a huge sparse map. Out of reach, not planned.

**Filtering the edge**
- Hardware PCF: the comparison sampler does 2 x 2 for free. Done.
- PCF with a kernel (Poisson disk, rotated by a noise): the edge gets soft
  and noisy-but-smooth. Stage 19.
- PCSS: first finds the blockers, then sets the radius by the distance, so a
  shadow is sharp at the foot and soft far away. Costs a lot of taps.
- VSM, ESM, EVSM, MSM: store moments, so the map can be blurred and sampled
  with a filter. Light bleeding or a lot of memory; not planned.

**Bias**
- Constant plus slope-scaled depth bias, and a **normal offset** (move the
  point along the normal by a texel); receiver plane depth bias for large
  PCF kernels. A bias is a setting of the light.

**Without a map**
- Contact shadows: a few steps of a ray in the depth buffer toward the light.
  Catches what the map is too coarse for. Stage 19.
- Screen-space shadows are the same thing for the whole light.
- Distance field shadows and capsule shadows (UE): need a distance field of
  the meshes. Later, maybe.
- Ray traced shadows: exact and soft, in the Hardware graph (below).
- Baked shadows: lightmaps. A later question.

### Anti-aliasing

| Kind | What it does | Cost | Notes |
| --- | --- | --- | --- |
| MSAA 2x/4x/8x | samples the edges of triangles | memory and bandwidth | forward only; the depth resolves; no help for shader aliasing |
| SSAA | draws bigger, shrinks | very high | a "render scale" above 1 does it |
| FXAA | one post pass on the picture | tiny | blurs; the web fallback |
| SMAA (1x, T2x) | finds edge shapes, blends | small | sharper than FXAA |
| CMAA2 | compute edge pass | small | |
| **TAA** | jitter per frame and a history | small, needs velocity | blurs a bit, ghosting; fixed with a sharpen and a clamp |
| Upscalers (TSR style, FSR 2/3) | TAA at a lower resolution, scaled up | gains time | a later node; DLSS/XeSS are vendor SDKs and not reachable from `wgpu` |
| Geometric / specular AA | widens roughness by the variance of the normal | free | always on in the shader |
| Alpha to coverage | soft edges of masked materials (leaves) | free with MSAA | a material flag |

All of them are nodes, so a graph picks what it wants. The setting
`anti_aliasing` is a list that may hold several, with the rules that make
sense: geometric AA always; MSAA with FXAA or SMAA; TAA instead of FXAA and
SMAA (it can run with MSAA on, which is expensive); an upscaler needs TAA's
inputs. The editor shows the combination as the nodes it makes.

### Decals

A decal is a picture put on top of the surfaces that are already there: a
bullet hole, blood, dirt, a road mark, a painted sign.
- **Mesh decals**: a plane drawn on the surface with a depth bias. The
  simplest; it needs a mesh that fits.
- **Projected (box) decals**: a box volume in the world; every pixel of the
  scene that is inside the box takes the picture projected along the box.
  This needs the depth (and the normal, to fade at steep angles), which the
  thin G-buffer has.
- In deferred these are written into the G-buffer. Here (forward) there are
  two ways: **clustered decals**, which go into the same cells as the lights
  and are blended by the material shader (works with every material, costs
  every pixel a loop), or a **decal pass** after the opaque pass that draws
  the boxes over the color (cheap, the lighting is the one of the pixel under
  it). The first version is the pass; clustered decals when the lights are
  clustered.
- Fields: a material or a texture set (color, normal, roughness), a size, a
  fade angle, a sort order, a lifetime (so they can disappear).

### Volumetric light and fog, and keeping them cheap

It is not hard if it is made in the cheap way. What is cheap:

1. **Height fog**, analytic: no marching, the fog on a ray has a closed form.
   One line in the final pass. Always on in every graph.
2. **Light shafts as a screen-space blur** from the sun: nearly free, fake,
   needs the sun on the screen. Optional.
3. **Froxel volumetric fog** (what shipped games use): a 3D texture lined up
   with the camera, for example 160 x 90 x 64 cells with the depth slices on
   a curve. Compute passes: (a) write the density and the scattering of each
   cell, (b) add each light (the light cells of Forward+ or the clustered
   grid) with its shadow map, (c) integrate along the depth. The shading
   pass reads the result with one texture sample per pixel. The cost does not
   grow with the screen size, only with the cells. Two more things make it
   cheap: **temporal reprojection** (jitter the cells per frame and blend with
   the last frame, so it can use few samples per cell) and a **lower
   resolution** than the screen.
4. Ray marching the sun through the cascades with few steps and blue noise,
   upscaled: only if the froxels are not enough.

The web can do it: 3D storage textures with `rgba16float` are allowed for
writing in WebGPU. It needs the compute pass, the frame buffers with history
and the shadow atlas, so it comes after stages 16 and 17 (stage 21).
Parameters: density, height falloff, scattering color, anisotropy, the
distance, the cell counts, and per light a "volumetric" intensity.

### Graphs for Software and Hardware

A **profile** says what the graph needs from the machine:

- **Software**: rasterization and compute shaders only. It runs on every
  device that WebGPU runs on, the web too. Forward+, shadow maps, SSAO, SSR,
  froxel fog, TAA: all of section 21. This is the default.
- **Hardware**: needs ray tracing hardware (ray queries, an acceleration
  structure). Ray traced shadows, reflections, ambient occlusion and global
  illumination, and the **path tracer**. In `wgpu` ray tracing is behind an
  experimental feature on Vulkan, DX12 and Metal and does not exist on the
  web, so this profile is native only. **[verify]** the state of the feature
  in the `wgpu` version in use.

How it fits the graphs:

- A graph asset gets a field `profile` (`software` or `hardware`) and a
  `platforms` list (for example `desktop`, `web`, `mobile`); empty means all.
  A pass has a `requires` in its description (a feature of the device, a
  limit), and a graph that uses a pass the machine cannot run does not
  compile for it, with the reason in the diagnostics.
- **The graph set** of a project (a file in `Settings/`): for each platform,
  the graph per profile. The renderer decides at start: the Hardware graph if
  the device has the features and the setting allows it, else the Software
  one. The camera may name a graph (it wins); then the main graph of the
  profile; then the built-in graph of the profile.
- The built-in graphs are `builtin_software` (what we have, growing with the
  stages) and `builtin_hardware` (stage 22). The editor graph is the same
  idea: `editor_software`, `editor_hardware`.
- The graph editor shows the profile and marks nodes that the other profile
  cannot run.
- A game can switch the profile at run time (the quality menu); passes are
  made again for the new graph, the cache stays.

**Path tracing**
- *Hardware*: one ray query path tracer pass: a primary ray per pixel, a few
  bounces, next event estimation toward the lights, the same materials as the
  raster path (so the PBR parameters are shared), accumulation over frames
  while the camera is still, and a denoiser (a classic one, no neural network:
  an edge-aware a-trous wavelet filter guided by the normal, the depth and
  the albedo, with a temporal part like SVGF) for real-time use. It is a node: `zenith.path_trace`.
- *Software*: the same algorithm with the BVH built on the CPU and traversed in
  a compute shader. It runs everywhere and is slow, so it is for the
  reference picture in the editor ("bake a still") and for the web where
  nothing else exists. Only the traversal differs; the shading code is
  shared.
- Both are for the editor and for games that want it; the raster graphs do
  not depend on them.

### Atmospheric scattering

The blue of the sky, the red of the sunset, and the haze that makes far
mountains pale and blue (**aerial perspective**) come from light scattering in
the air. The way to do it in real time (Hillaire, 2020) is lookup tables that
are small and cheap to update:

1. **Transmittance table** (2D): how much light of the sun reaches a point at
   a height, along an angle. Made once, remade when the atmosphere settings
   change.
2. **Multiple scattering table** (2D): the light that scattered more than
   once; it keeps the sky from going dark.
3. **Sky-view table** (2D, per frame, small): the color of the sky in every
   direction from the camera. The sky pass reads it instead of marching a ray
   per pixel.
4. **Aerial perspective** (a 3D table in the camera froxels, shared with the
   volumetric fog): the haze and the loss of color along the way to every
   pixel. The scene pass applies it with one sample, so far geometry gets
   the blue.

Parameters: the planet and atmosphere radius, Rayleigh (blue, air),
Mie (white, dust and haze), ozone, the sun direction and color; the
directional light of the world can drive it, so the sky follows the sun. The
tables also give the ambient light and the color of the sun on the ground, so
a sunset turns the whole scene. It replaces the environment picture as the
sky when it is on (the picture stays for the reflections, or is rendered
from the atmosphere into a cube map). It is compute and small textures, so it
runs on the web. It is a node: `zenith.atmosphere`.

### Light through transparent things

What a transparent material can do, from cheap to costly:

1. **Blending** (what we have): the color of the surface mixes with what is
   behind it by alpha. No bending of light at all.
2. **Absorption** (Beer-Lambert): glass and water take light out by the
   thickness, so thick stained glass is darker and more colored than thin.
   Needs the thickness (the depth behind minus the depth of the surface, or a
   value in the material). Cheap.
3. **Transmission with refraction (screen space)**: the opaque pass is copied
   (with its mips), and the transparent surface samples that copy at a point
   moved along the refracted direction by the normal and the index of
   refraction. Roughness picks a blurrier mip (frosted glass). Cheap, looks
   right for bottles and windows; it cannot show what is not on the screen or
   what is in front of the glass. Stage 20 (it needs the frame buffers and the
   copy of the color).
4. **Dispersion**: the index of refraction depends on the color, so the
   colors bend by different amounts. In screen space it is done by sampling
   the copy three times (red, green, blue) with three slightly different
   indices: colored fringes on the edges of glass. Cheap, a parameter of the
   material (`abbe` number).
5. **Subsurface scattering** (skin, wax, marble, leaves): light goes in, wanders
   and comes out nearby. In raster it is a blur of the diffuse light in
   screen space or a thickness-based glow; real in the path tracer.
6. **Real refraction**: light follows the glass, bends at each surface and may
   bounce inside. Needs ray tracing: the Hardware profile and the software
   path tracer, stage 22.

**A prism that turns white light into a rainbow.** Dispersion is the same
effect: the index of refraction of glass is a bit larger for blue than for
red, so a beam that goes through a prism fans out. How hard it is depends on
what is wanted:

- *Seeing the colors in the prism* (fringes, a rainbow inside the glass, edges
  of a lens): easy, point 4 gives it.
- *A decoration* (a beam of light with a rainbow fan on a wall): easy and
  honest, a mesh or a decal with a gradient made by hand and driven by the
  angle. It is a fake but it looks the same, and it is what most games do.
- *The real thing: a white beam goes through a modelled prism and paints a
  rainbow on a wall.* That is a **caustic**, and it is the hard case for any
  renderer. It needs the Hardware profile: a spectral path tracer (each path
  picks a wavelength, the index of refraction comes from a formula like
  Cauchy or Sellmeier, the result is turned into a color with the standard
  color matching curves). The dispersion in the glass itself is easy in that
  tracer; what is hard is that the light reaches the wall only through the
  glass, and a path that starts at the camera and tries to connect to a
  light cannot do so through a refracting surface. So the wall stays black
  and noisy unless the tracer also follows light from the lamp
  (bidirectional tracing, photon mapping or light tracing). That is a
  separate piece of work after the path tracer works, and the result is a
  slow picture that converges over seconds, not a real-time effect. It is
  noted as **later**, after stage 22.

So: rainbows in the glass, soon and cheap; a rainbow on the wall, as a fake
at once or physically much later.

## 23. Bright spots, lens effects and the rest of the catalog

### What a bright thing leaves on the picture

These are different effects with different causes; a graph can have any of
them, each as its own node (or one node with switches).

| Effect | What it looks like | Cause | How it is made | State |
| --- | --- | --- | --- | --- |
| **Bloom** | a soft glow around bright areas | light scattering in the eye or the lens | a chain of downsamples and upsamples of the bright part, added back | planned, stage 20 |
| **Glare / starburst** | rays or a star from a bright point | diffraction at the aperture blades and the lashes | a streak filter: the bright part is smeared along a few directions (4, 6, 8 rays, a rotation) | planned, stage 20 |
| **Anamorphic streak** | one long horizontal (often blue) line | the cylindrical lens of a cinema camera | a streak filter along X only, tinted | planned, stage 20 |
| **Lens flare: ghosts** | a row of colored discs on the line through the screen center | reflections between the lens elements | the bright part is mirrored through the center and repeated with a tint (screen space), or sprites placed along the line | planned, stage 20 |
| **Lens flare: halo** | a ring around the center | the same, the ring is the shape of the aperture | a ring sampled from the mirrored bright part | planned, stage 20 |
| **Bokeh** | bright discs (or polygons) in the blur of what is out of focus | the aperture shape in the depth of field | a gather or scatter blur by the circle of confusion; the shape is a texture or a polygon of N blades | with the depth of field, later |
| **Light shafts / god rays** | beams in dust or mist | scattering along the ray | section 22 (screen space or froxel fog) | planned, stage 21 |
| **Sun disc and corona** | a bright disc with a halo | the sun itself | in the atmosphere node (section 22) | planned, stage 21 |
| **Flare sprites** | the same as lens flare but from a game object, hidden by what stands in front | an artist's choice | a sprite at the light's screen position, scaled by an occlusion test (a few depth samples) | later |

Anything that adds light needs the scene in **HDR** (we have that), and the
bright part is picked by a threshold or, better, by the exposure, so a lamp
in a dark room blooms and a lamp in daylight does not.

### The rest of the catalog

What the lists of modern techniques name, with what we do about each.

| Technique | What it is | Plan |
| --- | --- | --- |
| Motion blur | smear along the velocity | later, after TAA (needs the same buffer) |
| Depth of field and bokeh | blur by the distance from the focus plane, with bright discs | later, after stage 20 |
| Subsurface scattering | light that enters a material and leaves nearby (skin, wax, leaves) | screen-space blur of the diffuse light for raster, later; true in the path tracer |
| Iridescence / thin film | rainbow on soap, oil, beetles, heated metal | a material parameter (thickness and index of the film), the shader computes the interference per color; cheap, planned with the material work |
| Dispersion | the rainbow in glass | section 22 |
| Caustics | patterns of light from glass and water | section 22: a fake for water (animated texture projected by the light), the real one later in the Hardware profile |
| Displacement | the surface really moved by a height map | needs tessellation or a dense mesh; later |
| Parallax occlusion mapping | the look of relief on a flat surface by a ray in the height map | a material option, later; cheaper than displacement |
| Decals | section 22 | stage 21 |
| Global illumination (Lumen-like radiance cache) | many bounces of light with no ray tracing | later; the cheap steps are probes with irradiance (DDGI style) and screen-space GI; a big job, not before the Hardware profile |
| ReSTIR | picks the important lights among millions by resampling over space and time | later, only for the Hardware path tracer; the clustered grid covers the raster side |
| Spectral rendering | light as wavelengths, not as RGB | an option of the path tracer in stage 22 (it is what makes a real prism and the colors of gems right) |
| Nanite (virtual geometry) | clusters of triangles with their own LOD, drawn by compute | not planned; the first step is meshlets, LODs by hand, then frustum culling (stage 23) |
| Mesh shaders | the GPU decides which triangles to draw before the rasterizer | not in WebGPU; not planned |
| 3D Gaussian splatting | a scene made of millions of translucent colored ellipsoids from photos | feasible (we would only draw ready files; making them needs training, which is not ours): a compute sort plus a splat pass, an asset type `.zenithsplat`; later, as an addition |

### Decided not to do

- **Effects that only spoil the picture**: film grain, halation, chromatic
  aberration, lens dirt, veiling glare. Not planned. (The vignette stays: it
  gives the picture a mood.)
- **Anything that needs a neural network** (DLSS and its relatives, ray
  reconstruction, NeRF, neural texture compression): training and data are
  out of reach, and the vendors' versions are not reachable from `wgpu`.

### Added back, and the big ones for later

- **Vignette**: a node of stage 20 (a darkening toward the corners, with a
  radius, a softness and a color).
- **Denoiser**: classic and not neural, in stage 22 with the path tracer (an
  a-trous filter guided by the normal, the depth and the albedo, plus a
  temporal part like SVGF). The ray traced shadows and reflections use the
  same node.

### ReSTIR (later, with the Hardware profile)

Resampled importance sampling of lights: every pixel keeps a small
"reservoir" with one chosen light and its weight, offers a few random lights
to it, then mixes it with the reservoirs of the neighbors (space) and of the
pixel in the last frame (time), and casts a shadow ray only to the survivor.
Many lights cost about as much as a few.

Where it is useful here:
- the Hardware path tracer and the ray traced shadows (stage 22) in a scene
  with many small lights: lamps along a street, neon signs, a city at night,
  emissive meshes as lights (a screen, lava, a fire made of many sparks);
- direct light from **emissive triangles**, which the raster path cannot
  treat as lights at all;
- later the same idea for indirect light (ReSTIR GI), as the way to make
  the path tracer usable in real time.
It needs per-pixel history (the frame buffers with history of stage 18), the
G-buffer (normal, depth, velocity) and a denoiser behind it. The raster path
does not need it: the clustered light grid is the answer there.

### Nanite-like virtual geometry (possible, later)

Clusters of about 128 triangles in a hierarchy of levels of detail, a compute
pass that chooses for every frame the clusters whose size on the screen is
about a pixel, a culling pass (frustum and occlusion with the depth of the
last frame), and a visibility buffer (`entity_id` plus the triangle id) so
the material is shaded once per pixel. Small triangles are drawn by a
compute rasterizer, the big ones by the hardware. It is done in **several
passes of work**, each one useful alone:
1. meshlets (clusters) in the model asset, frustum culling per cluster;
2. the hierarchy of levels of detail built at import (a cluster group is
   merged and simplified, with the error kept);
3. choosing clusters by error on the GPU, and the visibility buffer;
4. occlusion culling with the hierarchical depth;
5. the compute rasterizer for tiny triangles, and streaming of clusters.
It needs compute, indirect drawing and storage buffers (WebGPU has them);
no mesh shaders. After stage 23.

### Lumen-like global illumination (possible, later)

Light that bounces between surfaces in real time, with no baked lightmaps. Also
in steps, each one useful alone and each a node of the graph:
1. **Screen-space GI**: bounces from what is on the screen (cheap, needs the
   thin G-buffer and the history).
2. **Probes with irradiance** (DDGI): a grid of light probes in the world that
   are updated a few per frame (ray traced in the Hardware profile, from a
   simple scene copy in the Software one), sampled by the shading.
3. **A surface cache**: cards, a small lit picture of each mesh from a few
   sides, which is what the rays read instead of the full material.
4. **A world radiance cache** that the screen probes read, plus a distance
   field of the scene (or the hardware rays) for the tracing, and a temporal
   filter.
5. **Reflections** from the same cache, so metals and wet floors are lit by
   the room, not only by the sky.
It is the biggest piece of the list and sits after the Hardware profile
(stage 22) and the denoiser; stages 1 and 2 may come earlier.

### Culling (later)

Not drawing what cannot be seen. Today every model is drawn and only the GPU
clips it. The kinds, in the order they pay off:

| Kind | What it removes | How |
| --- | --- | --- |
| Back-face culling | triangles that face away | the pipeline state (`cull_mode`); done for single-sided materials |
| **Frustum culling** | models outside the view | a bounding box (`GpuMesh::bounds`, the transform) against the six planes; on the CPU first, in compute later |
| Distance culling | models too far or too small on the screen | a size threshold per model, with a fade; a setting of the model |
| **Occlusion culling** | models hidden behind others | the depth of the last frame as a pyramid (hierarchical Z), the bounding box is tested against it in compute; the result decides the indirect draws |
| Shadow culling | casters that no visible receiver sees | frustum of the light, per cascade |
| Light culling | lights that touch nothing | already in the clustered grid |
| Portals / PVS | rooms that cannot be seen | for indoor levels only; not planned |
| Cluster culling | parts of a model | with the virtual geometry above |

It is stage 23 work: frustum culling first (simple and the biggest win),
then the depth pyramid and occlusion culling, which share their work with the
virtual geometry.

## 24. PBR

### What there is today

`standard.wesl` shades with Lambert for the diffuse part and **Blinn-Phong**
for the highlight; the shininess is made from the roughness, and the Fresnel
term is a guess. `roughness` and `metallic` exist, but only as two numbers
per material, and they do the right thing only roughly: `metallic` takes the
diffuse away and colors the highlight, and that is all. The material has a
base color (and its texture) and a normal map; it has no texture for
roughness and metallic, no occlusion, no emissive. The sky gives the ambient
light as a blurry look at the picture (a high mip level), which is not
prefiltered, and the picture of a sky is imported without mips, so the
ambient light is sharp. The vertices already carry the tangent and the
bitangent, and the shader still takes the frame of the normal map from
derivatives.

So the engine has the *parameters* of PBR and none of its *model*: plastic and
metal look different, but not right, and a rough surface does not look like
a rough one.

### Stage 13: the model (small to medium)

1. **The BRDF**: GGX distribution, height-correlated Smith visibility,
   Schlick Fresnel with `F0 = mix(0.04, base_color, metallic)`; the diffuse
   part is Lambert, scaled by `(1 - F)` and `(1 - metallic)` so that the
   energy is kept. One function used by the directional lights and the point
   lights (and later by the clustered lights and the spot lights).
2. **Material fields and textures** (the format is changed, the version of
   `MaterialData` goes up, and old files still read): `metallic_roughness`
   texture (glTF layout: roughness in G, metallic in B), `occlusion` texture
   (R, often in the same picture), `emissive` color and texture and an
   intensity, `normal_scale`, `occlusion_strength`. The importer of glTF fills
   them. `roughness` and `metallic` stay as multipliers.
3. **Image based lighting (split sum)**:
   - **Prefiltered specular**: the environment picture is filtered by GGX
     into the mips of a texture, a level per roughness. It is made by a render
     pass (the compute passes come with stage 18, and this must not wait), once
     when the sky is loaded.
   - **Diffuse irradiance** as 9 spherical harmonics coefficients, computed on
     the CPU when the sky is loaded (the picture is small enough) and put into
     the scene uniform.
   - **BRDF lookup** (a 2D table of the scale and the bias of the Fresnel by
     roughness and view angle): made once by a render pass at the start (or the
     analytic approximation of Karis first).
   - **Specular occlusion** from the occlusion map and a horizon fade, so a
     reflection does not show through a crevice.
4. **Tangents from the vertices** (the frame of the mesh) in place of the
   derivatives, so a normal map does not change with the size of the triangle.
5. The sky node and the environment data carry the prefiltered picture and the
   coefficients; the sky is drawn from the original picture.

The cost is mostly one shader rewrite, a bigger material, and two small
render passes at load. It is a few hundred lines and needs no new part of the
graph. The risk is in the details (a table that is a little off makes metal
too dark or too bright), so the check is a **row of spheres**, the usual test:
roughness from 0 to 1 along a row, a dielectric row and a metal row, under a
sky and a sun, compared with the same test in other renderers.

### Stage 14: detail from textures (medium)

- **Slots and import**: every texture slot of a material says how its picture
  is read (sRGB for a color, linear for data), and the importer can **pack**
  channels: separate roughness, metallic and occlusion pictures become one
  texture (ORM), as the engines do, to save samples and memory.
- **UV transform** per material (scale, offset, rotation), so one texture can
  tile a floor, and **detail maps**: a second albedo/normal at a higher
  frequency that fades in near the camera, so a floor has detail up close and
  does not look like a repeated tile far away. A **triplanar** option for
  terrain and rocks with no UV.
- **Height**: a height map for **parallax occlusion mapping** (a short ray
  march in the height map, a few steps, with a distance fade): a wall of
  bricks has depth when looked at from the side. Real displacement needs
  tessellation and is later.
- **Vertex colors** as a multiplier and as a mask between two materials.
- **Extra layers**, each with a few parameters and a few lines of shader:
  **clear coat** (a second thin specular lobe: car paint, varnished wood),
  **anisotropy** (brushed metal, hair), **sheen** (cloth), **transmission and
  thickness** (glass; the refraction itself comes with the post-processing
  stage), and the glTF extensions that carry them (`KHR_materials_*`).
- A material editor panel: sliders and texture slots, a preview sphere.

### The sample scene (a floor, a wall and a sun)

The scene that gave the Hazel demo its look is made of a few things, and we
will have them in this order:
1. PBR with image based lighting (stage 13): the floor and the wall stop
   looking like painted plastic, the sky lights them and reflects in them.
2. A tiling floor and a brick wall with a normal map, a roughness map and a
   height map (stage 14).
3. A sun with **shadows** (stage 19, cascades and soft edges): without them
   the scene is flat.
4. HDR sky and the tone mapping (the first we have) and **bloom** (stage 20).
5. The atmosphere for the sunset (stage 21), optional.

What it will look like with 1 and 2 only: a convincing lit floor and wall with
a sky reflecting in the glossy parts and a sun with a hard single-map shadow
(the one we have now); with 3 it starts to look like the demos of other
engines; with 4 it has the glow and the contrast. "Walking in it" is not the
renderer: it needs a character, input and collision, which belong to the
game side (scripting and physics) and are not in this plan; a camera that
walks (no collision) is enough for the demo.

The textures for it (a brick wall, a floor, a sky) are free CC0 ones; they
have to be put into the sample project by hand or with permission to
download them.
