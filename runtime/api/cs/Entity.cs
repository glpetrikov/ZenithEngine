using System.Text;

namespace ZeroEngine;

/// A lightweight handle to an engine entity, identified by its packed 64-bit id
/// (generation in the high 32 bits, index in the low 32 bits). Component
/// accessors operate on the live entity through the engine bridge; the handle
/// itself holds no component data.
public readonly unsafe struct Entity : IEquatable<Entity>
{
    /// Sentinel returned by the lookup helpers when nothing matches. Mirrors the
    /// native `INVALID_ENTITY`.
    public static readonly Entity Null = new(ulong.MaxValue);

    public ulong Id { get; }

    public Entity(ulong id)
    {
        Id = id;
    }

    public uint Index => (uint)(Id & 0xFFFFFFFF);

    public uint Generation => (uint)(Id >> 32);

    /// True when this handle refers to an actual entity: not the `Null`
    /// sentinel, and not a default-initialized (never-assigned) handle
    /// either. `default(Entity)` has `Id == 0`, which the engine reserves so
    /// it can never be a real, live entity's id -- unlike `Null` (`Id ==
    /// ulong.MaxValue`), it isn't returned by any lookup API, so seeing it
    /// means a script variable was never assigned.
    public bool IsValid => Id != ulong.MaxValue && Id != 0;

    /// True when the entity is active and False when entity have Inactive component.
    public bool IsActive
    {
        get => !HasComponent<Inactive>();
        set
        {
            if (value) RemoveComponent<Inactive>();
            else if (!HasComponent<Inactive>()) AddComponent<Inactive>();
        }
    }

    /// Sets the entity's active state.
    public void SetActive(bool active) => IsActive = active;

    /// Finds the first entity with the given name, or `Entity.Null` if none.
    public static Entity Find(string name)
    {
        if (string.IsNullOrEmpty(name))
        {
            return Null;
        }

        var bytes = Encoding.UTF8.GetBytes(name);
        fixed (byte* ptr = bytes)
        {
            return new Entity(EngineAPI.Current->find_entity_by_name(ptr, bytes.Length));
        }
    }

    /// Finds the first entity tagged with `tag`, or `Entity.Null` if none.
    public static Entity FindWithTag(string tag)
    {
        if (string.IsNullOrEmpty(tag))
        {
            return Null;
        }

        var bytes = Encoding.UTF8.GetBytes(tag);
        fixed (byte* ptr = bytes)
        {
            return new Entity(EngineAPI.Current->find_entity_by_tag(ptr, bytes.Length));
        }
    }

    /// Returns every entity tagged with `tag` (empty array if none).
    public static Entity[] FindEntitiesWithTag(string tag) => FindAllWithTag(tag);

    /// Returns every entity tagged with `tag` (empty array if none).
    public static Entity[] FindAllWithTag(string tag)
    {
        if (string.IsNullOrEmpty(tag))
        {
            return Array.Empty<Entity>();
        }

        var bytes = Encoding.UTF8.GetBytes(tag);
        fixed (byte* ptr = bytes)
        {
            // First call sizes the result, second fills it -- the count can
            // change in between only if the scene mutates mid-frame, which
            // scripts can't cause between two synchronous bridge calls.
            int count = EngineAPI.Current->find_entities_by_tag(ptr, bytes.Length, null, 0);
            if (count <= 0)
            {
                return Array.Empty<Entity>();
            }

            var ids = new ulong[count];
            int written;
            fixed (ulong* idPtr = ids)
            {
                written = EngineAPI.Current->find_entities_by_tag(ptr, bytes.Length, idPtr, count);
            }

            int usable = Math.Min(written, count);
            var result = new Entity[usable];
            for (int i = 0; i < usable; i++)
            {
                result[i] = new Entity(ids[i]);
            }

            return result;
        }
    }

    /// Finds the live entity with the given index, or `Entity.Null` if none.
    public static Entity FindById(int id) =>
        new(EngineAPI.Current->find_entity_by_id((uint)id));

    /// Spawns a component-for-component copy of `template` at the given position
    /// and rotation (degrees). Returns `Entity.Null` if the template could not
    /// be cloned (e.g. it no longer exists).
    public static Entity Instantiate(Entity template, Vector2 position, float rotation)
    {
        ulong newId = ulong.MaxValue;
        bool created = EngineAPI.Current->instantiate_entity(
            template.Id, position.X, position.Y, rotation, &newId);
        return created ? new Entity(newId) : Null;
    }

    public static Entity Instantiate(Entity template, Vector2 position) =>
        Instantiate(template, position, 0.0f);

    public static Entity Instantiate(Entity template) =>
        Instantiate(template, Vector2.Zero, 0.0f);

    /// Destroys the entity with the given index.
    public static void Destroy(int id) =>
        EngineAPI.Current->destroy_entity(new Entity(EngineAPI.Current->find_entity_by_id((uint)id)).Id);

    /// Destroys this entity (and its children).
    public void Destroy() => EngineAPI.Current->destroy_entity(Id);

    /// Adds a default-constructed component of type `T` and returns a handle to
    /// it. Data-carrying components (sprites, cameras, UI widgets) are not
    /// addable this way and are logged + skipped by the engine. When `T` is a
    /// `ZEScript` subclass, this instead creates and tracks a live script
    /// instance for this entity (firing `OnCreate`/`OnEnable`), mirroring what
    /// the engine does for scene-declared scripts.
    public T AddComponent<T>()
        where T : class, new()
    {
        if (!IsValid)
        {
            throw new InvalidOperationException(
                $"Cannot add {typeof(T).Name} to an invalid entity (Id={Id}).");
        }

        if (typeof(ZEScript).IsAssignableFrom(typeof(T)))
        {
            return (T)(object)ZEScript.AddInstance(Id, typeof(T));
        }

        if (!typeof(ZEComponent).IsAssignableFrom(typeof(T)))
        {
            throw new InvalidOperationException(
                $"{typeof(T).Name} is neither a ZEComponent nor a ZEScript.");
        }

        var component = (ZEComponent)(object)new T();
        EngineAPI.Current->add_component(Id, (uint)component.ComponentType);
        component.Bind(Id);
        return (T)(object)component;
    }

    /// Removes component `T`. When `T` is a `ZEScript` subclass, this instead
    /// tears down the tracked script instance for this entity (firing
    /// `OnDisable`/`OnDestroy`).
    public void RemoveComponent<T>()
        where T : class, new()
    {
        if (!IsValid)
        {
            return;
        }

        if (typeof(ZEScript).IsAssignableFrom(typeof(T)))
        {
            ZEScript.RemoveInstance(Id, typeof(T));
            return;
        }

        if (!typeof(ZEComponent).IsAssignableFrom(typeof(T)))
        {
            throw new InvalidOperationException(
                $"{typeof(T).Name} is neither a ZEComponent nor a ZEScript.");
        }

        var component = (ZEComponent)(object)new T();
        EngineAPI.Current->remove_component(Id, (uint)component.ComponentType);
    }

    /// True when this entity has component `T`. When `T` is a `ZEScript`
    /// subclass, this checks for a tracked live script instance instead of a
    /// native component.
    public bool HasComponent<T>()
        where T : class, new()
    {
        if (!IsValid)
        {
            return false;
        }

        if (typeof(ZEScript).IsAssignableFrom(typeof(T)))
        {
            return ZEScript.TryGetInstance(Id, typeof(T), out _);
        }

        if (!typeof(ZEComponent).IsAssignableFrom(typeof(T)))
        {
            throw new InvalidOperationException(
                $"{typeof(T).Name} is neither a ZEComponent nor a ZEScript.");
        }

        var component = (ZEComponent)(object)new T();
        return EngineAPI.HasComponent(Id, component.ComponentType);
    }

    /// True when this entity has a `Tag` component whose value equals `tag`.
    public bool HasTag(string tag) =>
        TryGetComponent<Tag>(out var component) && component.Value == tag;

    /// Attempts to fetch component `T`. When `T` is a `ZEScript` subclass,
    /// this returns the live script instance the engine already tracks for
    /// this entity (not a copy) instead of a `ZEComponent` proxy.
    public bool TryGetComponent<T>(out T component)
        where T : class
    {
        if (!IsValid)
        {
            component = default!;
            return false;
        }

        if (typeof(ZEScript).IsAssignableFrom(typeof(T)))
        {
            if (ZEScript.TryGetInstance(Id, typeof(T), out var script))
            {
                component = (T)(object)script!;
                return true;
            }

            component = default!;
            return false;
        }

        if (!typeof(ZEComponent).IsAssignableFrom(typeof(T)))
        {
            throw new InvalidOperationException(
                $"{typeof(T).Name} is neither a ZEComponent nor a ZEScript.");
        }

        var candidate = (ZEComponent)Activator.CreateInstance(typeof(T))!;
        if (!EngineAPI.HasComponent(Id, candidate.ComponentType))
        {
            component = default!;
            return false;
        }

        candidate.Bind(Id);
        component = (T)(object)candidate;
        return true;
    }

    /// Returns a handle to component `T` (or the live script instance if `T`
    /// is a `ZEScript` subclass), throwing if the entity lacks it.
    public T GetComponent<T>()
        where T : class
    {
        if (!TryGetComponent<T>(out var component))
        {
            throw new InvalidOperationException(
                $"Entity {Index}.{Generation} does not have component {typeof(T).Name}.");
        }

        return component;
    }

    public bool Equals(Entity other) => Id == other.Id;

    public override bool Equals(object? obj) => obj is Entity other && Equals(other);

    public override int GetHashCode() => Id.GetHashCode();

    public static bool operator ==(Entity left, Entity right) => left.Equals(right);

    public static bool operator !=(Entity left, Entity right) => !left.Equals(right);

    public override string ToString() => IsValid ? $"Entity({Index}.{Generation})" : "Entity(Null)";
}
