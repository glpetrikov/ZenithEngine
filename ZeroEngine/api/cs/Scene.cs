using System.Text;

namespace ZeroEngine;

public static unsafe class Scene
{
    public static void Load(string name)
    {
        if (string.IsNullOrWhiteSpace(name))
        {
            throw new ArgumentException("Scene name cannot be empty.", nameof(name));
        }

        var bytes = Encoding.UTF8.GetBytes(name.Trim());
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->scene_load(ptr, bytes.Length);
        }
    }

    public static void LoadMain() => EngineAPI.Current->scene_load_main();

    public static void Reload() => EngineAPI.Current->scene_reload();
}
