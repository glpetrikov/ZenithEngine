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
    pub target: CameraTarget,         // Window | Texture(AssetId) (later: a render texture asset)
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
6. **The first passes**: `zenith.input_camera`, `zenith.input_meshes`,
   `zenith.opaque`, `zenith.tonemap`, `zenith.output` and the built-in graph
   of the five. The passes ask for their shaders by name (`Zenith/standard`)
   through `ShaderSources`, which gives the engine's copies until stage 10.
   Check: a headless render into a texture, and the runtime draws the world
   with one camera.
7. **Cameras and targets.** `order`, `viewport`, `clear`, `RenderTargets`,
   `Camera.graph`. Check: two cameras on one window, a third into a texture.
8. **The rest of the 3D passes**, carrying over what the old renderer
   does: `zenith.input_lights`, `zenith.input_environment`, `zenith.sky`,
   lighting, normal mapping, environment reflections, `zenith.transparent`,
   `zenith.shadow`.
9. **2D**: `zenith.input_sprites`, `zenith.sprites` and the orthographic
   camera.
10. **Shaders in the project** (section 19): `Shaders://`, the engine's
    shaders written into new projects and into old ones on open, the version
    line and "the engine has a newer one", loading from the project, WESL
    compiled with `wesl` (imports through the project, conditions in the
    pipeline key), checking with `naga`, the stand-ins of a missing or broken
    shader, custom shaders named by materials. The engine's shaders written
    before this stage move to `.wesl` here. Check: a project whose
    `Zenith/standard.wesl` was changed draws with the change; a broken one
    draws with the engine's copy and a warning.
11. **The editor: viewport.** The texture in ImGui, the editor camera, hot
    reload (of shaders too), replacing a source, the editor passes
    (`editor.grid`, `editor.gizmos`, `editor.outline`).
12. **The graph editor** with imnodes.
13. **Scriptable render passes** (section 20), after the scripting, made by Gleb, can call
    into the engine. Check: a fullscreen effect written as a script and a
    WESL file in the project, put into the graph in the graph editor.
14. **Later.** The asset index in a build, asynchronous loading, frustum
    culling, MSAA, the parameters of custom shaders read by reflection
    (section 19), removing the old `zenith_renderer`.

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

**[decided]** Later, after the scripting is done (stage 13 of section 16; Gleb writes the scripting and this part, not the renderer work): render passes written
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
