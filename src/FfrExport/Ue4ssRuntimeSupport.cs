using System.Diagnostics;
using System.Text;
using System.Text.Json;

namespace FfrExport;

internal enum Ue4ssInstallState
{
    NotInstalled,
    InstalledByFfrExport,
    InstalledByUser,
    PartialUnknown,
}

internal sealed record Ue4ssInspection(
    Ue4ssInstallState State,
    string Win64Directory,
    string Ue4ssDirectory,
    string MarkerPath,
    bool HasDwmapiDll,
    bool HasUe4ssDll,
    bool HasValidMarker,
    string? InstalledVersion);

/// <summary>
/// Shared UE4SS runtime inspection, temporary mod cleanup, and game process lifecycle helpers
/// linked into both FfrExport (CLI) and FfrExport.Gui (WinForms) without pulling CUE4Parse or network dependencies.
/// </summary>
internal static class Ue4ssRuntimeSupport
{
    internal const string GameProcessName = "FFRS-Win64-Shipping";
    internal const string GameExeFileName = "FFRS-Win64-Shipping.exe";
    internal const string RelativeWin64Subpath = "FFRS/Binaries/Win64";
    internal const string RelativeUe4ssSubpath = "FFRS/Binaries/Win64/ue4ss";
    internal const string DwmapiFileName = "dwmapi.dll";
    internal const string Ue4ssDllFileName = "UE4SS.dll";
    internal const string MarkerFileName = ".installed-by-ffr-export.json";
    internal const string DumpModName = "FfrUsmapDump";
    internal const string EnabledTxtFileName = "enabled.txt";
    internal const string ModsTxtFileName = "mods.txt";
    internal const string DoneMarkerFileName = "ffr-export-usmap.done";
    internal const string SteamRunGameUri = "steam://rungameid/3259780";
    internal const string InstalledByIdentifier = "ffr-export";

    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    internal static string GetWin64Directory(string gameDirectory)
    {
        return SteamLocator.CombineRelativeSubpath(
            PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDirectory)),
            RelativeWin64Subpath);
    }

    internal static string GetUe4ssDirectory(string gameDirectory)
    {
        return Path.Combine(GetWin64Directory(gameDirectory), "ue4ss");
    }

    internal static string GetInstallMarkerPath(string gameDirectory)
    {
        return Path.Combine(GetUe4ssDirectory(gameDirectory), MarkerFileName);
    }

    internal static string GetDumpModDirectory(string gameDirectory)
    {
        return Path.Combine(GetUe4ssDirectory(gameDirectory), "Mods", DumpModName);
    }

    internal static string GetDoneMarkerPath(string gameDirectory)
    {
        return Path.Combine(GetUe4ssDirectory(gameDirectory), DoneMarkerFileName);
    }

    /// <summary>
    /// Returns true only when the marker file ue4ss/.installed-by-ffr-export.json exists
    /// and contains valid metadata written by ffr-export.
    /// </summary>
    internal static bool HasOwnInstallMarker(string? gameDirectory)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return false;
        }

        try
        {
            var markerPath = GetInstallMarkerPath(gameDirectory);
            return TryReadMarkerHeader(markerPath, out _);
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// Inspects whether UE4SS is installed in the given game directory and distinguishes between
    /// a user's pre-existing installation and an installation managed by ffr-export.
    /// </summary>
    internal static Ue4ssInspection InspectUe4ssState(string gameDirectory)
    {
        var win64Dir = GetWin64Directory(gameDirectory);
        var ue4ssDir = Path.Combine(win64Dir, "ue4ss");
        var dwmapiPath = Path.Combine(win64Dir, DwmapiFileName);
        var ue4ssDllPath = Path.Combine(ue4ssDir, Ue4ssDllFileName);
        var markerPath = Path.Combine(ue4ssDir, MarkerFileName);

        var hasDwmapi = File.Exists(PathSupport.ToIoPath(dwmapiPath));
        var hasUe4ssDll = File.Exists(PathSupport.ToIoPath(ue4ssDllPath));
        var hasValidMarker = TryReadMarkerHeader(markerPath, out var version);

        Ue4ssInstallState state;
        if (hasDwmapi && hasUe4ssDll)
        {
            state = hasValidMarker
                ? Ue4ssInstallState.InstalledByFfrExport
                : Ue4ssInstallState.InstalledByUser;
        }
        else if (!hasDwmapi && !hasUe4ssDll && !hasValidMarker)
        {
            state = Ue4ssInstallState.NotInstalled;
        }
        else if (hasValidMarker)
        {
            state = Ue4ssInstallState.InstalledByFfrExport;
        }
        else
        {
            state = Ue4ssInstallState.PartialUnknown;
        }

        return new Ue4ssInspection(
            state,
            win64Dir,
            ue4ssDir,
            markerPath,
            hasDwmapi,
            hasUe4ssDll,
            hasValidMarker,
            version);
    }

    internal static bool TryReadMarkerHeader(string markerPath, out string? version)
    {
        version = null;
        try
        {
            var ioPath = PathSupport.ToIoPath(markerPath);
            if (!File.Exists(ioPath))
            {
                return false;
            }

            var json = File.ReadAllText(ioPath, Utf8WithoutBom);
            using var doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return false;
            }

            if (!doc.RootElement.TryGetProperty("installed_by", out var installedByProp)
                || installedByProp.ValueKind != JsonValueKind.String
                || !string.Equals(installedByProp.GetString(), InstalledByIdentifier, StringComparison.Ordinal))
            {
                return false;
            }

            if (!doc.RootElement.TryGetProperty("files", out var filesProp)
                || filesProp.ValueKind != JsonValueKind.Array)
            {
                return false;
            }

            if (doc.RootElement.TryGetProperty("ue4ss_version", out var versionProp)
                && versionProp.ValueKind == JsonValueKind.String)
            {
                version = versionProp.GetString();
            }

            return true;
        }
        catch
        {
            return false;
        }
    }

    /// <summary>
    /// Disables and removes the temporary FfrUsmapDump mod and completion marker without touching
    /// mods.txt, UE4SS core files, or any other mods.
    /// </summary>
    internal static void DisableAndRemoveDumpMod(string gameDirectory, Action<string>? log = null)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return;
        }

        try
        {
            var modDir = GetDumpModDirectory(gameDirectory);
            var enabledTxt = Path.Combine(modDir, EnabledTxtFileName);
            var ioEnabled = PathSupport.ToIoPath(enabledTxt);
            if (File.Exists(ioEnabled))
            {
                try
                {
                    File.Delete(ioEnabled);
                }
                catch (Exception exception)
                {
                    log?.Invoke($"警告：删除 {enabledTxt} 失败：{exception.Message}");
                }
            }

            var ioModDir = PathSupport.ToIoPath(modDir);
            if (Directory.Exists(ioModDir))
            {
                try
                {
                    Directory.Delete(ioModDir, recursive: true);
                }
                catch (Exception exception)
                {
                    log?.Invoke($"警告：清理临时转储模组目录失败（{modDir}）：{exception.Message}");
                }
            }

            var donePath = GetDoneMarkerPath(gameDirectory);
            TryDeleteFileQuietly(donePath);
            TryDeleteFileQuietly(donePath + ".tmp");
        }
        catch (Exception exception)
        {
            log?.Invoke($"警告：停用临时转储模组时出错：{exception.Message}");
        }
    }

    internal static bool IsGameProcessRunning()
    {
        var processes = Process.GetProcessesByName(GameProcessName);
        try
        {
            return processes.Length > 0;
        }
        finally
        {
            foreach (var process in processes)
            {
                process.Dispose();
            }
        }
    }

    /// <summary>
    /// Attempts graceful shutdown via CloseMainWindow first, waits up to gracefulWait (15s by default),
    /// then terminates any remaining FFRS-Win64-Shipping processes.
    /// </summary>
    internal static async Task CloseGameProcessesAsync(
        TimeSpan gracefulWait,
        Action<string>? log = null,
        CancellationToken cancellationToken = default)
    {
        var processes = Process.GetProcessesByName(GameProcessName);
        if (processes.Length == 0)
        {
            return;
        }

        try
        {
            log?.Invoke($"正在请求关闭游戏进程（{GameExeFileName}，共 {processes.Length} 个）……");
            foreach (var process in processes)
            {
                try
                {
                    if (!process.HasExited)
                    {
                        _ = process.CloseMainWindow();
                    }
                }
                catch
                {
                    // Ignore races where the process exited or has no main window yet.
                }
            }

            var deadline = DateTimeOffset.UtcNow + gracefulWait;
            while (DateTimeOffset.UtcNow < deadline)
            {
                if (AllExited(processes))
                {
                    log?.Invoke("游戏窗口已正常退出。");
                    return;
                }

                try
                {
                    await Task.Delay(TimeSpan.FromMilliseconds(250), cancellationToken).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                    break;
                }
            }

            foreach (var process in processes)
            {
                try
                {
                    if (!process.HasExited)
                    {
                        log?.Invoke($"游戏在 {Math.Ceiling(gracefulWait.TotalSeconds)} 秒内未退出，正在强制结束进程（PID {process.Id}）……");
                        process.Kill(entireProcessTree: true);
                        process.WaitForExit(5000);
                    }
                }
                catch
                {
                    // Ignore races where the process exited between HasExited and Kill.
                }
            }
        }
        finally
        {
            foreach (var process in processes)
            {
                process.Dispose();
            }
        }
    }

    private static bool AllExited(Process[] processes)
    {
        foreach (var process in processes)
        {
            try
            {
                if (!process.HasExited)
                {
                    return false;
                }
            }
            catch
            {
                // Treat inaccessible or already-terminated handles as exited.
            }
        }

        return true;
    }

    private static void TryDeleteFileQuietly(string path)
    {
        try
        {
            var ioPath = PathSupport.ToIoPath(path);
            if (File.Exists(ioPath))
            {
                File.Delete(ioPath);
            }
        }
        catch
        {
            // Ignore cleanup errors for stale temporary files.
        }
    }
}
