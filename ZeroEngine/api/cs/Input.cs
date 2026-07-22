namespace ZeroEngine;

/// Two-axis virtual input, mirroring Unity's default Horizontal/Vertical axes
/// but selected by an enum rather than a string name.
public enum Axis
{
    Horizontal,
    Vertical,
}

/// How the OS cursor is constrained to the game window.
public enum CursorGrabMode
{
    None = 0,
    Confined = 1,
    Locked = 2,
}

public static unsafe class Input
{
    public static bool IsKeyDown(Key key) => IsKeyPressed((KeyCode)key);

    public static bool IsKeyPressed(Key key) => IsKeyPressed((KeyCode)key);

    public static bool IsKeyPressed(KeyCode key) => EngineAPI.Current->is_key_pressed((int)key);

    public static bool IsKeyJustPressed(Key key) => IsKeyJustPressed((KeyCode)key);

    public static bool IsKeyJustPressed(KeyCode key) => EngineAPI.Current->is_key_just_pressed((int)key);

    public static bool IsKeyUp(Key key) => IsKeyReleased((KeyCode)key);

    public static bool IsKeyReleased(Key key) => IsKeyReleased((KeyCode)key);

    public static bool IsKeyReleased(KeyCode key) => EngineAPI.Current->is_key_released((int)key);

    public static bool IsKeyJustReleased(Key key) => IsKeyJustReleased((KeyCode)key);

    public static bool IsKeyJustReleased(KeyCode key) => EngineAPI.Current->is_key_just_released((int)key);

    public static bool IsMouseButtonPressed(int button) => EngineAPI.Current->is_mouse_button_pressed(button);

    public static bool IsMouseButtonDown(MouseButton button) => IsMouseButtonPressed((int)button);

    public static bool IsMouseButtonPressed(MouseButton button) => IsMouseButtonPressed((int)button);

    public static bool IsMouseButtonJustPressed(int button) => EngineAPI.Current->is_mouse_button_just_pressed(button);

    public static bool IsMouseButtonJustPressed(MouseButton button) => IsMouseButtonJustPressed((int)button);

    public static bool IsMouseButtonReleased(int button) => EngineAPI.Current->is_mouse_button_released(button);

    public static bool IsMouseButtonReleased(MouseButton button) => IsMouseButtonReleased((int)button);

    public static bool IsMouseButtonJustReleased(int button) => EngineAPI.Current->is_mouse_button_just_released(button);

    public static bool IsMouseButtonJustReleased(MouseButton button) => IsMouseButtonJustReleased((int)button);

    public static Vector2 GetMousePosition()
    {
        float x = 0.0f;
        float y = 0.0f;
        EngineAPI.Current->get_mouse_position(&x, &y);
        return new Vector2(x, y);
    }

    public static Vector2 GetMouseDelta()
    {
        float x = 0.0f;
        float y = 0.0f;
        EngineAPI.Current->get_mouse_delta(&x, &y);
        return new Vector2(x, y);
    }

    /// The mouse cursor's position projected into world space (on the z = 0
    /// plane) using the active camera. Falls back to raw screen coordinates when
    /// no camera view is available yet.
    public static Vector2 GetMouseWorldPosition()
    {
        float x = 0.0f;
        float y = 0.0f;
        EngineAPI.Current->get_mouse_world_position(&x, &y);
        return new Vector2(x, y);
    }

    // Smoothed axis state, advanced at most once per frame regardless of how
    // many scripts poll it, so GetAxis stays frame-rate independent and
    // consistent across callers.
    private const float AxisSensitivity = 3.0f;
    private static readonly float[] AxisSmoothed = new float[2];
    private static readonly ulong[] AxisFrame = { ulong.MaxValue, ulong.MaxValue };

    /// Instant, unsmoothed axis value: -1, 0, or +1. Horizontal is A/Left vs
    /// D/Right; Vertical is S/Down vs W/Up (up is positive, Unity-style).
    public static float GetAxisRaw(Axis axis) => axis switch
    {
        Axis.Horizontal => AxisValue(KeyCode.D, KeyCode.ArrowRight, KeyCode.A, KeyCode.ArrowLeft),
        Axis.Vertical => AxisValue(KeyCode.W, KeyCode.ArrowUp, KeyCode.S, KeyCode.ArrowDown),
        _ => 0.0f,
    };

    /// Smoothed axis value that ramps toward the raw target over time, like
    /// Unity's GetAxis.
    public static float GetAxis(Axis axis)
    {
        int index = (int)axis;
        float target = GetAxisRaw(axis);
        ulong frame = Time.FrameCount;

        if (AxisFrame[index] != frame)
        {
            AxisFrame[index] = frame;
            float step = AxisSensitivity * Time.DeltaTime;
            float current = AxisSmoothed[index];

            if (target > current)
            {
                current = MathF.Min(target, current + step);
            }
            else if (target < current)
            {
                current = MathF.Max(target, current - step);
            }

            AxisSmoothed[index] = current;
        }

        return AxisSmoothed[index];
    }

    private static float AxisValue(KeyCode positiveA, KeyCode positiveB, KeyCode negativeA, KeyCode negativeB)
    {
        float value = 0.0f;
        if (IsKeyPressed(positiveA) || IsKeyPressed(positiveB))
        {
            value += 1.0f;
        }

        if (IsKeyPressed(negativeA) || IsKeyPressed(negativeB))
        {
            value -= 1.0f;
        }

        return value;
    }

    private static bool _cursorVisible = true;

    /// Whether the OS cursor is drawn over the game window.
    public static bool CursorVisible
    {
        get => _cursorVisible;
        set
        {
            _cursorVisible = value;
            EngineAPI.Current->set_cursor_visible(value ? 1 : 0);
        }
    }

    /// Locks (or confines/frees) the OS cursor relative to the game window.
    public static void SetCursorGrabMode(CursorGrabMode mode) =>
        EngineAPI.Current->set_cursor_grab_mode((int)mode);
}
