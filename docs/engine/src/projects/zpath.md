# ZPath and schemes

A `ZPath` names a place in a project: `scheme://path`, for example
`Assets://textures/ferris.png`. It is only a value. It reads nothing; the
`Project` that receives it turns it into a real location, and
`zenith_filesystem` is the only place where that happens.

```rust,ignore
let p = ZPath::new("Assets/textures/ferris.png")?;   // or "Assets://textures/ferris.png"
p.scheme();     // "Assets"
p.path();       // "textures/ferris.png"
p.file_name();  // Some("ferris.png")
p.parent();     // Some(Assets://textures)
p.join("x")?;   // Assets://textures/ferris.png/x
```

`ZPath::new` takes one string. The scheme is whatever stands before the first
`/` (or before `://`), so `Worlds/world.zenith` and `Worlds://world.zenith` are
the same value, printed with `://`. `ZPath::parse` does the same, and
`ZPath::from_parts("Worlds", "world.zenith")` takes the two halves separately.

## What is rejected

The parser is an early check with a clear error (`InvalidZPath` /
`InvalidScheme`). It rejects:

* `..` and `.` segments, absolute paths, doubled or trailing slashes (empty
  segments);
* backslashes, `:` and control characters in a path (so a path can never turn
  into a drive-relative or absolute path on Windows);
* a scheme that is not made of a letter followed by letters, digits and `_ - + .`.

The empty path is valid and means the root of the scheme (`Assets://`).

The parser is not the only line of defence: the `Fs` backend opens the project
root once as a `cap_std` directory handle and only resolves paths inside it
(see [Working with a project](./using.md#symlinks-and-the-project-boundary)).

The scheme can be written in any case style: `assets`, `Assets` and `ASSETS` are
the same scheme, and so are `MyTextures`, `myTextures`, `my_textures` and
`my-textures`. It is stored, compared and printed in its canonical form,
PascalCase (`Assets://x`, `MyTextures://x`; made with `convert_case`; digits do not
start a word, so `tex2d` is `Tex2d`). A scheme with single-letter words, such as
`a-b`, is refused: it would print as `AB`, which reads back as one word.
The path after `://` is a file path and keeps its case.

Examples (the left column is what you may write, the right one is what the value
is and what `to_string()` prints):

| Written                               | Scheme       | Printed                        |
| ------------------------------------- | ------------ | ------------------------------ |
| `assets://Textures/Ferris.png`        | `Assets`     | `Assets://Textures/Ferris.png` |
| `Assets://Textures/Ferris.png`        | `Assets`     | `Assets://Textures/Ferris.png` |
| `ASSETS://Textures/Ferris.png`        | `Assets`     | `Assets://Textures/Ferris.png` |
| `myTextures://a.png` (camelCase)      | `MyTextures` | `MyTextures://a.png`           |
| `MyTextures://a.png` (PascalCase)     | `MyTextures` | `MyTextures://a.png`           |
| `my_textures://a.png` (snake_case)    | `MyTextures` | `MyTextures://a.png`           |
| `my-textures://a.png` (kebab-case)    | `MyTextures` | `MyTextures://a.png`           |
| `MY_TEXTURES://a.png` (CONSTANT_CASE) | `MyTextures` | `MyTextures://a.png`           |
| `Tex2D://a.png` / `tex2d://a.png`     | `Tex2d`      | `Tex2d://a.png`                |

```rust,ignore
let a = ZPath::parse("my_textures://Ferris.png")?;
let b = ZPath::parse("MyTextures://Ferris.png")?;
assert_eq!(a, b);                                  // the same value
assert_eq!(a.to_string(), "MyTextures://Ferris.png");
assert_eq!(a.scheme(), "MyTextures");

// The path keeps its case: it is a file name.
assert_ne!(ZPath::parse("Assets://ferris.png")?, ZPath::parse("Assets://Ferris.png")?);

// Registering and using a scheme: any spelling finds it.
project.register_scheme("my_textures", "Assets/Textures")?;
project.read_bytes(&ZPath::parse("MY-TEXTURES://Ferris.png")?)?;
```

Refused: `1abc://x` and `_a://x` (not starting with a letter), `my tex://x` (a
space), `é://x` (not ASCII), and `a-b://x` (single-letter words, see above).

## Schemes

A scheme maps a name to a folder inside the project. Built in:

| Scheme     | Folder          | Notes                                                  |
| ---------- | --------------- | ------------------------------------------------------ |
| `assets`   | `Assets`        |                                                        |
| `worlds`   | `Assets/Worlds` | `Worlds://A.zenith` is `Assets://Worlds/A.zenith`      |
| `settings` | `Settings`      |                                                        |
| `packages` | `Packages`      |                                                        |
| `temp`     | `Temp`          |                                                        |
| `trash`    | `Trash`         | read-only, see [the trash](./trash-gitignore-watch.md) |

An unknown scheme is `SchemeNotRegistered`. Add your own for the lifetime of
one `Project` handle:

```rust,ignore
project.register_scheme("textures", "Assets/Textures")?;
project.write(&ZPath::parse("textures://ferris.png")?, &png)?;
```

The root must be a relative folder path inside the project (not empty, no `..`,
and not `.v1`, `.gitignore`, `.git` or the manifest, which a scheme could
otherwise overwrite or, in `.git/hooks`, turn into a program that runs);
a name already in use is `SchemeAlreadyRegistered`. Custom schemes are not
saved: register them again after opening. Anything that resolves to a path
below `Trash/` is read-only, whichever scheme names it.

When a path has to be turned back into a `ZPath` (watcher events, trash
entries) the scheme with the **longest matching root** wins, so
`Assets/Worlds/A.zenith` becomes `Worlds://A.zenith`, not `Assets://Worlds/A.zenith`.

Builds resolve `ZPath`s the same way: paths inside a build are relative to the
project root and use `/`.
