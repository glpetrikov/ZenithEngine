namespace ZeroEngine;

public struct Color
{
    public float R { get; set; }
    public float G { get; set; }
    public float B { get; set; }
    public float A { get; set; }

    public Color(float r, float g, float b, float a)
    {
        R = r;
        G = g;
        B = b;
        A = a;
    }

    public static Color RGB(byte r, byte g, byte b) => RGBA(r, g, b, 255);

    public static Color RGBA(byte r, byte g, byte b, byte a) => new(r / 255f, g / 255f, b / 255f, a / 255f);

    public static Color White => new(1f, 1f, 1f, 1f);
    public static Color Black => new(0f, 0f, 0f, 1f);
    public static Color Gray => new(0.5f, 0.5f, 0.5f, 1f);
    public static Color DarkGray => new(0.2f, 0.2f, 0.2f, 1f);
    public static Color Transparent => new(0f, 0f, 0f, 0f);
    public static Color Red => new(0.8f, 0.1f, 0.1f, 1f);
    public static Color Green => new(0.1f, 0.8f, 0.1f, 1f);
    public static Color Blue => new(0.2f, 0.2f, 0.8f, 1f);
    public static Color LightBlue => new(0.5f, 0.6f, 1f, 1f);
    public static Color DarkBlue => new(0.05f, 0.05f, 0.4f, 1f);
    public static Color Yellow => new(0.9f, 0.85f, 0.1f, 1f);
    public static Color Orange => new(0.9f, 0.5f, 0.1f, 1f);
    public static Color Purple => new(0.5f, 0.15f, 0.7f, 1f);
    public static Color Pink => new(0.95f, 0.5f, 0.7f, 1f);
    public static Color Cyan => new(0.1f, 0.8f, 0.8f, 1f);
    public static Color Magenta => new(0.8f, 0.1f, 0.8f, 1f);
    public static Color VividRed => new(1f, 0f, 0f, 1f);
    public static Color VividGreen => new(0f, 1f, 0f, 1f);
    public static Color VividBlue => new(0f, 0f, 1f, 1f);
}
