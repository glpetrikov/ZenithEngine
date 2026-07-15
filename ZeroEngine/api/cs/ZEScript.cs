using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;

namespace ZeroEngine;

public abstract class ZEScript
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase,
    };

    // Composite key: "{entityId}:{classPath}" so multiple scripts on one entity
    // each have their own instance slot and cannot clobber each other.
    private static readonly Dictionary<string, ZEScript> _instances = new();

    private static string InstanceKey(ulong entityId, string classPath) => $"{entityId}:{classPath}";
    private static string? _lastBridgeError;

    // Components handed out via GetComponent<T>() so they can be polled once
    // per frame (see PollBoundComponents), e.g. to fire Button.SetOnClick
    // callbacks without requiring the script to call IsClicked() itself.
    private readonly List<ZEComponent> _boundComponents = new();

    public ulong EntityId { get; protected set; }

    public uint EntityIndex => (uint)(EntityId & 0xFFFFFFFF);

    public uint EntityGeneration => (uint)(EntityId >> 32);

    public T GetComponent<T>()
        where T : ZEComponent, new()
    {
        var component = new T();
        component.Bind(EntityId);

        if (!EngineAPI.HasComponent(EntityId, component.ComponentType))
        {
            throw new InvalidOperationException(
                $"Entity {EntityIndex}.{EntityGeneration} does not have component {typeof(T).Name}.");
        }

        _boundComponents.Add(component);
        return component;
    }

    private void PollBoundComponents()
    {
        foreach (var component in _boundComponents)
        {
            component.PollEvents();
        }
    }

    public virtual void OnCreate() { }

    public virtual void OnStart() { }

    public virtual void OnDestroy() { }

    public virtual void OnUpdate() { }

    public virtual void OnFixedUpdate() { }

    public virtual void OnEnable() { }

    public virtual void OnDisable() { }

    public virtual void OnContactEnter(ulong otherEntity) { }

    public virtual void OnContactStay(ulong otherEntity) { }

    public virtual void OnContactExit(ulong otherEntity) { }

    public virtual void OnSensorEnter(ulong otherEntity) { }

    public virtual void OnSensorStay(ulong otherEntity) { }

    public virtual void OnSensorExit(ulong otherEntity) { }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnEngineInit(
        ulong entityId,
        EngineAPI* api,
        byte* classPathPtr,
        int classPathLength)
    {
        try
        {
            EngineAPI.Initialize(api);
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            var key = InstanceKey(entityId, classPath);

            if (_instances.ContainsKey(key))
            {
                return;
            }

            var scriptType = ResolveScriptType(classPath);
            var instance = (ZEScript)Activator.CreateInstance(scriptType)!;
            instance.EntityId = entityId;
            _instances[key] = instance;
        }
        catch (Exception ex)
        {
            ReportException($"failed to initialize script instance for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe int NativeDescribeScriptFields(byte* classPathPtr, int classPathLength, byte* outBuffer, int outBufferLength)
    {
        try
        {
            if (classPathPtr is null || classPathLength <= 0)
            {
                return -1;
            }

            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            var scriptType = ResolveScriptType(classPath);
            var fields = DescribeScriptFields(scriptType);
            var json = JsonSerializer.Serialize(fields, JsonOptions);
            return WriteUtf8(json, outBuffer, outBufferLength);
        }
        catch (Exception ex)
        {
            ReportException("failed to describe C# script fields", ex);
            return -1;
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe int NativeListScripts(byte* outBuffer, int outBufferLength)
    {
        try
        {
            var json = JsonSerializer.Serialize(ListScriptClasses(), JsonOptions);
            return WriteUtf8(json, outBuffer, outBufferLength);
        }
        catch (Exception ex)
        {
            ReportException("failed to list C# script classes", ex);
            return -1;
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe int NativeGetLastScriptBridgeError(byte* outBuffer, int outBufferLength)
    {
        try
        {
            return WriteUtf8(_lastBridgeError ?? string.Empty, outBuffer, outBufferLength);
        }
        catch
        {
            return -1;
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe int NativeApplyScriptFields(
        ulong entityId,
        byte* classPathPtr,
        int classPathLength,
        byte* fieldsJsonPtr,
        int fieldsJsonLength)
    {
        try
        {
            if (fieldsJsonPtr is null || fieldsJsonLength <= 0)
            {
                return 0;
            }

            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            var key = InstanceKey(entityId, classPath);
            var json = Encoding.UTF8.GetString(fieldsJsonPtr, fieldsJsonLength);
            if (!_instances.TryGetValue(key, out var instance))
            {
                return -1;
            }

            var fields = JsonSerializer.Deserialize<Dictionary<string, ScriptFieldValuePayload>>(json, JsonOptions);
            if (fields is null)
            {
                return 0;
            }

            ApplyScriptFields(instance, fields);
            return 0;
        }
        catch (Exception ex)
        {
            ReportException($"failed to apply C# script fields for entity {entityId}", ex);
            return -1;
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnCreate(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnCreate();
        }
        catch (Exception ex)
        {
            ReportException($"script OnCreate failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnStart(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnStart();
        }
        catch (Exception ex)
        {
            ReportException($"script OnStart failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnDestroy(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            var key = InstanceKey(entityId, classPath);
            if (_instances.Remove(key, out var instance))
            {
                instance.OnDestroy();
            }
        }
        catch (Exception ex)
        {
            ReportException($"script OnDestroy failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnUpdate(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            var instance = Lookup(entityId, classPath);
            instance.PollBoundComponents();
            instance.OnUpdate();
        }
        catch (Exception ex)
        {
            ReportException($"script OnUpdate failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnFixedUpdate(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnFixedUpdate();
        }
        catch (Exception ex)
        {
            ReportException($"script OnFixedUpdate failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnEnable(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnEnable();
        }
        catch (Exception ex)
        {
            ReportException($"script OnEnable failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnDisable(ulong entityId, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnDisable();
        }
        catch (Exception ex)
        {
            ReportException($"script OnDisable failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnContactEnter(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnContactEnter(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnContactEnter failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnContactStay(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnContactStay(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnContactStay failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnContactExit(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnContactExit(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnContactExit failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnSensorEnter(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnSensorEnter(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnSensorEnter failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnSensorStay(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnSensorStay(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnSensorStay failed for entity {entityId}", ex);
        }
    }

    [UnmanagedCallersOnly]
    public static unsafe void NativeOnSensorExit(ulong entityId, ulong otherEntity, byte* classPathPtr, int classPathLength)
    {
        try
        {
            var classPath = Encoding.UTF8.GetString(classPathPtr, classPathLength);
            Lookup(entityId, classPath).OnSensorExit(otherEntity);
        }
        catch (Exception ex)
        {
            ReportException($"script OnSensorExit failed for entity {entityId}", ex);
        }
    }

    private static ZEScript Lookup(ulong entityId, string classPath)
    {
        var key = InstanceKey(entityId, classPath);
        if (!_instances.TryGetValue(key, out var instance))
        {
            throw new InvalidOperationException($"No script instance registered for entity id {entityId} with class `{classPath}`.");
        }

        return instance;
    }

    private static void ReportException(string context, Exception exception)
    {
        var message = $"{context}: {exception}";
        _lastBridgeError = message;
        try
        {
            Log.Error(message);
        }
        catch
        {
            Console.Error.WriteLine(message);
        }
    }

    private static Type ResolveScriptType(string classPath)
    {
        var scriptsAssembly = Assembly.GetExecutingAssembly();
        var assemblyName = scriptsAssembly.GetName().Name
            ?? throw new InvalidOperationException("Could not resolve scripts assembly name.");
        var fullClassPath = classPath.Contains('.') ? classPath : $"{assemblyName}.{classPath}";
        var scriptType = Type.GetType($"{fullClassPath}, {assemblyName}", throwOnError: false)
            ?? scriptsAssembly.GetType(fullClassPath, throwOnError: false)
            ?? scriptsAssembly.GetType(classPath, throwOnError: false);

        if (scriptType is null)
        {
            throw new InvalidOperationException($"Could not resolve script type `{fullClassPath}` in `{assemblyName}`.");
        }

        if (!typeof(ZEScript).IsAssignableFrom(scriptType))
        {
            throw new InvalidOperationException($"Script type `{fullClassPath}` must inherit from ZEScript.");
        }

        return scriptType;
    }

    private static string[] ListScriptClasses()
    {
        return Assembly.GetExecutingAssembly()
            .GetTypes()
            .Where(type => !type.IsAbstract && typeof(ZEScript).IsAssignableFrom(type))
            .Select(type => type.FullName ?? type.Name)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();
    }

    private static ScriptFieldMetadata[] DescribeScriptFields(Type scriptType)
    {
        var fields = scriptType
            .GetFields(BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic)
            .Where(ShouldExposeScriptField)
            .OrderBy(field => field.Name, StringComparer.Ordinal)
            .ToArray();

        var defaultValueInstance = fields.Any(field => IsSupportedDefaultValueType(field.FieldType))
            ? CreateDefaultValueInstance(scriptType)
            : null;

        return fields
            .Select(field => new ScriptFieldMetadata
            {
                Name = field.Name,
                Type = FormatTypeName(field.FieldType),
                Source = field.IsPublic ? "public" : "zeroField",
                DefaultValue = ReadDefaultFieldValue(field, defaultValueInstance),
            })
            .ToArray();
    }

    private static bool ShouldExposeScriptField(FieldInfo field)
    {
        if (field.IsStatic || field.IsLiteral)
        {
            return false;
        }

        if (field.IsDefined(typeof(CompilerGeneratedAttribute), inherit: false)
            || field.Name.Contains("k__BackingField", StringComparison.Ordinal))
        {
            return false;
        }

        return field.IsPublic || field.IsDefined(typeof(ZeroFieldAttribute), inherit: false);
    }

    private static bool IsSupportedDefaultValueType(Type type) =>
        type == typeof(bool) || type == typeof(int) || type == typeof(uint) || type == typeof(float) || type == typeof(string)
        || type == typeof(Vector2) || type == typeof(Vector3);

    private static ZEScript CreateDefaultValueInstance(Type scriptType)
    {
        var instance = Activator.CreateInstance(scriptType) as ZEScript;
        return instance
            ?? throw new InvalidOperationException($"Could not create script instance for `{scriptType.FullName}` to read field defaults.");
    }

    private static ScriptFieldDefaultValuePayload? ReadDefaultFieldValue(FieldInfo field, ZEScript? instance)
    {
        if (instance is null || !IsSupportedDefaultValueType(field.FieldType))
        {
            return null;
        }

        try
        {
            var value = field.GetValue(instance);
            if (field.FieldType == typeof(bool) && value is bool boolValue)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "bool", Value = boolValue };
            }

            if (field.FieldType == typeof(int) && value is int intValue)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "int", Value = intValue };
            }

            if (field.FieldType == typeof(uint) && value is uint uintValue)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "uint", Value = uintValue };
            }

            if (field.FieldType == typeof(float) && value is float floatValue)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "float", Value = floatValue };
            }

            if (field.FieldType == typeof(string))
            {
                return new ScriptFieldDefaultValuePayload { Kind = "string", Value = value as string ?? string.Empty };
            }

            if (field.FieldType == typeof(Vector2) && value is Vector2 vec2)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "vec2", Value = vec2 };
            }

            if (field.FieldType == typeof(Vector3) && value is Vector3 vec3)
            {
                return new ScriptFieldDefaultValuePayload { Kind = "vec3", Value = vec3 };
            }
        }
        catch (Exception ex)
        {
            throw new InvalidOperationException(
                $"Failed to read default value for script field `{field.DeclaringType?.FullName}.{field.Name}`.",
                ex);
        }

        throw new InvalidOperationException(
            $"Default value for script field `{field.DeclaringType?.FullName}.{field.Name}` did not match field type `{field.FieldType.Name}`.");
    }

    private static string FormatTypeName(Type type)
    {
        if (type == typeof(bool)) return "bool";
        if (type == typeof(byte)) return "byte";
        if (type == typeof(sbyte)) return "sbyte";
        if (type == typeof(short)) return "short";
        if (type == typeof(ushort)) return "ushort";
        if (type == typeof(int)) return "int";
        if (type == typeof(uint)) return "uint";
        if (type == typeof(long)) return "long";
        if (type == typeof(ulong)) return "ulong";
        if (type == typeof(float)) return "float";
        if (type == typeof(double)) return "double";
        if (type == typeof(decimal)) return "decimal";
        if (type == typeof(char)) return "char";
        if (type == typeof(string)) return "string";
        if (type == typeof(object)) return "object";

        if (type.IsArray)
        {
            return $"{FormatTypeName(type.GetElementType()!)}[]";
        }

        var nullableType = Nullable.GetUnderlyingType(type);
        if (nullableType is not null)
        {
            return $"{FormatTypeName(nullableType)}?";
        }

        if (type.IsGenericType)
        {
            var typeName = type.Name;
            var tickIndex = typeName.IndexOf('`');
            if (tickIndex >= 0)
            {
                typeName = typeName[..tickIndex];
            }

            var genericArguments = string.Join(", ", type.GetGenericArguments().Select(FormatTypeName));
            return $"{typeName}<{genericArguments}>";
        }

        return type.Name;
    }

    private static void ApplyScriptFields(ZEScript instance, Dictionary<string, ScriptFieldValuePayload> fields)
    {
        var scriptType = instance.GetType();
        foreach (var (name, payload) in fields)
        {
            var field = scriptType.GetField(name, BindingFlags.Instance | BindingFlags.Public | BindingFlags.NonPublic);
            if (field is null || field.IsStatic)
            {
                Log.Warn($"Script field `{scriptType.FullName}.{name}` is missing.");
                continue;
            }

            if (!field.IsPublic && !field.IsDefined(typeof(ZeroFieldAttribute), inherit: false))
            {
                Log.Warn($"Script field `{scriptType.FullName}.{name}` is not editable.");
                continue;
            }

            if (!TryConvertFieldValue(field.FieldType, payload, out var value))
            {
                Log.Warn($"Script field `{scriptType.FullName}.{name}` has unsupported type `{field.FieldType.Name}`.");
                continue;
            }

            field.SetValue(instance, value);
        }
    }

    private static bool TryConvertFieldValue(Type fieldType, ScriptFieldValuePayload payload, out object? value)
    {
        value = null;
        try
        {
            switch (payload.Kind)
            {
                case "bool" when fieldType == typeof(bool):
                    value = payload.Value.GetBoolean();
                    return true;
                case "int" when fieldType == typeof(int):
                    value = payload.Value.GetInt32();
                    return true;
                case "uint" when fieldType == typeof(uint):
                    value = payload.Value.GetUInt32();
                    return true;
                case "float" when fieldType == typeof(float):
                    value = payload.Value.GetSingle();
                    return true;
                case "string" when fieldType == typeof(string):
                    value = payload.Value.GetString() ?? string.Empty;
                    return true;
                case "vec2" when fieldType == typeof(Vector2):
                    {
                        // Rust serialises glam::Vec2 as {x, y} via serde.
                        var x = payload.Value.GetProperty("x").GetSingle();
                        var y = payload.Value.GetProperty("y").GetSingle();
                        value = new Vector2(x, y);
                        return true;
                    }
                case "vec3" when fieldType == typeof(Vector3):
                    {
                        // Rust serialises glam::Vec3 as {x, y, z} via serde.
                        var x = payload.Value.GetProperty("x").GetSingle();
                        var y = payload.Value.GetProperty("y").GetSingle();
                        var z = payload.Value.GetProperty("z").GetSingle();
                        value = new Vector3(x, y, z);
                        return true;
                    }
                default:
                    return false;
            }
        }
        catch (Exception)
        {
            return false;
        }
    }

    private static unsafe int WriteUtf8(string text, byte* outBuffer, int outBufferLength)
    {
        var bytes = Encoding.UTF8.GetBytes(text);
        if (outBuffer is null || outBufferLength <= 0)
        {
            return bytes.Length;
        }

        var written = Math.Min(bytes.Length, outBufferLength);
        fixed (byte* src = bytes)
        {
            Buffer.MemoryCopy(src, outBuffer, outBufferLength, written);
        }

        return bytes.Length;
    }

    public sealed class ScriptFieldMetadata
    {
        public string Name { get; init; } = string.Empty;

        public string Type { get; init; } = string.Empty;

        public string Source { get; init; } = string.Empty;

        public ScriptFieldDefaultValuePayload? DefaultValue { get; init; }
    }

    public sealed class ScriptFieldDefaultValuePayload
    {
        public string Kind { get; init; } = string.Empty;

        public object? Value { get; init; }
    }

    private sealed class ScriptFieldValuePayload
    {
        public string Kind { get; init; } = string.Empty;

        public JsonElement Value { get; init; }
    }
}
