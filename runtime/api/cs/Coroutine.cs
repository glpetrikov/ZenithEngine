using System.Collections;

namespace ZeroEngine;

/// Base type for the objects a coroutine can `yield return` to suspend itself.
/// Yielding `null` resumes on the next update tick.
public abstract class YieldInstruction
{
}

/// Suspends a coroutine for `seconds` of scaled game time (respects
/// `Time.TimeScale`), like Unity's `WaitForSeconds`.
public sealed class WaitForSeconds : YieldInstruction
{
    internal float Remaining;

    public WaitForSeconds(float seconds)
    {
        Remaining = seconds;
    }
}

/// Suspends a coroutine until the next fixed-update tick.
public sealed class WaitForFixedUpdate : YieldInstruction
{
}

/// Suspends a coroutine until the next update tick (functionally the same as
/// yielding `null`; provided for Unity familiarity).
public sealed class WaitForEndOfFrame : YieldInstruction
{
}

/// Handle to a running coroutine, returned by `StartCoroutine`. Pass it back to
/// `StopCoroutine`, or `yield return` it to wait for the coroutine to finish.
public sealed class Coroutine
{
    internal CoroutineRunner Runner { get; }

    internal Coroutine(CoroutineRunner runner)
    {
        Runner = runner;
    }

    /// True once the coroutine has finished (or been stopped).
    public bool IsDone => Runner.Finished;
}

/// Drives a single coroutine, including nested `IEnumerator`s pushed onto an
/// internal stack. Advanced once per tick by the owning <see cref="ZEScript"/>.
internal sealed class CoroutineRunner
{
    private sealed class Frame
    {
        public IEnumerator Enumerator;
        public object? Pending;
        public bool Started;

        public Frame(IEnumerator enumerator)
        {
            Enumerator = enumerator;
        }
    }

    private readonly Stack<Frame> _stack = new();

    public IEnumerator Root { get; }

    public bool Finished { get; private set; }

    public CoroutineRunner(IEnumerator routine)
    {
        Root = routine;
        _stack.Push(new Frame(routine));
    }

    public void Stop()
    {
        _stack.Clear();
        Finished = true;
    }

    /// Advances the coroutine by at most one step this tick. `isFixed` selects
    /// whether this is a fixed-update tick (drives WaitForFixedUpdate) or a
    /// normal update tick (drives everything else, including WaitForSeconds
    /// timing).
    public void Tick(bool isFixed)
    {
        if (Finished)
        {
            return;
        }

        // Bounded so a pathological chain of empty nested coroutines can't spin
        // forever within a single tick.
        int guard = 0;
        while (_stack.Count > 0)
        {
            if (guard++ > 1024)
            {
                Finished = true;
                return;
            }

            var frame = _stack.Peek();
            bool freshlyStarted = !frame.Started;

            if (freshlyStarted)
            {
                frame.Started = true;
                if (!Advance(frame))
                {
                    PopFinishedFrame();
                    continue;
                }
            }

            object? current = frame.Pending;

            // A bare IEnumerator yield descends into a child coroutine which must
            // finish before the parent continues. (YieldInstruction and Coroutine
            // don't implement IEnumerator, so this can't match them.)
            if (current is IEnumerator nested)
            {
                _stack.Push(new Frame(nested));
                continue;
            }

            if (freshlyStarted)
            {
                // This frame's first MoveNext() already ran above (to reach its
                // first yield) -- that's this tick's one step. Resolving an
                // instantly-ready yield (e.g. `yield return null`) here too would
                // advance twice in one tick, so a fresh `yield return null` would
                // never actually wait a frame. Defer it to the next Tick() call.
                return;
            }

            if (!IsReady(current, isFixed))
            {
                return;
            }

            if (!Advance(frame))
            {
                PopFinishedFrame();
                if (_stack.Count == 0)
                {
                    Finished = true;
                }

                return;
            }

            return;
        }

        Finished = true;
    }

    private static bool Advance(Frame frame)
    {
        bool moved = frame.Enumerator.MoveNext();
        frame.Pending = moved ? frame.Enumerator.Current : null;
        return moved;
    }

    private void PopFinishedFrame()
    {
        _stack.Pop();
        if (_stack.Count > 0)
        {
            // The parent's Current still references the finished child enumerator;
            // clear it so we resume the parent next tick instead of re-descending.
            _stack.Peek().Pending = null;
        }
    }

    private static bool IsReady(object? current, bool isFixed)
    {
        switch (current)
        {
            case null:
            case WaitForEndOfFrame:
                return !isFixed;
            case WaitForFixedUpdate:
                return isFixed;
            case WaitForSeconds wait:
                if (!isFixed)
                {
                    wait.Remaining -= Time.DeltaTime;
                }

                return wait.Remaining <= 0.0f;
            case Coroutine coroutine:
                return coroutine.IsDone;
            default:
                // Unknown yield object -> treat like `null` (resume next update).
                return !isFixed;
        }
    }
}
