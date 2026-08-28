# ZeroEngine CLI Deployment

All CLI crates in this directory participate in the same resource staging standard as the engine and game crates.
Resource staging is centralized in the `ze_build` crate so CLI tools do not maintain their own build scripts.

The centralized build step:

- detect the active profile from Cargo build-script environment;
- create or verify a clean `dist` directory inside the active profile output directory;
- copy the repository root `NOTICE` file into that staging directory;
- prepare shared runtime resources used by packaged tools and applications;
- leave compiled Rust binaries exactly where Cargo links them.

This behavior is required for packaging because ZeroEngine distributions collect resources from the profile staging directory while executable placement remains a manual developer operation.
If staging drifts into per-tool build scripts, release and dist packages can miss legal notices or runtime resources even though Rust binaries compiled successfully.
