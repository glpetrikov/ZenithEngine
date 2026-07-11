# ZeroEngine
ZeroEngine is a modern GameEngine written in Rust with Editor, C# Scripting and ECS.


## github stats
![Status](https://img.shields.io/badge/status-alpha-red?style=flat-square)

[![CI](https://github.com/glpetrikov/ZeroEngine/actions/workflows/ci.yml/badge.svg)](https://github.com/glpetrikov/ZeroEngine/actions/workflows/ci.yml)

![GitHub Repo stars](https://img.shields.io/github/stars/glpetrikov/ZeroEngine?style=social)
![GitHub forks](https://img.shields.io/github/forks/glpetrikov/ZeroEngine?style=social)

![GitHub issues](https://img.shields.io/github/issues/glpetrikov/ZeroEngine?style=flat-square)
![GitHub PRs](https://img.shields.io/github/issues-pr/glpetrikov/ZeroEngine?style=flat-square)

## Getting Started

Requires [Rust](https://rustup.rs?style=flat-square).

On Linux, install `clang`, `mold` and `sccache`, then start the sccache server:
`sccache --start-server` and export Rustc wrapper `export RUSTC_WRAPPER=sccache`


```bash
git clone https://github.com/glpetrikov/ZeroEngine
cd ZeroEngine
cargo run -p ZeroEditor ./Sandbox/ZEProject.toml
```

## Supported Platforms
![Windows 11](https://img.shields.io/badge/Windows%2011-0078D4?style=flat-square&logo=windows11&logoColor=white)
![Linux](https://img.shields.io/badge/Linux-000000?style=flat-square&logo=linux&logoColor=white)
<!-- ![macOS](https://img.shields.io/badge/macOS-silver?style=flat-square&logo=apple&logoColor=black) -->
<!-- ![FreeBSD](https://img.shields.io/badge/FreeBSD-AB2B28?style=flat-square&logo=freebsd&logoColor=white) -->


## License

ZeroEngine is distributed under the **Apache 2.0 License** OR **Blue Oak Model License 1.0.0**.

See [![License Apache2.0](https://img.shields.io/badge/License-Apache_2.0-blue?style=flat-square)](LICENSE.APACHE-2.0) for details or
see [![License BlueOak1.0](https://img.shields.io/badge/License-Blue%20Oak%201.0-blue?style=flat-square)](LICENSE.BLUE-OAK-1.0.0) for details
