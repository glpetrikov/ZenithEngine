namespace ZeroEngine;

public sealed class Tag : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Tag;

    public string Value => EngineAPI.GetTag(EntityId);
}
