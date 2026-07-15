namespace ZeroEngine;

public struct RaycastHit
{
    public Vector2 Point;
    public Vector2 Normal;
    public ulong EntityId;
}

public static unsafe class Physics
{
    public static Vector2 GetVelocity(ulong entity)
    {
        float x = 0.0f;
        float y = 0.0f;
        EngineAPI.Current->get_velocity(entity, &x, &y);
        return new Vector2(x, y);
    }

    public static void Add2DForce(ulong entity, float x, float y) => EngineAPI.Current->add_2d_force(entity, x, y);

    public static void Add2DImpulse(ulong entity, float x, float y) => EngineAPI.Current->add_2d_impulse(entity, x, y);

    public static bool Raycast(Vector2 origin, Vector2 direction, float maxDistance, out RaycastHit hit)
    {
        float pointX = 0.0f, pointY = 0.0f;
        float normalX = 0.0f, normalY = 0.0f;
        ulong entityId = 0;

        bool didHit = EngineAPI.Current->raycast_2d(
            origin.X, origin.Y, direction.X, direction.Y, maxDistance,
            &pointX, &pointY, &normalX, &normalY, &entityId);

        hit = new RaycastHit
        {
            Point = new Vector2(pointX, pointY),
            Normal = new Vector2(normalX, normalY),
            EntityId = entityId,
        };

        return didHit;
    }
}
