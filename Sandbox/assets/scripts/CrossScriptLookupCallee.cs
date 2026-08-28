using ZeroEngine;

public class CrossScriptLookupCallee : ZEScript
{
    public int Counter;

    public int Increment() => ++Counter;
}
