using ZeroEngine;

/// Permanent regression fixture for the fix making Physics.Raycast()/
/// GetHoveredEntity() work from OnUpdate, not just OnFixedUpdate (the native
/// raycast provider used to be registered only for the duration of
/// PhysicsSystem's fixed-update step). Casts a fixed, known ray every regular
/// OnUpdate tick (deliberately not OnFixedUpdate) and encodes the result into
/// this entity's own Transform.Position so a headless Rust test can read it
/// back via the real ECS -- (1, frameCounter) means the ray hit
/// "RaycastTarget" this frame, (0, frameCounter) means it didn't.
public class RaycastFromUpdateProbe : ZEScript
{
    private int _frameCounter;

    public override void OnUpdate()
    {
        _frameCounter++;

        Entity target = Entity.FindWithTag("RaycastTarget");
        bool hitOk = Physics.Raycast(new Vector2(0f, 0f), new Vector2(0f, -1f), out RaycastHit hit)
            && new Entity(hit.EntityId) == target;

        var transform = GetComponent<Transform>();
        transform.Position = new Vector2(hitOk ? 1f : 0f, _frameCounter);
    }
}
