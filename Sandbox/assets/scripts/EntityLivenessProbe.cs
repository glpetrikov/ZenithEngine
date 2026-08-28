using ZeroEngine;

/// Temporary verification fixture for the TryGetComponent/IsValid liveness
/// fix: reports pass/fail as bit flags via set_velocity (mirrors the
/// CrossScriptLookupCaller convention for getting a result back out of a
/// headless test), and logs details for anything that fails.
public class EntityLivenessProbe : ZEScript
{
    private bool _ran;

    public override unsafe void OnUpdate()
    {
        // Runs the checks exactly once: the "victim" entity only exists for
        // this one check, and re-running every frame in a live Standalone
        // session would just spam the log.
        if (_ran)
        {
            return;
        }
        _ran = true;

        int result = 0;

        if (!default(Entity).TryGetComponent<Transform>(out _))
        {
            result |= 1;
        }
        else
        {
            Log.Error("BUG: default(Entity).TryGetComponent<Transform>() returned true.");
        }

        if (!default(Entity).IsValid)
        {
            result |= 2;
        }
        else
        {
            Log.Error("BUG: default(Entity).IsValid returned true.");
        }

        if (!Entity.Null.IsValid)
        {
            result |= 4;
        }
        else
        {
            Log.Error("BUG: Entity.Null.IsValid returned true.");
        }

        // Must be an entity that already existed *before* this tick (so it was
        // picked up by the engine's per-tick component cache) -- destroying
        // and re-checking an entity created this same tick would never
        // reproduce the staleness bug, since it was never cached to begin
        // with.
        Entity victim = Entity.FindWithTag("EntityLivenessProbeVictim");
        victim.Destroy();
        if (!victim.TryGetComponent<Transform>(out var destroyedTransform))
        {
            result |= 8;
        }
        else
        {
            Log.Error($"BUG: TryGetComponent<Transform> on a just-destroyed entity (same tick) returned true, position=({destroyedTransform.Position.X}, {destroyedTransform.Position.Y}).");
        }

        Log.Info($"EntityLivenessProbe result={result} (expect 15)");
        EngineAPI.Current->set_velocity(EntityId, result, 0f);
    }
}
