# ze_scripting_cs

C# scripting backend for ZeroEngine.

This crate hosts the .NET runtime with `netcorehost`, loads `assets/scripts/bin/Scripts.dll`,
and calls optional `[UnmanagedCallersOnly]` lifecycle methods on `Scripts.Script`.
