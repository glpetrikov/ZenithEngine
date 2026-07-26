namespace ZeroEngine;

public sealed unsafe class Sprite : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Sprite;

    public float TextureRotationDegrees
    {
        get => EngineAPI.Current->get_sprite_texture_rotation_degrees(EntityId);
        set => EngineAPI.Current->set_sprite_texture_rotation_degrees(EntityId, value);
    }
}
