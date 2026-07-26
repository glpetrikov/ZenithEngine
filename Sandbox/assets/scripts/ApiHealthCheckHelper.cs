using ZeroEngine;

/// Minimal companion script used only so ApiHealthCheckScript can exercise
/// TryGetComponent<T>() against a ZEScript subtype (not just a built-in
/// ZEComponent).
public class ApiHealthCheckHelper : ZEScript
{
    public int Marker = 42;
}
