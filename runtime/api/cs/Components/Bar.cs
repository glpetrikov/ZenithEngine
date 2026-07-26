using System.Text;

namespace ZeroEngine;

public sealed unsafe class Bar : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.UIBar;

    public void SetColor(Color color)
    {
        EngineAPI.Current->set_bar_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetBgColor(Color color)
    {
        EngineAPI.Current->set_bar_bg_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetLabel(string text)
    {
        var bytes = Encoding.UTF8.GetBytes(text ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_bar_label(EntityId, ptr, bytes.Length);
        }
    }
}
