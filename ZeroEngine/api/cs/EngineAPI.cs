using System.IO;
using System.Text;

namespace ZeroEngine;

public unsafe struct EngineAPI
{
    public delegate* unmanaged[Cdecl]<int, bool> is_key_pressed;
    public delegate* unmanaged[Cdecl]<int, bool> is_key_just_pressed;
    public delegate* unmanaged[Cdecl]<int, bool> is_key_released;
    public delegate* unmanaged[Cdecl]<int, bool> is_key_just_released;
    public delegate* unmanaged[Cdecl]<int, bool> is_mouse_button_pressed;
    public delegate* unmanaged[Cdecl]<int, bool> is_mouse_button_just_pressed;
    public delegate* unmanaged[Cdecl]<int, bool> is_mouse_button_released;
    public delegate* unmanaged[Cdecl]<int, bool> is_mouse_button_just_released;
    public delegate* unmanaged[Cdecl]<float*, float*, void> get_mouse_position;
    public delegate* unmanaged[Cdecl]<float*, float*, void> get_mouse_delta;
    public delegate* unmanaged[Cdecl]<TimeState*> get_time_state_ptr;
    public delegate* unmanaged[Cdecl]<ulong, uint, bool> has_component;
    public delegate* unmanaged[Cdecl]<ulong, float*, float*, void> get_position;
    public delegate* unmanaged[Cdecl]<ulong, float*, float*, void> get_velocity;
    public delegate* unmanaged[Cdecl]<ulong, float, float, void> add_2d_force;
    public delegate* unmanaged[Cdecl]<ulong, float, float, void> add_2d_impulse;
    public delegate* unmanaged[Cdecl]<ulong, void> play_audio;
    public delegate* unmanaged[Cdecl]<ulong, void> stop_audio;
    public delegate* unmanaged[Cdecl]<ulong, float, void> set_audio_volume;
    public delegate* unmanaged[Cdecl]<ulong, bool> is_audio_playing;
    public delegate* unmanaged[Cdecl]<float, float, float, float, float, float*, float*, float*, float*, ulong*, bool> raycast_2d;
    public delegate* unmanaged[Cdecl]<ulong, float> get_sprite_texture_rotation_degrees;
    public delegate* unmanaged[Cdecl]<ulong, float, void> set_sprite_texture_rotation_degrees;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_button_color;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_button_hover_color;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_bar_color;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_bar_bg_color;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_text_color;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, void> set_bar_label;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_button_pressed_color;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, void> set_button_text;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, void> set_text_text;
    public delegate* unmanaged[Cdecl]<ulong, float, void> set_text_font_size;
    public delegate* unmanaged[Cdecl]<ulong, bool> is_button_clicked;
    public delegate* unmanaged[Cdecl]<ulong, float, float, float, float, void> set_image_color;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, void> set_image_texture_path;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, uint, void> set_image_sheet_cell;
    public delegate* unmanaged[Cdecl]<byte*, int, void> scene_load;
    public delegate* unmanaged[Cdecl]<void> scene_load_main;
    public delegate* unmanaged[Cdecl]<void> scene_reload;
    public delegate* unmanaged[Cdecl]<byte*, int, void> log_info;
    public delegate* unmanaged[Cdecl]<byte*, int, void> log_warn;
    public delegate* unmanaged[Cdecl]<byte*, int, void> log_error;
    public delegate* unmanaged[Cdecl]<byte*, int, void> log_debug;
    public delegate* unmanaged[Cdecl]<void> quit_game;
    public delegate* unmanaged[Cdecl]<ulong, byte*, int, void> set_animator_state;

    private static EngineAPI* current;
    private static bool consoleRedirected;

    internal static void Initialize(EngineAPI* api)
    {
        current = api;
        Time.Initialize(api->get_time_state_ptr());
        RedirectConsole();
    }

    internal static bool HasComponent(ulong entity, ComponentType componentType)
    {
        return Current->has_component(entity, (uint)componentType);
    }

    internal static EngineAPI* Current
    {
        get
        {
            if (current == null)
            {
                throw new InvalidOperationException("ZeroEngine API was used before OnEngineInit.");
            }

            return current;
        }
    }

    internal static EngineAPI* CurrentOrNull => current;

    private static void RedirectConsole()
    {
        if (consoleRedirected)
        {
            return;
        }

        Console.SetOut(new EngineConsoleWriter(Log.Info));
        Console.SetError(new EngineConsoleWriter(Log.Error));
        consoleRedirected = true;
    }

    private sealed class EngineConsoleWriter(Action<string> writeLine) : TextWriter
    {
        private readonly StringBuilder buffer = new();

        public override Encoding Encoding => Encoding.UTF8;

        public override void Write(char value)
        {
            if (value == '\n')
            {
                FlushLine();
                return;
            }

            if (value != '\r')
            {
                buffer.Append(value);
            }
        }

        public override void Write(string? value)
        {
            if (string.IsNullOrEmpty(value))
            {
                return;
            }

            foreach (var character in value)
            {
                Write(character);
            }
        }

        public override void WriteLine(string? value)
        {
            Write(value);
            FlushLine();
        }

        public override void Flush()
        {
            FlushLine();
        }

        private void FlushLine()
        {
            if (buffer.Length == 0)
            {
                return;
            }

            var message = buffer.ToString();
            buffer.Clear();
            writeLine(message);
        }
    }
}
