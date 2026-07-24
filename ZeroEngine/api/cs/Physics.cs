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

    public static void SetVelocity(ulong entity, Vector2 velocity) =>
        EngineAPI.Current->set_velocity(entity, velocity.X, velocity.Y);

    // Default range for the no-distance overload. Large enough to cross any
    // sane 2D scene while still finite, so the native side's `max_distance > 0`
    // guard and ray normalization behave predictably.
    public const float DefaultRaycastDistance = 1_000_000.0f;

    public static bool Raycast(Vector2 origin, Vector2 direction, out RaycastHit hit) =>
        Raycast(origin, direction, DefaultRaycastDistance, out hit);

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

    /// Returns the entity under the current mouse world position, or
    /// <c>Entity.Null</c> when nothing is hit.
    public static Entity GetHoveredEntity()
    {
        Vector2 mouseWorld = Input.GetMouseWorldPosition();
        if (Raycast(mouseWorld, new Vector2(0, -1), out RaycastHit hit))
        {
            return new Entity(hit.EntityId);
        }

        return Entity.Null;
    }
}
