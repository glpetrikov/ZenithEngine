namespace ZeroEngine;

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
}
