using System.Text;

namespace ZeroEngine;

public sealed unsafe class Image : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.UIImage;

    public void SetColor(Color color)
    {
        EngineAPI.Current->set_image_color(EntityId, color.R, color.G, color.B, color.A);
    }

    public void SetTexturePath(string path)
    {
        var bytes = Encoding.UTF8.GetBytes(path ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_image_texture_path(EntityId, ptr, bytes.Length);
        }
    }

    public void SetSheetCell(string sheetPath, uint cellId)
    {
        var bytes = Encoding.UTF8.GetBytes(sheetPath ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_image_sheet_cell(EntityId, ptr, bytes.Length, cellId);
        }
    }
}
