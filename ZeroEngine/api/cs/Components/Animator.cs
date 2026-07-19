using System.Text;

namespace ZeroEngine;

public sealed unsafe class Animator : ZEComponent
{
    internal override ComponentType ComponentType => ComponentType.Animator;

    public void SetState(string name)
    {
        var bytes = Encoding.UTF8.GetBytes(name ?? string.Empty);
        fixed (byte* ptr = bytes)
        {
            EngineAPI.Current->set_animator_state(EntityId, ptr, bytes.Length);
        }
    }
}
