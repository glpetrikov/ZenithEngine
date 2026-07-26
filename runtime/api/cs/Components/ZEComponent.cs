namespace ZeroEngine;

public abstract class ZEComponent
{
    public ulong EntityId { get; private set; }

    public uint EntityIndex => (uint)(EntityId & 0xFFFFFFFF);

    public uint EntityGeneration => (uint)(EntityId >> 32);

    internal abstract ComponentType ComponentType { get; }

    internal void Bind(ulong entityId)
    {
        EntityId = entityId;
    }

    // Polled once per frame by ZEScript, before the owning script's OnUpdate(),
    // so components can react to native state changes (e.g. a button click)
    // even if the script never polls them directly.
    internal virtual void PollEvents() { }
}
