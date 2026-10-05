# Security Policy

## Supported versions

Zenith Engine is under active development and is currently in alpha. Only the latest release and the current `develop` branch receive security fixes. Older versions are not patched.

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report privately using GitHub's private vulnerability reporting: open the [Security tab](https://github.com/glpetrikov/ZenithEngine/security) of the repository and click "Report a vulnerability".

Include as much of the following as you can:

- A description of the problem and its impact
- Steps to reproduce, or a minimal proof of concept
- The affected version or commit, and your OS and GPU/driver if relevant
- Whether you plan to disclose it publicly, and when

## What to expect

- Acknowledgement within 7 days.
- An initial assessment within 14 days.
- Updates as the fix progresses. This is a solo-maintained project, so timelines can slip, but you will not be ignored.
- Credit in the release notes when the fix ships, unless you prefer to stay anonymous.

We ask for coordinated disclosure: please give us a reasonable window (90 days by default) to ship a fix before publishing details.

## Scope

In scope:

- The engine runtime, editor, and build tooling in this repository
- Packages and the package loader: how packages are fetched, verified, unpacked, and installed, including path traversal and tampering
- Loading and parsing of project files, assets, packages, native plugins, and `.zenithbuild` archives, especially from untrusted sources: memory-safety issues, unsafe parsing, and crashes on malformed input
- Networking code, including malformed-packet handling

Out of scope:

- Vulnerabilities in third-party dependencies that have no impact on Zenith Engine (report those upstream)
- Issues that require an attacker to already have full control of the machine
- Native plugins and packages are code that runs with the permissions of the engine. Loading a malicious plugin that you installed yourself is not an engine vulnerability, but flaws in how the engine verifies or isolates them are in scope
- Bugs in games built with the engine, unless the root cause is in the engine

## Safe harbor

If you act in good faith, avoid harming other users, and do not access or modify data that is not yours, we will not pursue action against you for your research.
