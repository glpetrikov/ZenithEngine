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

    public void Add2DForce(float x, float y, ForceMode mode = ForceMode.Force)
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

    public void Add2DForce(Vector2 force, ForceMode mode = ForceMode.Force)
    {
        Add2DForce(force.X, force.Y, mode);
    }

    public void AddRelative2DForce(float x, float y, ForceMode mode = ForceMode.Force)
    {
        float angleDeg = EngineAPI.Current->get_rotation(EntityId);
        float angleRad = angleDeg * (MathF.PI / 180.0f);
        float cos = MathF.Cos(angleRad);
        float sin = MathF.Sin(angleRad);
        float worldX = x * cos - y * sin;
        float worldY = x * sin + y * cos;
        Add2DForce(worldX, worldY, mode);
    }

    public void AddRelative2DForce(Vector2 relativeForce, ForceMode mode = ForceMode.Force)
    {
        AddRelative2DForce(relativeForce.X, relativeForce.Y, mode);
    }

    public void AddTorque(float torque, ForceMode mode = ForceMode.Force)
    {
        switch (mode)
        {
            case ForceMode.Force:
                EngineAPI.Current->add_torque(EntityId, torque);
                break;
            case ForceMode.Impulse:
                EngineAPI.Current->add_torque_impulse(EntityId, torque);
                break;
            default:
                throw new ArgumentOutOfRangeException(nameof(mode), mode, null);
        }
    }

    public void Add2DForceWithMax(Vector2 force, Vector2 maxVelocity, ForceMode mode = ForceMode.Force)
    {
        Add2DForceWithMax(force.X, force.Y, maxVelocity.X, maxVelocity.Y, mode);
    }

    public void Add2DForceWithMax(float x, float y, float maxX, float maxY, ForceMode mode = ForceMode.Force)
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
