using System;
using System.Text;

namespace ZeroEngine;

public sealed unsafe class Button : ZEComponent
{
    private Action? onClick;

    internal override ComponentType ComponentType => ComponentType.UIButton;

    public void SetColor(Color color)
    {
        EngineAPI.Current->set_button_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetHoverColor(Color color)
    {
        EngineAPI.Current->set_button_hover_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetPressedColor(Color color)
    {
        EngineAPI.Current->set_button_pressed_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetText(string text)
    {
        var bytes = Encoding.UTF8.GetBytes(text ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_button_text(EntityId, ptr, bytes.Length);
        }
    }

    public bool IsClicked() => EngineAPI.Current->is_button_clicked(EntityId);

    public void SetOnClick(Action callback)
    {
        onClick = callback;
    }

    // Edge-triggered: UISystem writes UIButton.pressed from yakui's click
    // event, which fires exactly once for the frame a click completes, so a
    // held-down mouse button does not repeatedly invoke the callback.
    internal override void PollEvents()
    {
        if (IsClicked())
        {
            onClick?.Invoke();
        }
    }
}
