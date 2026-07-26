using ZeroEngine;

public class CrossScriptLookupCaller : ZEScript
{
    public override unsafe void OnUpdate()
    {
        var callee = Entity.FindWithTag("CrossScriptLookupCallee").GetComponent<CrossScriptLookupCallee>();
        int value = callee.Increment();
        EngineAPI.Current->set_velocity(EntityId, value, 0f);
    }
}
