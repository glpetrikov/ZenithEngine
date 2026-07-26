using System.Text;

namespace ZeroEngine;

public sealed unsafe class Text : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.UIText;

    public void SetColor(Color color)
    {
        EngineAPI.Current->set_text_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetText(string text)
    {
        var bytes = Encoding.UTF8.GetBytes(text ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_text_text(EntityId, ptr, bytes.Length);
        }
    }

    public void SetFontSize(float fontSize)
    {
        EngineAPI.Current->set_text_font_size(EntityId, fontSize);
    }
}
