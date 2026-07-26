namespace ZeroEngine;

public sealed class Name : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Name;

    public string Value => EngineAPI.GetName(EntityId);
}
