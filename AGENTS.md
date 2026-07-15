# AGENTS.md

## Project

ZeroEngine is an experimental Rust game engine.

The engine is currently in early development. Prefer small, focused changes over large rewrites.

Main goals:

- Rust-native engine core
- wgpu renderer
- Shipyard ECS
- C# scripting API
- egui-based editor
- scene serialization
- future physics and asset pipeline support

## Repository expectations

Before making changes, inspect the existing code and follow the current structure. Only change architecture when the task explicitly requires it.

Do not redesign major systems unless the task explicitly asks for it.

Prefer preserving existing public APIs and file formats unless the task requires a breaking change.

Avoid introducing unnecessary dependencies.

Do not use `unsafe` unless the task explicitly requires it and the reason is explained.

Do not introduce global mutable state.

If you add debug logs, use ze_log::debug! or ze_log::trace! but not ze_log::info!/warn!/error!

## Rust style

Use idiomatic Rust.

Prefer:

- explicit error handling
- small modules
- clear trait bounds
- stable string identifiers for engine data formats
- compile-time validation where possible

Avoid:

- hidden runtime assumptions
- stringly-typed logic when a typed API is reasonable
- large unrelated refactors
- changing scene format without a clear reason

## Formatting

This project uses nightly rustfmt.

Run:

```bash
cargo +nightly fmt --all
```

Do not use stable rustfmt if it changes formatting differently from nightly.

## Checks

There is a project check script:

```bash
python scripts/check.py
```

Run it before considering the task complete.

If you need to run checks manually, use:

```bash
cargo +nightly fmt --all -- --check
cargo check --workspace --all-targets --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

If a check fails, fix the issue instead of ignoring it.

If a check cannot be run in the current environment, explain exactly why.

Do not mention successful checks just to say that they passed. Only mention checks when:

- a check failed;
- a check could not be run;
- the user explicitly asks for check results;
- the task specifically requires reporting executed checks.

If there are errors, warnings, failed tests, formatting issues, or clippy issues, report them clearly and preferably fix them before finishing the task.

## Commit quality

Keep changes focused.

A good change should:

- solve the requested task
- avoid unrelated formatting churn
- avoid unrelated refactors
- keep existing behavior unless explicitly changed
- compile successfully

## ECS and scene serialization

The engine uses Shipyard ECS.

Components are registered through the component registry.

Scene serialization uses stable component type ids such as:

```text
ze.name
ze.tag
ze.transform
ze.inactive
```

Do not replace stable component ids with Rust type names.

Scene format compatibility matters. Do not redesign the scene format unless explicitly requested.

## Reflection

Registered editor-visible or serializable components should implement the required reflection traits.

Reflection should support future editor inspector UI, but should not replace the stable scene serialization format unless explicitly requested.

## C# scripting

The C# scripting API is part of the engine design.

Do not invent large scripting API changes without matching the existing direction.

Prefer simple, explicit bindings.

Native interop boundaries must be reviewed carefully.

Do not hide failures. Invalid script methods, invalid signatures, missing attributes, or load errors should become clear engine errors.

## Renderer

The renderer uses wgpu.

Avoid changing renderer architecture unless the task is specifically about renderer design.

Do not add rendering features by bypassing existing abstractions.

Keep backend code explicit and debuggable.

## Editor

The editor uses egui.

Editor UI code should be practical and simple.

Prefer immediate-mode UI patterns that fit egui instead of trying to force retained-mode architecture.

## Dependencies

Do not add dependencies casually.

Before adding a dependency, check whether the project already has an equivalent utility.

If adding a dependency is necessary, keep it scoped to the crate that needs it.

## Generated code

Generated code must be readable and maintainable.

Do not leave placeholder code unless the task explicitly asks for scaffolding.

Do not add comments that merely repeat what the code says.

## Final response expectations

When reporting completed work, summarize:

- what changed
- which files were changed
- errors, warnings, failed checks, or checks that could not be run

Do not list successful checks unless the user asked for them or the task requires it.
