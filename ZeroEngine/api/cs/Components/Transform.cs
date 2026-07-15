namespace ZeroEngine;

public sealed unsafe class Transform : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Transform;

    public Vector2 Position
    {
        get
        {
            float x = 0.0f;
            float y = 0.0f;
            EngineAPI.Current->get_position(EntityId, &x, &y);
            return new Vector2(x, y);
        }
    }
}
