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

internal sealed record Ue4ssCleanupPlan(
    Ue4ssInspection Inspection,
    string[] MarkerFilesToDelete,
    string[] MarkerDirectoriesToClean,
    string[] RemnantPathsToDelete,
    string[] ForeignPathsToMove,
    string? MarkerReadError = null)
{
    internal bool IsForeignUe4ss =>
        Inspection.State is Ue4ssInstallState.InstalledByUser or Ue4ssInstallState.PartialUnknown;

    internal bool HasOurManagedInstall =>
        Inspection.State == Ue4ssInstallState.InstalledByFfrExport && Inspection.HasValidMarker;

    internal bool HasRemnants => RemnantPathsToDelete.Length > 0;

    internal bool HasForeignItems => ForeignPathsToMove.Length > 0;

    internal bool HasAnythingDetected =>
        HasOurManagedInstall
        || HasRemnants
        || HasForeignItems
        || Inspection.State != Ue4ssInstallState.NotInstalled
        || File.Exists(PathSupport.ToIoPath(Inspection.MarkerPath));
}

/// <summary>
/// Shared UE4SS runtime inspection, cleanup preview, temporary mod cleanup, and game process lifecycle helpers
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

    /// <summary>
    /// Builds a read-only preview of all files and directories that would be deleted or moved
    /// during UE4SS cleanup, used by both the GUI confirmation dialog and CLI preview.
    /// </summary>
    internal static Ue4ssCleanupPlan InspectCleanupPlan(string gameDirectory)
    {
        var inspection = InspectUe4ssState(gameDirectory);
        var remnants = CollectOurRemnantRelativePaths(inspection.Win64Directory, inspection.Ue4ssDirectory);
        var markerFiles = new List<string>();
        var markerDirs = new List<string>();
        string? markerReadError = null;

        if (inspection.State == Ue4ssInstallState.InstalledByFfrExport && inspection.HasValidMarker)
        {
            if (TryReadMarkerManifestPaths(inspection.MarkerPath, out var rawFiles, out var rawDirs, out var err))
            {
                foreach (var relFile in rawFiles)
                {
                    if (TryResolveSafeRelativeUnderWin64(inspection.Win64Directory, relFile, out var fullFile)
                        && File.Exists(PathSupport.ToIoPath(fullFile)))
                    {
                        markerFiles.Add(relFile.Replace('\\', '/'));
                    }
                }

                var markerRel = $"ue4ss/{MarkerFileName}";
                if (File.Exists(PathSupport.ToIoPath(inspection.MarkerPath))
                    && !markerFiles.Contains(markerRel, StringComparer.OrdinalIgnoreCase))
                {
                    markerFiles.Add(markerRel);
                }

                foreach (var relDir in rawDirs)
                {
                    if (TryResolveSafeRelativeUnderWin64(inspection.Win64Directory, relDir, out var fullDir)
                        && Directory.Exists(PathSupport.ToIoPath(fullDir)))
                    {
                        markerDirs.Add(relDir.Replace('\\', '/') + "/");
                    }
                }
            }
            else
            {
                markerReadError = err;
            }
        }

        var foreignItems = new List<string>();
        if (inspection.State is Ue4ssInstallState.InstalledByUser or Ue4ssInstallState.PartialUnknown)
        {
            if (inspection.HasDwmapiDll)
            {
                foreignItems.Add(DwmapiFileName);
            }

            if (Directory.Exists(PathSupport.ToIoPath(inspection.Ue4ssDirectory)))
            {
                foreignItems.Add("ue4ss/");
            }
        }

        return new Ue4ssCleanupPlan(
            inspection,
            [.. markerFiles.Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(p => p, StringComparer.OrdinalIgnoreCase)],
            [.. markerDirs.Distinct(StringComparer.OrdinalIgnoreCase).OrderBy(p => p, StringComparer.OrdinalIgnoreCase)],
            remnants,
            [.. foreignItems],
            markerReadError);
    }

    internal static string FormatCleanupPreviewText(
        Ue4ssCleanupPlan plan,
        bool includeForeign,
        string backupRootPreview)
    {
        ArgumentNullException.ThrowIfNull(plan);

        var sb = new StringBuilder();
        sb.AppendLine($"游戏 Win64 目录：{plan.Inspection.Win64Directory}");
        sb.AppendLine();

        if (plan.HasOurManagedInstall)
        {
            sb.AppendLine("【将删除本工具安装的 UE4SS 文件】");
            if (plan.MarkerFilesToDelete.Length == 0)
            {
                sb.AppendLine("  （清单中记录的文件已不存在）");
            }
            else
            {
                foreach (var file in plan.MarkerFilesToDelete)
                {
                    sb.AppendLine($"  [删除] {file}");
                }
            }

            if (plan.MarkerDirectoriesToClean.Length > 0)
            {
                sb.AppendLine();
                sb.AppendLine("【若变空将清理的目录】");
                foreach (var dir in plan.MarkerDirectoriesToClean)
                {
                    sb.AppendLine($"  [空目录清理] {dir}");
                }
            }

            if (plan.HasRemnants)
            {
                sb.AppendLine();
                sb.AppendLine("【将同时清理本工具残留】");
                foreach (var remnant in plan.RemnantPathsToDelete)
                {
                    sb.AppendLine($"  [删除残留] {remnant}");
                }
            }

            return sb.ToString().TrimEnd();
        }

        if (plan.IsForeignUe4ss)
        {
            sb.AppendLine("【本工具残留文件（将删除）】");
            if (plan.HasRemnants)
            {
                foreach (var remnant in plan.RemnantPathsToDelete)
                {
                    sb.AppendLine($"  [删除残留] {remnant}");
                }
            }
            else
            {
                sb.AppendLine("  （未发现本工具残留文件）");
            }

            sb.AppendLine();
            if (includeForeign)
            {
                sb.AppendLine($"【非本工具安装的 UE4SS（不删除，将移动到备份目录：{backupRootPreview}）】");
                foreach (var item in plan.ForeignPathsToMove)
                {
                    sb.AppendLine($"  [移动到备份] {item}");
                }

                sb.AppendLine();
                sb.AppendLine("恢复方法：只需将备份目录内的 dwmapi.dll 和 ue4ss 文件夹复制回上述 Win64 目录即可恢复。");
            }
            else
            {
                sb.AppendLine("【非本工具安装的 UE4SS（默认保持不动）】");
                foreach (var item in plan.ForeignPathsToMove)
                {
                    sb.AppendLine($"  [保留] {item}");
                }
            }

            return sb.ToString().TrimEnd();
        }

        if (plan.HasRemnants)
        {
            sb.AppendLine("【将清理本工具残留文件】");
            foreach (var remnant in plan.RemnantPathsToDelete)
            {
                sb.AppendLine($"  [删除残留] {remnant}");
            }

            return sb.ToString().TrimEnd();
        }

        return "未发现 UE4SS 或本工具残留文件。";
    }

    internal static string[] CollectOurRemnantRelativePaths(string win64Dir, string ue4ssDir)
    {
        var items = new List<string>();
        var dumpModDir = Path.Combine(ue4ssDir, "Mods", DumpModName);
        if (Directory.Exists(PathSupport.ToIoPath(dumpModDir)))
        {
            items.Add($"ue4ss/Mods/{DumpModName}/");
        }

        var donePath = Path.Combine(ue4ssDir, DoneMarkerFileName);
        if (File.Exists(PathSupport.ToIoPath(donePath)))
        {
            items.Add($"ue4ss/{DoneMarkerFileName}");
        }

        var ioUe4ssDir = PathSupport.ToIoPath(ue4ssDir);
        if (Directory.Exists(ioUe4ssDir))
        {
            try
            {
                foreach (var rawFile in Directory.EnumerateFiles(ioUe4ssDir, "*.tmp", SearchOption.TopDirectoryOnly))
                {
                    var fileName = Path.GetFileName(rawFile);
                    if (!string.IsNullOrEmpty(fileName))
                    {
                        items.Add($"ue4ss/{fileName}");
                    }
                }
            }
            catch
            {
                // Ignore enumeration failures during best-effort preview.
            }
        }

        return items
            .Distinct(StringComparer.OrdinalIgnoreCase)
            .OrderBy(p => p, StringComparer.OrdinalIgnoreCase)
            .ToArray();
    }

    /// <summary>
    /// Cleans all remnants created by ffr-export inside the game directory:
    /// ue4ss/Mods/FfrUsmapDump/, ue4ss/ffr-export-usmap.done, and *.tmp files directly inside ue4ss/.
    /// </summary>
    internal static (int RemovedFiles, int RemovedDirs) CleanOurRemnants(
        string gameDirectory,
        bool removeEmptyParentDirs = false,
        Action<string>? log = null)
    {
        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            return (0, 0);
        }

        var removedFiles = 0;
        var removedDirs = 0;
        var ue4ssDir = GetUe4ssDirectory(gameDirectory);
        var modsDir = Path.Combine(ue4ssDir, "Mods");
        var dumpModDir = Path.Combine(modsDir, DumpModName);

        var ioDumpModDir = PathSupport.ToIoPath(dumpModDir);
        if (Directory.Exists(ioDumpModDir))
        {
            try
            {
                removedFiles += Directory.EnumerateFiles(ioDumpModDir, "*", SearchOption.AllDirectories).Count();
                removedDirs += 1 + Directory.EnumerateDirectories(ioDumpModDir, "*", SearchOption.AllDirectories).Count();
                Directory.Delete(ioDumpModDir, recursive: true);
            }
            catch (Exception exception)
            {
                log?.Invoke($"警告：清理临时转储模组目录失败（{dumpModDir}）：{exception.Message}");
            }
        }

        var donePath = Path.Combine(ue4ssDir, DoneMarkerFileName);
        if (TryDeleteExistingFile(donePath, log))
        {
            removedFiles++;
        }

        var ioUe4ssDir = PathSupport.ToIoPath(ue4ssDir);
        if (Directory.Exists(ioUe4ssDir))
        {
            try
            {
                foreach (var tmpFile in Directory.EnumerateFiles(ioUe4ssDir, "*.tmp", SearchOption.TopDirectoryOnly).ToArray())
                {
                    if (TryDeleteExistingFile(tmpFile, log))
                    {
                        removedFiles++;
                    }
                }
            }
            catch (Exception exception)
            {
                log?.Invoke($"警告：扫描临时 .tmp 文件失败（{ue4ssDir}）：{exception.Message}");
            }
        }

        if (removeEmptyParentDirs)
        {
            if (TryDeleteEmptyDirectory(modsDir))
            {
                removedDirs++;
            }

            if (TryDeleteEmptyDirectory(ue4ssDir))
            {
                removedDirs++;
            }
        }

        return (removedFiles, removedDirs);
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

    private static bool TryReadMarkerManifestPaths(
        string markerPath,
        out List<string> files,
        out List<string> directories,
        out string? errorMessage)
    {
        files = [];
        directories = [];
        errorMessage = null;
        try
        {
            var json = File.ReadAllText(PathSupport.ToIoPath(markerPath), Utf8WithoutBom);
            using var doc = JsonDocument.Parse(json);
            if (doc.RootElement.TryGetProperty("files", out var filesProp)
                && filesProp.ValueKind == JsonValueKind.Array)
            {
                foreach (var item in filesProp.EnumerateArray())
                {
                    if (item.ValueKind == JsonValueKind.String)
                    {
                        var val = item.GetString();
                        if (!string.IsNullOrWhiteSpace(val))
                        {
                            files.Add(val);
                        }
                    }
                }
            }

            if (doc.RootElement.TryGetProperty("directories", out var dirsProp)
                && dirsProp.ValueKind == JsonValueKind.Array)
            {
                foreach (var item in dirsProp.EnumerateArray())
                {
                    if (item.ValueKind == JsonValueKind.String)
                    {
                        var val = item.GetString();
                        if (!string.IsNullOrWhiteSpace(val))
                        {
                            directories.Add(val);
                        }
                    }
                }
            }

            return true;
        }
        catch (Exception exception)
        {
            errorMessage = exception.Message;
            return false;
        }
    }

    private static bool TryResolveSafeRelativeUnderWin64(string win64Dir, string rawRelative, out string fullPath)
    {
        fullPath = string.Empty;
        if (string.IsNullOrWhiteSpace(rawRelative))
        {
            return false;
        }

        var normalized = rawRelative.Replace('\\', '/');
        if (normalized.StartsWith('/')
            || normalized.Contains('\0', StringComparison.Ordinal)
            || (normalized.Length >= 2 && normalized[1] == ':'))
        {
            return false;
        }

        foreach (var segment in normalized.Split('/'))
        {
            if (string.IsNullOrEmpty(segment) || segment == "." || segment == "..")
            {
                return false;
            }
        }

        try
        {
            var fullWin64 = PathSupport.TrimTrailingSeparators(Path.GetFullPath(win64Dir));
            var combined = Path.GetFullPath(SteamLocator.CombineRelativeSubpath(fullWin64, normalized));
            if (!PathSupport.IsSameOrDescendantPath(fullWin64, combined))
            {
                return false;
            }

            fullPath = combined;
            return true;
        }
        catch
        {
            return false;
        }
    }

    private static bool TryDeleteExistingFile(string path, Action<string>? log)
    {
        try
        {
            var ioPath = PathSupport.ToIoPath(path);
            if (!File.Exists(ioPath))
            {
                return false;
            }

            File.Delete(ioPath);
            return true;
        }
        catch (Exception exception)
        {
            log?.Invoke($"警告：删除残留文件失败（{path}）：{exception.Message}");
            return false;
        }
    }

    private static bool TryDeleteEmptyDirectory(string dirPath)
    {
        try
        {
            var ioDir = PathSupport.ToIoPath(dirPath);
            if (Directory.Exists(ioDir) && !Directory.EnumerateFileSystemEntries(ioDir).Any())
            {
                Directory.Delete(ioDir, recursive: false);
                return true;
            }
        }
        catch
        {
            // Ignore locked or non-empty directory cleanup failures.
        }

        return false;
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
