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

    /// True when this handle refers to an actual entity (i.e. it is not the
    /// `Null` sentinel).
    public bool IsValid => Id != ulong.MaxValue;

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
    /// addable this way and are logged + skipped by the engine.
    public T AddComponent<T>()
        where T : ZEComponent, new()
    {
        var component = new T();
        EngineAPI.Current->add_component(Id, (uint)component.ComponentType);
        component.Bind(Id);
        return component;
    }

    public void RemoveComponent<T>()
        where T : ZEComponent, new()
    {
        var component = new T();
        EngineAPI.Current->remove_component(Id, (uint)component.ComponentType);
    }

    public bool HasComponent<T>()
        where T : ZEComponent, new()
    {
        var component = new T();
        return EngineAPI.HasComponent(Id, component.ComponentType);
    }

    public bool TryGetComponent<T>(out T component)
        where T : ZEComponent, new()
    {
        var candidate = new T();
        if (!EngineAPI.HasComponent(Id, candidate.ComponentType))
        {
            component = default!;
            return false;
        }

        candidate.Bind(Id);
        component = candidate;
        return true;
    }

    /// Returns a handle to component `T`, throwing if the entity lacks it.
    public T GetComponent<T>()
        where T : ZEComponent, new()
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
