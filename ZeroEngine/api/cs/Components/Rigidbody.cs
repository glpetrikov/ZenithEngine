namespace ZeroEngine;

public enum ForceMode
{
    Force,
    Impulse,
}

public sealed unsafe class Rigidbody : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Rigidbody;

    public Vector2 Velocity
    {
        get
        {
            float x = 0.0f;
            float y = 0.0f;
            EngineAPI.Current->get_velocity(EntityId, &x, &y);
            return new Vector2(x, y);
        }
        set => EngineAPI.Current->set_velocity(EntityId, value.X, value.Y);
    }

    public void Add2DForce(float x, float y, ForceMode mode = ForceMode.Impulse)
    {
        switch (mode)
        {
            case ForceMode.Force:
                EngineAPI.Current->add_2d_force(EntityId, x, y);
                break;
            case ForceMode.Impulse:
                EngineAPI.Current->add_2d_impulse(EntityId, x, y);
                break;
            default:
                throw new ArgumentOutOfRangeException(nameof(mode), mode, null);
        }
    }

    public void Add2DForce(Vector2 force, ForceMode mode = ForceMode.Impulse)
    {
        Add2DForce(force.X, force.Y, mode);
    }

    public void Add2DForceWithMax(Vector2 force, Vector2 maxVelocity, ForceMode mode = ForceMode.Impulse)
    {
        Add2DForceWithMax(force.X, force.Y, maxVelocity.X, maxVelocity.Y, mode);
    }

    public void Add2DForceWithMax(float x, float y, float maxX, float maxY, ForceMode mode = ForceMode.Impulse)
    {
        var currentVel = Velocity;
        float requiredX = x;

        if (x > 0.0f && currentVel.X >= maxX) requiredX = 0.0f;
        if (x < 0.0f && currentVel.X <= -maxX) requiredX = 0.0f;

        float requiredY = y;
        if (y > 0.0f && currentVel.Y >= maxY) requiredY = 0.0f;
        if (y < 0.0f && currentVel.Y <= -maxY) requiredY = 0.0f;

        if (requiredX == 0.0f && requiredY == 0.0f)
        {
            return;
        }

        Add2DForce(requiredX, requiredY, mode);
    }
}
