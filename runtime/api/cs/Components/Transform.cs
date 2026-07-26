namespace ZeroEngine;

public sealed unsafe class Transform : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Transform;

    public Vector2 Position
    {
        get
        {
            float x, y = 0.0f;
            EngineAPI.Current->get_position(EntityId, &x, &y);
            return new Vector2(x, y);
        }
        set
        {
            EngineAPI.Current->set_position(EntityId, value.X, value.Y);
        }
    }

    public float Rotation
    {
        get => EngineAPI.Current->get_rotation(EntityId);
        set => EngineAPI.Current->set_rotation(EntityId, value);
    }

    public Vector2 Scale
    {
        get
        {
            float scaleX, scaleY = 0.0f;
            EngineAPI.Current->get_scale(EntityId, &scaleX, &scaleY);
            return new Vector2(scaleX, scaleY);
        }
        set
        {
            EngineAPI.Current->set_scale(EntityId, value.X, value.Y);
        }
    }
}
