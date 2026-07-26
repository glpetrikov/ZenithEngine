using System;
using System.IO;
using System.Text;

namespace ZeroEngine;

public static unsafe class Log
{
    private static readonly TextWriter FallbackError = new StreamWriter(Console.OpenStandardError(), Encoding.UTF8) { AutoFlush = true };

    public static void Info(string message)
    {
        var api = EngineAPI.CurrentOrNull;
        delegate* unmanaged[Cdecl]<byte*, int, void> sink = null;
        if (api != null)
        {
            sink = api->log_info;
        }
        Write("Info", message, api, sink);
    }

    public static void Warn(string message)
    {
        var api = EngineAPI.CurrentOrNull;
        delegate* unmanaged[Cdecl]<byte*, int, void> sink = null;
        if (api != null)
        {
            sink = api->log_warn;
        }
        Write("Warn", message, api, sink);
    }

    public static void Error(string message)
    {
        var api = EngineAPI.CurrentOrNull;
        delegate* unmanaged[Cdecl]<byte*, int, void> sink = null;
        if (api != null)
        {
            sink = api->log_error;
        }
        Write("Error", message, api, sink);
    }

    public static void Debug(string message)
    {
        var api = EngineAPI.CurrentOrNull;
        delegate* unmanaged[Cdecl]<byte*, int, void> sink = null;
        if (api != null)
        {
            sink = api->log_debug;
        }
        Write("Debug", message, api, sink);
    }

    private static void Write(string level, string message, EngineAPI* api, delegate* unmanaged[Cdecl]<byte*, int, void> sink)
    {
        message ??= string.Empty;
        if (api == null)
        {
            FallbackError.WriteLine($"ZeroEngine Log.{level} dropped before EngineAPI initialization: {message}");
            return;
        }

        if (sink == null)
        {
            FallbackError.WriteLine($"ZeroEngine Log.{level} bridge is unavailable: {message}");
            return;
        }

        var bytes = Encoding.UTF8.GetBytes(message);
        fixed (byte* ptr = bytes)
        {
            sink(ptr, bytes.Length);
        }
    }
}
