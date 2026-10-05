# Contributing to Zenith Engine

Thanks for wanting to help. Zenith Engine is a game engine written in Rust, maintained by Gleb Petrikov. Please read this before opening a pull request.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md). For security problems, see [SECURITY.md](SECURITY.md) and do not open a public issue.

## Ways to contribute

- Report bugs
- Suggest features and discuss design
- Improve documentation
- Fix bugs and implement features

## Before you start

Small changes (typos, obvious bug fixes, documentation) can go straight to a pull request.

For bigger changes, such as a new feature or system, a change to a public API, or a large refactor, please open an issue first (or comment on an existing one) and describe what you want to do. This does not replace the pull request: it is a short conversation before it, so you do not spend days on something that does not fit the direction of the engine. Once the approach is agreed, send the PR and link the issue.

The engine is in alpha and the API is unstable, so things can change quickly.

Check the roadmap in the README and the existing issues to avoid duplicating work.

## Reporting bugs

Open an issue using the bug report template and fill in every section:

- **Description**: what is wrong, clearly and concisely
- **Steps to reproduce**: ideally a minimal example
- **Expected behavior** and **actual behavior**
- **Environment**: Zenith Engine version or commit, OS, GPU, Rust version, WGPU backend
- **Additional context**: logs, screenshots, videos, crash reports, code snippets

When you open a pull request, fill in the pull request template as well.

## Development setup

You need a recent stable Rust toolchain (install with [`rustup`](https://rustup.rs)) and a GPU and driver supported by wgpu (Vulkan, DX12, OpenGL, or WebGPU in the browser). The supported platforms are listed in the README.

Run the editor:

```sh
git clone https://github.com/glpetrikov/ZenithEngine.git
cd ZenithEngine
cargo run
```

Run the native runtime:

```sh
cargo run -p zenith_runtime
```

The WASM runtime needs Node.js and npm as well. See the README for the full steps.

Before submitting, make sure all of these pass:

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

The repository also has a check script in the `scripts/` directory that runs the same checks.

Always read a script before you run it. This applies to every script in this repository and to anything else you download: never run code you have not looked at first.

## Pull requests

- Keep each PR focused on one change. Unrelated cleanups belong in a separate PR.
- Sign off every commit with `git commit -s` (see the [DCO](#developer-certificate-of-origin) section below).
- Write a clear description: what changed and why.
- Link the issue it addresses, if there is one.
- Add or update tests and documentation when it makes sense.
- Make sure CI passes.
- Avoid new `unsafe` code unless needed. If you add it, explain why it is sound in a `// SAFETY:` comment.
- Be careful with new dependencies. Explain why one is needed and check that its license is compatible (see below). Fewer dependencies is better.
- Do not commit generated files, build artifacts, or large binary assets without discussing it first.

If you used an AI tool to write part of your change, you are still responsible for it: you must understand it, have tested it, and be sure that you are allowed to submit it.

## Commit messages

A good commit message looks like this:

```
Fix swapchain resize crash on minimize

The swapchain was reconfigured with a zero-sized surface when the
window was minimized, which panics in wgpu. Skip the resize while
either dimension is zero and reconfigure on restore.

Fixes #123
Signed-off-by: Your Name <your.email@example.com>
```

Guidelines:

- Write the subject line in the imperative mood, as if completing the sentence "This commit will ...": "Fix crash", not "Fixed crash" or "Fixes crash".
- Start with a capital letter and a verb, and do not end with a period.
- Keep the subject short, around 50 characters, 72 at most.
- Separate the subject from the body with a blank line. The body is optional for trivial changes, but for anything else explain what changed and why (the diff already shows how). Wrap lines at about 72 characters.
- Reference issues at the bottom with `Fixes #123` or `Refs #123`.
- One logical change per commit. Do not mix a refactor with a bug fix.
- A short area prefix is fine when it helps: `renderer: Fix swapchain resize crash`.

The usual starting verbs:

- `Add`: a new feature, file, test, or dependency
- `Fix`: a bug fix
- `Refactor`: restructure code without changing behavior
- `Improve`: make something better without a clear bug or new feature (performance, errors, ergonomics)
- `Update`: change existing behavior, content, or a dependency version
- `Remove`: delete code, features, or files
- `Rename`, `Move`: change names or locations only
- `Document`: documentation and comments only
- `Test`: add or change tests only
- `Bump`: raise a version number
- `Revert`: undo a previous commit, with its hash in the body

## Style

- Format with `rustfmt`, lint with `clippy`.
- Prefer clear, readable code over clever code.
- Public items should have doc comments.
- Emoji are not forbidden, but please keep them out of code, comments, commit messages, and documentation unless there is a good reason.

## Licensing and copyright

Zenith Engine is licensed under either of

- [Apache License, Version 2.0](LICENSE.APACHE-2.0)
- [Blue Oak Model License 1.0.0](LICENSE.BLUE-OAK-1.0.0)

at your option.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in the project is licensed under the same terms (Apache-2.0 OR BlueOak-1.0.0), without any additional terms or conditions.

You keep the copyright on your contribution. The project's copyright notice reads "Copyright Gleb Petrikov and Zenith Engine Contributors", and you are one of those contributors.

There is no CLA to sign. Instead, we use the Developer Certificate of Origin (see below).

### Developer Certificate of Origin

Every commit must be signed off, which certifies that you wrote the contribution or have the right to submit it under the project's license, and that you agree it will be distributed under those terms. The full text is in [DCO.md](DCO.md).

Add the sign-off with the `-s` flag:

```sh
git commit -s -m "Fix swapchain resize crash"
```

This appends a line to the commit message:

```
Signed-off-by: Your Name <your.email@example.com>
```

Use your real name and an email address you control. If you forgot, fix the last commit with `git commit --amend -s --no-edit`, or for several commits use `git rebase --signoff <base>`. Pull requests with unsigned commits cannot be merged.

### Third-party code and assets

Do not submit code copied from projects with incompatible licenses (for example GPL or AGPL), and do not submit code you do not have the right to share.

If your change includes code or assets from another project (copied or adapted code, fonts, textures, models, and so on):

1. Say so in the PR and mention where it comes from and under which license.
2. Add its license and copyright notice to `about.hbs`. Do not edit `NOTICE` by hand.
3. Regenerate `NOTICE` by running the `gen_notice` script in `scripts/` (read it before you run it).

Crates from Cargo dependencies are picked up automatically when `NOTICE` is regenerated.

## Contributors

Everyone who has contributed to Zenith Engine is a contributor, and holds the copyright on their own contributions under the license above. The full list is in the git history. Notable contributors may also be listed here.

- Gleb Petrikov (maintainer)

## Questions

Open a discussion or an issue. Please be patient: this is a solo-maintained project.
