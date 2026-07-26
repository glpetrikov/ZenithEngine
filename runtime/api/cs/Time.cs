using System.Runtime.InteropServices;

namespace ZeroEngine;

[StructLayout(LayoutKind.Sequential)]
public struct TimeState
{
    public float delta_time;
    public float fixed_delta_time;
    public float unscaled_delta_time;
    public float time_scale;
    public double time_since_startup;
    public double unscaled_time_since_startup;
    public double fixed_time;
    public ulong frame_count;
    public ulong fixed_frame_count;
    public byte is_fixed_update;
}

public static unsafe class Time
{
    private static TimeState* _state;

    public static float DeltaTime => State->delta_time;

    public static float FixedDeltaTime => State->fixed_delta_time;

    public static float UnscaledDeltaTime => State->unscaled_delta_time;

    public static float TimeScale => State->time_scale;

    public static double TimeSinceStartup => State->time_since_startup;

    public static double UnscaledTimeSinceStartup => State->unscaled_time_since_startup;

    public static double RealtimeSinceStartup => State->unscaled_time_since_startup;

    public static double FixedTime => State->fixed_time;

    public static ulong FrameCount => State->frame_count;

    public static ulong FixedFrameCount => State->fixed_frame_count;

    public static bool IsFixedUpdate => State->is_fixed_update != 0;

    internal static void Initialize(TimeState* statePtr) => _state = statePtr;

    private static TimeState* State
    {
        get
        {
            if (_state == null)
            {
                throw new InvalidOperationException("ZeroEngine Time was used before engine initialization.");
            }

            return _state;
        }
    }
}
