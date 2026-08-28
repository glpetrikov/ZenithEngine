using System.Collections;
using ZeroEngine;

/// Temporary headless health-check fixture. Exercises the Entity/Rigidbody/
/// Transform/Coroutine C# API surface. Self-checkable results go into a
/// bitmask reported every tick via set_velocity, mirroring the
/// EntityLivenessProbe convention. Rigidbody force/torque/velocity-setter
/// calls are made unconditionally and verified from the Rust side via
/// drain_commands(), since their effect isn't observable from C# alone in
/// this headless (no real physics) harness.
///
/// A few checks (Instantiate, AddComponent, SetActive) mutate this same
/// entity/a freshly-created one and then need to see that mutation reflected
/// -- the engine's per-tick component cache only refreshes at the start/end
/// of each `update()` tick, so a mutation isn't visible via
/// HasComponent/TryGetComponent until *at least* the next tick. Those checks
/// are deferred into the coroutine (which naturally spans multiple ticks)
/// rather than verified same-tick.
public class ApiHealthCheckScript : ZEScript
{
    public const int FindBit = 1 << 0;
    public const int FindWithTagBit = 1 << 1;
    public const int FindEntitiesWithTagBit = 1 << 2;
    public const int FindByIdBit = 1 << 3;
    public const int InstantiateBit = 1 << 4;
    public const int DestroyBit = 1 << 5;
    public const int SetActiveIsActiveBit = 1 << 6;
    public const int AddRemoveHasComponentBit = 1 << 7;
    public const int GetTryGetComponentBit = 1 << 8;
    public const int TryGetComponentScriptBit = 1 << 9;
    public const int NameValueBit = 1 << 10;
    public const int TagValueBit = 1 << 11;
    public const int HasTagBit = 1 << 12;
    public const int RigidbodyVelocityGetterBit = 1 << 13;
    public const int TransformPositionBit = 1 << 14;
    public const int TransformRotationBit = 1 << 15;
    public const int TransformScaleBit = 1 << 16;
    public const int CoroutineYieldNullBit = 1 << 17;
    public const int CoroutineWaitForSecondsBit = 1 << 18;

    public const int FullMask =
        FindBit | FindWithTagBit | FindEntitiesWithTagBit | FindByIdBit | InstantiateBit | DestroyBit
        | SetActiveIsActiveBit | AddRemoveHasComponentBit | GetTryGetComponentBit | TryGetComponentScriptBit
        | NameValueBit | TagValueBit | HasTagBit | RigidbodyVelocityGetterBit | TransformPositionBit
        | TransformRotationBit | TransformScaleBit | CoroutineYieldNullBit | CoroutineWaitForSecondsBit;

    private int _resultMask;
    private bool _ran;
    private Entity _instantiated;
    private Entity _componentOpsEntity;

    public override unsafe void OnUpdate()
    {
        if (!_ran)
        {
            _ran = true;
            RunOneShotChecks();
            StartCoroutine(CoroutineCheck());

            // Rigidbody force/torque/velocity-setter calls: not self-verifiable
            // in C# under this headless mock (no real physics simulation), so
            // just invoke them once each -- the Rust test asserts the right
            // ScriptingApiCommand was queued for each via drain_commands().
            var rb = GetComponent<Rigidbody>();
            rb.Velocity = new Vector2(9f, 9f);
            rb.Add2DForce(1f, 2f, ForceMode.Force);
            rb.Add2DForce(3f, 4f, ForceMode.Impulse);
            rb.AddTorque(5f, ForceMode.Force);
            rb.AddTorque(6f, ForceMode.Impulse);
        }

        EngineAPI.Current->set_velocity(EntityId, _resultMask, 0f);
    }

    private void RunOneShotChecks()
    {
        if (Entity.Find("FindableEntity").IsValid && !Entity.Find("NoSuchEntity12345").IsValid)
        {
            _resultMask |= FindBit;
        }
        else
        {
            Log.Error("BUG: Entity.Find did not behave as expected.");
        }

        if (Entity.FindWithTag("HealthCheckTag") == Self)
        {
            _resultMask |= FindWithTagBit;
        }
        else
        {
            Log.Error("BUG: Entity.FindWithTag did not find this entity.");
        }

        if (Entity.FindEntitiesWithTag("MultiTag").Length == 2)
        {
            _resultMask |= FindEntitiesWithTagBit;
        }
        else
        {
            Log.Error($"BUG: Entity.FindEntitiesWithTag expected 2, got {Entity.FindEntitiesWithTag("MultiTag").Length}.");
        }

        if (Entity.FindById((int)Self.Index) == Self)
        {
            _resultMask |= FindByIdBit;
        }
        else
        {
            Log.Error("BUG: Entity.FindById did not resolve back to this entity.");
        }

        Entity template = Entity.Find("InstantiateTemplate");

        // Destroy is verified same-tick: the engine treats liveness (unlike
        // component-cache freshness) as always-current, so a destroyed
        // entity must be rejected immediately, not just after a cache
        // refresh.
        Entity toDestroy = Entity.Instantiate(template);
        toDestroy.Destroy();
        if (!toDestroy.TryGetComponent<Transform>(out _))
        {
            _resultMask |= DestroyBit;
        }
        else
        {
            Log.Error("BUG: destroyed entity still has a Transform.");
        }

        // Instantiate/AddComponent/SetActive: mutate now, verify next tick
        // (see CoroutineCheck) once the component cache has caught up.
        _instantiated = Entity.Instantiate(template, new Vector2(5f, 6f));
        _componentOpsEntity = Entity.Instantiate(template);
        _componentOpsEntity.AddComponent<Inactive>();
        Self.SetActive(false);

        if (Self.TryGetComponent<Transform>(out var selfTransform) && GetComponent<Transform>() is not null && selfTransform is not null)
        {
            _resultMask |= GetTryGetComponentBit;
        }
        else
        {
            Log.Error("BUG: GetComponent/TryGetComponent<Transform> failed on a live entity.");
        }

        AddComponent<ApiHealthCheckHelper>();
        if (Self.TryGetComponent<ApiHealthCheckHelper>(out var helper) && helper.Marker == 42)
        {
            _resultMask |= TryGetComponentScriptBit;
        }
        else
        {
            Log.Error("BUG: TryGetComponent<T> for a ZEScript subtype failed.");
        }

        if (GetComponent<Name>().Value == "HealthCheckEntity")
        {
            _resultMask |= NameValueBit;
        }
        else
        {
            Log.Error($"BUG: Name.Value was `{GetComponent<Name>().Value}`, expected `HealthCheckEntity`.");
        }

        if (GetComponent<Tag>().Value == "HealthCheckTag")
        {
            _resultMask |= TagValueBit;
        }
        else
        {
            Log.Error($"BUG: Tag.Value was `{GetComponent<Tag>().Value}`, expected `HealthCheckTag`.");
        }

        if (Self.HasTag("HealthCheckTag") && !Self.HasTag("SomeOtherTag"))
        {
            _resultMask |= HasTagBit;
        }
        else
        {
            Log.Error("BUG: HasTag did not behave as expected.");
        }

        var rb = GetComponent<Rigidbody>();
        if ((rb.Velocity - new Vector2(3f, 4f)).Length() < 0.01f)
        {
            _resultMask |= RigidbodyVelocityGetterBit;
        }
        else
        {
            Log.Error($"BUG: Rigidbody.Velocity getter returned {rb.Velocity}, expected (3, 4).");
        }

        var transform = GetComponent<Transform>();
        transform.Position = new Vector2(11f, 12f);
        if ((transform.Position - new Vector2(11f, 12f)).Length() < 0.01f)
        {
            _resultMask |= TransformPositionBit;
        }
        else
        {
            Log.Error($"BUG: Transform.Position round-trip failed, got {transform.Position}.");
        }

        transform.Rotation = 33f;
        if (System.Math.Abs(transform.Rotation - 33f) < 0.01f)
        {
            _resultMask |= TransformRotationBit;
        }
        else
        {
            Log.Error($"BUG: Transform.Rotation round-trip failed, got {transform.Rotation}.");
        }

        transform.Scale = new Vector2(2f, 3f);
        if ((transform.Scale - new Vector2(2f, 3f)).Length() < 0.01f)
        {
            _resultMask |= TransformScaleBit;
        }
        else
        {
            Log.Error($"BUG: Transform.Scale round-trip failed, got {transform.Scale}.");
        }
    }

    private IEnumerator CoroutineCheck()
    {
        ulong frameAtStart = Time.FrameCount;
        yield return null;
        if (Time.FrameCount > frameAtStart)
        {
            _resultMask |= CoroutineYieldNullBit;
        }
        else
        {
            Log.Error("BUG: `yield return null` did not advance to the next frame.");
        }

        // By now at least one full tick has completed since RunOneShotChecks
        // ran, so the component cache reflects last tick's mutations.
        bool instantiateOk = _instantiated.IsValid
            && _instantiated.TryGetComponent<Transform>(out var instantiatedTransform)
            && (instantiatedTransform.Position - new Vector2(5f, 6f)).Length() < 0.01f;
        if (instantiateOk)
        {
            _resultMask |= InstantiateBit;
        }
        else
        {
            Log.Error("BUG: Entity.Instantiate did not produce a valid positioned clone (checked one tick later).");
        }

        bool hasAfterAdd = _componentOpsEntity.HasComponent<Inactive>();
        bool inactiveNow = !Self.IsActive;

        // Issue the "undo" mutations now; verified after the WaitForSeconds
        // below, once another tick boundary has passed.
        _componentOpsEntity.RemoveComponent<Inactive>();
        Self.SetActive(true);

        double startTime = Time.TimeSinceStartup;
        yield return new WaitForSeconds(0.05f);
        double elapsed = Time.TimeSinceStartup - startTime;
        if (elapsed >= 0.04)
        {
            _resultMask |= CoroutineWaitForSecondsBit;
        }
        else
        {
            Log.Error($"BUG: WaitForSeconds(0.05) resumed too early after {elapsed:F3}s.");
        }

        bool hasAfterRemove = !_componentOpsEntity.HasComponent<Inactive>();
        if (hasAfterAdd && hasAfterRemove)
        {
            _resultMask |= AddRemoveHasComponentBit;
        }
        else
        {
            Log.Error($"BUG: AddComponent/RemoveComponent/HasComponent round-trip failed (afterAdd={hasAfterAdd}, afterRemove={hasAfterRemove}).");
        }

        bool activeAgain = Self.IsActive;
        if (inactiveNow && activeAgain)
        {
            _resultMask |= SetActiveIsActiveBit;
        }
        else
        {
            Log.Error($"BUG: SetActive/IsActive round-trip failed (inactiveNow={inactiveNow}, activeAgain={activeAgain}).");
        }
    }
}
