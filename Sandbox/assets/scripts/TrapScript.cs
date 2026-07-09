using System;
using ZeroEngine;

namespace Sandbox;

public class TrapScript : ZEScript
{
    private const uint CircleEntityIndex = 4;

    public override void OnContactEnter(ulong otherEntity)
    {
        if (EntityIndexOf(otherEntity) == CircleEntityIndex)
        {
            Log.Info("Circle trapped!");
        }
    }

    public override void OnContactExit(ulong otherEntity)
    {
        if (EntityIndexOf(otherEntity) == CircleEntityIndex)
        {
            Log.Info("Circle escaped!");
        }
    }

    private static uint EntityIndexOf(ulong entityId) => (uint)(entityId & 0xFFFFFFFF);
}
