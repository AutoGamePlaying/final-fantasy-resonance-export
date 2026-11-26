using System.Diagnostics;
using System.Globalization;
using System.IO.Compression;
using System.Text;
using System.Text.RegularExpressions;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal sealed record FetchUsmapOptions(
    string GameDirectory,
    int TimeoutSeconds,
    bool Yes);

internal sealed record UninstallUe4ssOptions(
    string GameDirectory,
    bool IncludeForeign = false,
    bool Yes = false);

internal enum Ue4ssUninstallOutcome
{
    NotFound,
    RemovedOwnInstall,
    RemovedRemnantsOnly,
    BackedUpForeignInstall,
    Cancelled,
}

internal sealed record Ue4ssUninstallResult(
    Ue4ssUninstallOutcome Outcome,
    int RemovedFiles,
    int RemovedDirs,
    int RemainingFilesInUe4ss,
    string? BackupDirectory);

internal sealed record Ue4ssInstallMarker(
    [property: JsonProperty("format")] int Format,
    [property: JsonProperty("installed_by")] string InstalledBy,
    [property: JsonProperty("ue4ss_version")] string Ue4ssVersion,
    [property: JsonProperty("source_url")] string SourceUrl,
    [property: JsonProperty("archive_sha256")] string ArchiveSha256,
    [property: JsonProperty("installed_at")] string InstalledAt,
    [property: JsonProperty("files")] string[] Files,
    [property: JsonProperty("directories")] string[] Directories);

internal sealed record FetchUsmapResult(
    string ArchivedUsmapPath,
    string SourceUsmapPath,
    UsmapHashInfo UsmapHashes,
    UsmapComparisonResult KnownBuildComparison,
    bool InstalledUe4ss);

internal interface IGameLauncher
{
    bool IsGameRunning();

    void LaunchGameViaSteam(string steamUri);

    Task CloseGameAsync(TimeSpan gracefulWait, Action<string> log, CancellationToken cancellationToken);
}

internal sealed class WindowsSteamGameLauncher : IGameLauncher
{
    public bool IsGameRunning()
    {
        return Ue4ssRuntimeSupport.IsGameProcessRunning();
    }

    public void LaunchGameViaSteam(string steamUri)
    {
        try
        {
            using var process = Process.Start(new ProcessStartInfo
            {
                FileName = steamUri,
                UseShellExecute = true,
            });
        }
        catch (Exception exception)
        {
            throw new InvalidOperationException(
                $"通过 Steam 启动游戏失败（URI：{steamUri}）：{exception.Message}；请确认本机已安装并登录 Steam。",
                exception);
        }
    }

    public Task CloseGameAsync(TimeSpan gracefulWait, Action<string> log, CancellationToken cancellationToken)
    {
        return Ue4ssRuntimeSupport.CloseGameProcessesAsync(gracefulWait, log, cancellationToken);
    }
}

/// <summary>
/// Best-effort CLI log file under %LOCALAPPDATA%\agp\ffr-export\logs\cli-&lt;yyyyMMdd-HHmmss&gt;.log
/// retaining only the latest 10 log files.
/// </summary>
internal sealed class CliLogWriter : TextWriter
{
    private const int DefaultMaxRetainedLogs = 10;
    private static readonly Regex ManagedLogRegex = new(
        @"^(?:gui|cli)-\d{8}-\d{6}(?:-[0-9A-Za-z-]+)?\.log$",
        RegexOptions.IgnoreCase | RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private readonly TextWriter _console;
    private readonly StreamWriter? _fileWriter;
    private readonly object _lock = new();

    private CliLogWriter(TextWriter console, StreamWriter? fileWriter, string? logFilePath)
    {
        _console = console;
        _fileWriter = fileWriter;
        LogFilePath = logFilePath;
    }

    internal string? LogFilePath { get; }

    public override Encoding Encoding => _console.Encoding;

    internal static CliLogWriter Create(TextWriter console, string? logsDirectoryOverride = null)
    {
        StreamWriter? fileWriter = null;
        string? logPath = null;
        try
        {
            var logsDir = string.IsNullOrWhiteSpace(logsDirectoryOverride)
                ? AgpPaths.GetLogsDirectory()
                : PathSupport.TrimTrailingSeparators(Path.GetFullPath(logsDirectoryOverride));
            Directory.CreateDirectory(PathSupport.ToIoPath(logsDir));

            var now = DateTimeOffset.Now;
            var stamp = now.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
            for (var attempt = 0; attempt < 50; attempt++)
            {
                var fileName = attempt == 0
                    ? $"cli-{stamp}.log"
                    : string.Create(CultureInfo.InvariantCulture, $"cli-{stamp}-{attempt + 1}.log");
                var candidate = Path.Combine(logsDir, fileName);
                try
                {
                    fileWriter = TextOutput.CreateNewFileStreamWriter(candidate, FileShare.ReadWrite, autoFlush: true);
                    logPath = candidate;
                    break;
                }
                catch (IOException) when (File.Exists(PathSupport.ToIoPath(candidate)))
                {
                    // Same-second collision; try next suffix.
                }
            }

            if (logPath is not null)
            {
                PruneOldLogs(logsDir, logPath, DefaultMaxRetainedLogs);
            }
        }
        catch
        {
            // Logging to disk is best-effort and must never fail CLI execution.
            fileWriter?.Dispose();
            fileWriter = null;
            logPath = null;
        }

        return new CliLogWriter(console, fileWriter, logPath);
    }

    public override void Write(char value)
    {
        lock (_lock)
        {
            _console.Write(value);
            try
            {
                _fileWriter?.Write(value);
            }
            catch
            {
                // Ignore disk log write errors.
            }
        }
    }

    public override void Write(string? value)
    {
        lock (_lock)
        {
            _console.Write(value);
            try
            {
                _fileWriter?.Write(value is null ? null : TextOutput.NormalizeLineEndings(value));
            }
            catch
            {
                // Ignore disk log write errors.
            }
        }
    }

    public override void WriteLine(string? value)
    {
        lock (_lock)
        {
            _console.WriteLine(value);
            try
            {
                _fileWriter?.WriteLine(value is null ? null : TextOutput.NormalizeLineEndings(value));
            }
            catch
            {
                // Ignore disk log write errors.
            }
        }
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            lock (_lock)
            {
                try
                {
                    _fileWriter?.Dispose();
                }
                catch
                {
                    // Ignore disposal errors.
                }
            }
        }

        base.Dispose(disposing);
    }

    internal static void PruneOldLogs(string logsDir, string? keepFilePath, int maxRetainedLogs)
    {
        if (string.IsNullOrWhiteSpace(logsDir) || maxRetainedLogs <= 0)
        {
            return;
        }

        try
        {
            var ioDir = PathSupport.ToIoPath(logsDir);
            if (!Directory.Exists(ioDir))
            {
                return;
            }

            var normalizedKeep = string.IsNullOrWhiteSpace(keepFilePath)
                ? null
                : NormalizeCompareKey(keepFilePath);

            var entries = new List<(string FullPath, string FileName, DateTime LastWriteUtc, bool IsKeep)>();
            foreach (var raw in Directory.EnumerateFiles(ioDir, "*.log", SearchOption.TopDirectoryOnly))
            {
                var fileName = Path.GetFileName(raw);
                if (!ManagedLogRegex.IsMatch(fileName))
                {
                    continue;
                }

                var fullPath = Path.GetFullPath(raw);
                var writeUtc = DateTime.MinValue;
                try
                {
                    writeUtc = File.GetLastWriteTimeUtc(PathSupport.ToIoPath(fullPath));
                }
                catch
                {
                    // Treat unreadable metadata as oldest.
                }

                var isKeep = normalizedKeep is not null
                    && string.Equals(NormalizeCompareKey(fullPath), normalizedKeep, StringComparison.Ordinal);
                entries.Add((fullPath, fileName, writeUtc, isKeep));
            }

            if (entries.Count <= maxRetainedLogs)
            {
                return;
            }

            var ordered = entries
                .OrderByDescending(e => e.IsKeep)
                .ThenByDescending(e => e.LastWriteUtc)
                .ThenByDescending(e => e.FileName, StringComparer.OrdinalIgnoreCase)
                .ToList();

            for (var i = maxRetainedLogs; i < ordered.Count; i++)
            {
                if (ordered[i].IsKeep)
                {
                    continue;
                }

                try
                {
                    File.Delete(PathSupport.ToIoPath(ordered[i].FullPath));
                }
                catch
                {
                    // Ignore locked log files.
                }
            }
        }
        catch
        {
            // Pruning is best-effort.
        }
    }

    private static string NormalizeCompareKey(string path)
    {
        var full = PathSupport.TrimTrailingSeparators(Path.GetFullPath(path));
        return OperatingSystem.IsWindows() ? full.ToUpperInvariant() : full;
    }
}

/// <summary>
/// Manages deploying, enabling, and removing the temporary FfrUsmapDump Lua mod.
/// UE4SS v4.0.0 natively loads mods with an enabled.txt marker file without editing mods.txt.
/// </summary>
internal static class Ue4ssModManager
{
    internal const string EmbeddedMainLuaResourceName = "FfrExport.FfrUsmapDump.main.lua";

    internal static string LoadEmbeddedMainLua()
    {
        var assembly = typeof(Ue4ssModManager).Assembly;
        using var stream = assembly.GetManifestResourceStream(EmbeddedMainLuaResourceName);
        if (stream is null)
        {
            throw new InvalidOperationException($"程序内缺少嵌入的转储模组脚本资源：{EmbeddedMainLuaResourceName}");
        }

        using var reader = new StreamReader(stream, Encoding.UTF8);
        return reader.ReadToEnd();
    }

    /// <summary>
    /// Deploys ue4ss/Mods/FfrUsmapDump/Scripts/main.lua and enables it via enabled.txt
    /// without touching mods.txt.
    /// </summary>
    internal static string DeployTemporaryDumpMod(string gameDirectory)
    {
        var modDir = Ue4ssRuntimeSupport.GetDumpModDirectory(gameDirectory);
        var scriptsDir = Path.Combine(modDir, "Scripts");
        var mainLuaPath = Path.Combine(scriptsDir, "main.lua");
        var enabledTxtPath = Path.Combine(modDir, Ue4ssRuntimeSupport.EnabledTxtFileName);
        var donePath = Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDirectory);

        try
        {
            var ioDone = PathSupport.ToIoPath(donePath);
            if (File.Exists(ioDone))
            {
                File.Delete(ioDone);
            }

            var ioDoneTmp = PathSupport.ToIoPath(donePath + ".tmp");
            if (File.Exists(ioDoneTmp))
            {
                File.Delete(ioDoneTmp);
            }
        }
        catch (Exception exception)
        {
            throw new IOException($"清理旧的转储完成标记失败（{donePath}）：{exception.Message}", exception);
        }

        var luaContent = LoadEmbeddedMainLua();
        Directory.CreateDirectory(PathSupport.ToIoPath(scriptsDir));
        TextOutput.WriteText(mainLuaPath, luaContent);
        TextOutput.WriteText(enabledTxtPath, string.Empty);
        return modDir;
    }

    /// <summary>
    /// Fallback helper that appends a single mod enable line to mods.txt while preserving all existing
    /// lines, comments, blank lines, and line endings verbatim.
    /// Not used for UE4SS v4.0.0 because v4.0.0 natively supports enabled.txt.
    /// </summary>
    internal static string AppendModEntryPreservingModsTxt(string existingContent, string modName, bool enabled)
    {
        ArgumentNullException.ThrowIfNull(existingContent);
        if (string.IsNullOrWhiteSpace(modName))
        {
            throw new ArgumentException("模组名不能为空", nameof(modName));
        }

        var trimmedName = modName.Trim();
        var value = enabled ? "1" : "0";
        var lineEnding = existingContent.Contains("\r\n", StringComparison.Ordinal) ? "\r\n" : "\n";

        var lines = existingContent.Length == 0
            ? new List<string>()
            : SplitLinesPreservingEmpty(existingContent);
        var hadTrailingNewline = existingContent.EndsWith('\n');

        var entryRegex = new Regex(
            $@"^(\s*){Regex.Escape(trimmedName)}(\s*:\s*)[01](\s*(?:;.*)?)$",
            RegexOptions.IgnoreCase | RegexOptions.CultureInvariant);

        var updated = false;
        for (var i = 0; i < lines.Count; i++)
        {
            var match = entryRegex.Match(lines[i]);
            if (match.Success)
            {
                lines[i] = $"{match.Groups[1].Value}{trimmedName}{match.Groups[2].Value}{value}{match.Groups[3].Value}";
                updated = true;
                break;
            }
        }

        if (!updated)
        {
            lines.Add($"{trimmedName} : {value}");
        }

        var joined = string.Join(lineEnding, lines);
        return hadTrailingNewline || existingContent.Length == 0 ? joined + lineEnding : joined;
    }

    private static List<string> SplitLinesPreservingEmpty(string text)
    {
        var normalized = text.Replace("\r\n", "\n", StringComparison.Ordinal);
        if (normalized.EndsWith('\n'))
        {
            normalized = normalized[..^1];
        }

        return [.. normalized.Split('\n')];
    }
}

/// <summary>
/// Transactional installer and marker-verified uninstaller for the official UE4SS v4.0.0 archive.
/// </summary>
internal static class Ue4ssArchiveInstaller
{
    private static readonly UTF8Encoding Utf8WithoutBom = TextOutput.Utf8WithoutBom;

    internal static Ue4ssInstallMarker InstallFromVerifiedZip(
        string zipPath,
        string gameDirectory,
        string archiveSha256Lower,
        string ue4ssVersion = HttpsUe4ssPackageDownloader.Ue4ssVersion,
        string sourceUrl = HttpsUe4ssPackageDownloader.OfficialDownloadUrl,
        Action<string>? beforeFileWriteHook = null,
        DateTimeOffset? installedAtUtc = null)
    {
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDirectory);
        if (!Directory.Exists(PathSupport.ToIoPath(win64Dir)))
        {
            throw new DirectoryNotFoundException($"安装 UE4SS 失败：Win64 目录不存在：{win64Dir}");
        }

        using var archive = ZipFile.OpenRead(PathSupport.ToIoPath(zipPath));
        var plannedEntries = PlanZipExtraction(archive, zipPath);

        // Refuse to overwrite any pre-existing files in Win64 before writing anything.
        foreach (var entry in plannedEntries)
        {
            var destination = CombineUnderWin64(win64Dir, entry.RelativePath);
            if (File.Exists(PathSupport.ToIoPath(destination)))
            {
                throw new IOException(
                    $"安装 UE4SS 失败：目标位置已存在同名文件（{destination}），拒绝覆盖；" +
                    "如已自行安装其他模组或代理 DLL，请先手动备份或清理后重试。");
            }
        }

        var writtenFiles = new List<string>();
        var writtenRelFiles = new List<string>();
        var createdDirs = new List<string>();
        var createdRelDirs = new HashSet<string>(StringComparer.Ordinal);

        try
        {
            foreach (var entry in plannedEntries)
            {
                var destination = CombineUnderWin64(win64Dir, entry.RelativePath);
                EnsureDirectoryTracked(win64Dir, Path.GetDirectoryName(destination)!, createdDirs, createdRelDirs);

                beforeFileWriteHook?.Invoke(entry.RelativePath);

                using (var sourceStream = entry.ZipEntry.Open())
                using (var targetStream = new FileStream(
                    PathSupport.ToIoPath(destination),
                    FileMode.CreateNew,
                    FileAccess.Write,
                    FileShare.None))
                {
                    sourceStream.CopyTo(targetStream);
                }

                writtenFiles.Add(destination);
                writtenRelFiles.Add(entry.RelativePath);
            }

            // Patch UE4SS-settings.ini for our managed install: disable external console and GUI debug window.
            var settingsIniRel = "ue4ss/UE4SS-settings.ini";
            var settingsIniPath = CombineUnderWin64(win64Dir, settingsIniRel);
            if (File.Exists(PathSupport.ToIoPath(settingsIniPath)))
            {
                var originalIni = File.ReadAllText(PathSupport.ToIoPath(settingsIniPath), Encoding.UTF8);
                var patchedIni = PatchUe4ssSettingsIni(originalIni);
                TextOutput.WriteText(settingsIniPath, patchedIni);
            }

            var markerRel = $"ue4ss/{Ue4ssRuntimeSupport.MarkerFileName}";
            var markerPath = CombineUnderWin64(win64Dir, markerRel);
            EnsureDirectoryTracked(win64Dir, Path.GetDirectoryName(markerPath)!, createdDirs, createdRelDirs);

            var recordedFiles = new SortedSet<string>(writtenRelFiles, StringComparer.Ordinal)
            {
                markerRel,
            };
            var recordedDirs = new SortedSet<string>(createdRelDirs, StringComparer.Ordinal);

            var marker = new Ue4ssInstallMarker(
                Format: 1,
                InstalledBy: Ue4ssRuntimeSupport.InstalledByIdentifier,
                Ue4ssVersion: ue4ssVersion,
                SourceUrl: sourceUrl,
                ArchiveSha256: archiveSha256Lower.ToLowerInvariant(),
                InstalledAt: (installedAtUtc ?? DateTimeOffset.UtcNow).UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
                Files: [.. recordedFiles],
                Directories: [.. recordedDirs]);

            var markerJson = TextOutput.SerializeJson(marker, trailingNewline: true);
            beforeFileWriteHook?.Invoke(markerRel);
            TextOutput.WriteText(markerPath, markerJson);
            writtenFiles.Add(markerPath);

            return marker;
        }
        catch
        {
            RollbackWrittenFiles(writtenFiles, createdDirs);
            throw;
        }
    }

    /// <summary>
    /// Disables the external console and debug GUI window in UE4SS v4.0.0's UE4SS-settings.ini,
    /// and applies stable FFRS engine settings when those sections exist.
    /// </summary>
    internal static string PatchUe4ssSettingsIni(string iniContent)
    {
        ArgumentNullException.ThrowIfNull(iniContent);

        var lineEnding = iniContent.Contains("\r\n", StringComparison.Ordinal) ? "\r\n" : "\n";
        var hadTrailingNewline = iniContent.EndsWith('\n');
        var rawNormalized = iniContent.Replace("\r\n", "\n", StringComparison.Ordinal);
        if (hadTrailingNewline)
        {
            rawNormalized = rawNormalized[..^1];
        }

        var lines = rawNormalized.Length == 0
            ? new List<string>()
            : rawNormalized.Split('\n').ToList();

        // Always disable console / debug GUI windows in [Debug] (v4.0.0 keys: ConsoleEnabled, GuiConsoleEnabled, GuiConsoleVisible).
        ApplyIniSectionOverrides(
            lines,
            "Debug",
            new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                ["ConsoleEnabled"] = "0",
                ["GuiConsoleEnabled"] = "0",
                ["GuiConsoleVisible"] = "0",
            },
            createSectionIfMissing: true);

        // Apply verified stable FFRS settings when the upstream sections exist.
        ApplyIniSectionOverrides(
            lines,
            "General",
            new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                ["EnableHotReloadSystem"] = "0",
                ["bUseUObjectArrayCache"] = "false",
            },
            createSectionIfMissing: false);

        ApplyIniSectionOverrides(
            lines,
            "EngineVersionOverride",
            new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase)
            {
                ["MajorVersion"] = "5",
                ["MinorVersion"] = "6",
            },
            createSectionIfMissing: false);

        var result = string.Join(lineEnding, lines);
        return hadTrailingNewline || result.Length > 0 ? result + lineEnding : result;
    }

    internal static Ue4ssUninstallResult Uninstall(
        string gameDirectory,
        IGameLauncher launcher,
        TextWriter output,
        bool includeForeign = false,
        bool yes = false,
        TextReader? input = null,
        string? agpHomeOverride = null,
        Func<DateTimeOffset>? nowOverride = null)
    {
        var gamePaths = GameVersionDetector.ValidateGameDirectory(gameDirectory);
        if (launcher.IsGameRunning())
        {
            throw new InvalidOperationException(
                $"清理 UE4SS 被拒绝：游戏进程（{Ue4ssRuntimeSupport.GameExeFileName}）正在运行，请先关闭游戏后重试。");
        }

        var inspection = Ue4ssRuntimeSupport.InspectUe4ssState(gamePaths.GameRoot);
        var markerExists = File.Exists(PathSupport.ToIoPath(inspection.MarkerPath));

        // Case 1: Installed by ffr-export (marker file exists — validate strictly against path traversal).
        if (markerExists)
        {
            var marker = ReadAndValidateMarker(inspection.MarkerPath, inspection.Win64Directory);

            var (remnantFiles, remnantDirs) = Ue4ssRuntimeSupport.CleanOurRemnants(
                gamePaths.GameRoot,
                removeEmptyParentDirs: false,
                log: message => output.WriteLine(message));

            var removedFiles = remnantFiles;
            foreach (var relativeFile in marker.Files)
            {
                var fullPath = CombineUnderWin64(inspection.Win64Directory, relativeFile);
                var ioPath = PathSupport.ToIoPath(fullPath);
                if (File.Exists(ioPath))
                {
                    File.Delete(ioPath);
                    removedFiles++;
                }
            }

            // Ensure the marker file itself is removed even if an older marker omitted its own path.
            var ioMarker = PathSupport.ToIoPath(inspection.MarkerPath);
            if (File.Exists(ioMarker))
            {
                File.Delete(ioMarker);
                removedFiles++;
            }

            var removedDirs = remnantDirs;
            var dirsDeepestFirst = marker.Directories
                .Concat(["ue4ss/Mods", "ue4ss"])
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .OrderByDescending(dir => dir.Length)
                .ThenByDescending(dir => dir, StringComparer.Ordinal)
                .ToArray();
            foreach (var relativeDir in dirsDeepestFirst)
            {
                var fullDir = CombineUnderWin64(inspection.Win64Directory, relativeDir);
                var ioDir = PathSupport.ToIoPath(fullDir);
                if (Directory.Exists(ioDir) && !Directory.EnumerateFileSystemEntries(ioDir).Any())
                {
                    Directory.Delete(ioDir, recursive: false);
                    removedDirs++;
                }
            }

            var remaining = CountFilesInDirectory(inspection.Ue4ssDirectory);
            output.WriteLine($"已按安装标记卸载 UE4SS：删除 {removedFiles} 个文件，清理 {removedDirs} 个空目录。");
            if (remaining > 0)
            {
                output.WriteLine(
                    $"提示：{inspection.Ue4ssDirectory} 下仍保留 {remaining} 个非安装清单文件（如 UE4SS.log、用户模组或转储文件），未予删除。");
            }

            output.WriteLine("说明：已复制到 agp\\ffr-export\\usmap\\ 的 .usmap 文件不受影响，仍可继续使用。");
            return new Ue4ssUninstallResult(
                Ue4ssUninstallOutcome.RemovedOwnInstall,
                removedFiles,
                removedDirs,
                remaining,
                BackupDirectory: null);
        }

        // Case 2: Foreign UE4SS installation (dwmapi.dll and/or ue4ss/UE4SS.dll present without our marker).
        if (inspection.HasDwmapiDll || inspection.HasUe4ssDll)
        {
            if (!includeForeign)
            {
                var (remnantFiles, remnantDirs) = Ue4ssRuntimeSupport.CleanOurRemnants(
                    gamePaths.GameRoot,
                    removeEmptyParentDirs: false,
                    log: message => output.WriteLine(message));
                var remaining = CountFilesInDirectory(inspection.Ue4ssDirectory);

                if (remnantFiles > 0 || remnantDirs > 0)
                {
                    output.WriteLine(
                        $"检测到游戏目录中的 UE4SS 不是本工具安装的（未找到 {Ue4ssRuntimeSupport.MarkerFileName}），" +
                        $"已仅清理本工具残留（删除 {remnantFiles} 个文件，{remnantDirs} 个目录），原有 UE4SS 保持不动。");
                }
                else
                {
                    output.WriteLine(
                        $"检测到游戏目录中的 UE4SS 不是本工具安装的（未找到 {Ue4ssRuntimeSupport.MarkerFileName}），" +
                        "未发现本工具残留文件，原有 UE4SS 保持不动。");
                }

                output.WriteLine(
                    "提示：如需连同非本工具安装的 UE4SS 一起移出游戏目录（移动至备份目录，不直接删除），请加上 --include-foreign。");
                return new Ue4ssUninstallResult(
                    Ue4ssUninstallOutcome.RemovedRemnantsOnly,
                    remnantFiles,
                    remnantDirs,
                    remaining,
                    BackupDirectory: null);
            }

            var backupRoot = AgpPaths.GetUe4ssBackupDirectory(agpHomeEnvOverride: agpHomeOverride);
            var now = nowOverride?.Invoke() ?? DateTimeOffset.Now;
            var backupDir = AllocateUniqueBackupDirectoryPath(backupRoot, now);

            if (!yes)
            {
                var plan = Ue4ssRuntimeSupport.InspectCleanupPlan(gamePaths.GameRoot);
                output.WriteLine("警告：当前游戏目录中的 UE4SS 不是本工具安装的，里面可能包含您自行安装的其他模组！");
                output.WriteLine(Ue4ssRuntimeSupport.FormatCleanupPreviewText(plan, includeForeign: true, backupDir));
                output.WriteLine();

                var confirmed = PromptYesNoForCleanup(input ?? TextReader.Null, output);
                if (confirmed is not true)
                {
                    output.WriteLine("已取消清理，未对游戏目录做任何修改。");
                    return new Ue4ssUninstallResult(
                        Ue4ssUninstallOutcome.Cancelled,
                        0,
                        0,
                        CountFilesInDirectory(inspection.Ue4ssDirectory),
                        BackupDirectory: null);
                }
            }

            var (cleanedFiles, cleanedDirs) = Ue4ssRuntimeSupport.CleanOurRemnants(
                gamePaths.GameRoot,
                removeEmptyParentDirs: false,
                log: message => output.WriteLine(message));

            MoveForeignUe4ssToBackup(inspection, backupDir);

            output.WriteLine($"已将非本工具安装的 UE4SS 移出游戏目录并完整备份到：{backupDir}");
            if (cleanedFiles > 0 || cleanedDirs > 0)
            {
                output.WriteLine($"同时已清理本工具残留：{cleanedFiles} 个文件，{cleanedDirs} 个目录。");
            }

            output.WriteLine(
                $"恢复方法：如需恢复原有的 UE4SS 与模组，将 {backupDir} 内的 dwmapi.dll 和 ue4ss 文件夹复制回 {inspection.Win64Directory} 即可。");
            output.WriteLine("说明：已复制到 agp\\ffr-export\\usmap\\ 的 .usmap 文件不受影响，仍可继续使用。");

            return new Ue4ssUninstallResult(
                Ue4ssUninstallOutcome.BackedUpForeignInstall,
                cleanedFiles,
                cleanedDirs,
                RemainingFilesInUe4ss: 0,
                BackupDirectory: backupDir);
        }

        // Case 3: No UE4SS installed; clean any stray remnants of ours if present, otherwise report "未发现 UE4SS".
        var (strayFiles, strayDirs) = Ue4ssRuntimeSupport.CleanOurRemnants(
            gamePaths.GameRoot,
            removeEmptyParentDirs: true,
            log: message => output.WriteLine(message));
        if (strayFiles > 0 || strayDirs > 0)
        {
            output.WriteLine($"未发现完整 UE4SS 安装，已清理本工具残留（删除 {strayFiles} 个文件，{strayDirs} 个目录）。");
            return new Ue4ssUninstallResult(
                Ue4ssUninstallOutcome.RemovedRemnantsOnly,
                strayFiles,
                strayDirs,
                CountFilesInDirectory(inspection.Ue4ssDirectory),
                BackupDirectory: null);
        }

        output.WriteLine("未发现 UE4SS（当前游戏目录未安装 UE4SS，也未发现本工具残留文件）。");
        return new Ue4ssUninstallResult(
            Ue4ssUninstallOutcome.NotFound,
            0,
            0,
            0,
            BackupDirectory: null);
    }

    private static int CountFilesInDirectory(string directoryPath)
    {
        var ioDir = PathSupport.ToIoPath(directoryPath);
        return Directory.Exists(ioDir)
            ? Directory.EnumerateFiles(ioDir, "*", SearchOption.AllDirectories).Count()
            : 0;
    }

    private static string AllocateUniqueBackupDirectoryPath(string backupRoot, DateTimeOffset now)
    {
        var stamp = now.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
        for (var attempt = 0; attempt < 100; attempt++)
        {
            var folderName = attempt == 0
                ? stamp
                : string.Create(CultureInfo.InvariantCulture, $"{stamp}-{attempt + 1}");
            var candidate = Path.Combine(backupRoot, folderName);
            if (!Directory.Exists(PathSupport.ToIoPath(candidate)) && !File.Exists(PathSupport.ToIoPath(candidate)))
            {
                return candidate;
            }
        }

        return Path.Combine(backupRoot, $"{stamp}-{Guid.NewGuid().ToString("N")[..8]}");
    }

    /// <summary>
    /// Moves dwmapi.dll and the entire ue4ss/ folder out of the game directory into backupDir.
    /// Uses copy-verify-then-delete so cross-volume moves always succeed and any failure
    /// leaves the original game directory intact.
    /// </summary>
    private static void MoveForeignUe4ssToBackup(Ue4ssInspection inspection, string backupDir)
    {
        var dwmapiSrc = Path.Combine(inspection.Win64Directory, Ue4ssRuntimeSupport.DwmapiFileName);
        var hasDwmapi = File.Exists(PathSupport.ToIoPath(dwmapiSrc));
        var hasUe4ssDir = Directory.Exists(PathSupport.ToIoPath(inspection.Ue4ssDirectory));

        if (!hasDwmapi && !hasUe4ssDir)
        {
            return;
        }

        Directory.CreateDirectory(PathSupport.ToIoPath(backupDir));
        var copyCompleted = false;
        try
        {
            if (hasDwmapi)
            {
                var dwmapiDst = Path.Combine(backupDir, Ue4ssRuntimeSupport.DwmapiFileName);
                File.Copy(PathSupport.ToIoPath(dwmapiSrc), PathSupport.ToIoPath(dwmapiDst), overwrite: false);
            }

            if (hasUe4ssDir)
            {
                var ue4ssDst = Path.Combine(backupDir, "ue4ss");
                CopyDirectoryRecursive(inspection.Ue4ssDirectory, ue4ssDst);
            }

            copyCompleted = true;
        }
        catch (Exception exception)
        {
            try
            {
                if (Directory.Exists(PathSupport.ToIoPath(backupDir)))
                {
                    Directory.Delete(PathSupport.ToIoPath(backupDir), recursive: true);
                }
            }
            catch
            {
                // Ignore cleanup failures of incomplete backup directory.
            }

            throw new IOException(
                $"备份非本工具安装的 UE4SS 失败（目标备份目录：{backupDir}）：{exception.Message}；游戏目录保持未修改。",
                exception);
        }

        if (!copyCompleted)
        {
            return;
        }

        try
        {
            if (hasDwmapi && File.Exists(PathSupport.ToIoPath(dwmapiSrc)))
            {
                File.Delete(PathSupport.ToIoPath(dwmapiSrc));
            }

            if (hasUe4ssDir && Directory.Exists(PathSupport.ToIoPath(inspection.Ue4ssDirectory)))
            {
                Directory.Delete(PathSupport.ToIoPath(inspection.Ue4ssDirectory), recursive: true);
            }
        }
        catch (Exception exception)
        {
            throw new IOException(
                $"已完成 UE4SS 备份（{backupDir}），但移出原目录文件失败：{exception.Message}；" +
                "请确认游戏及相关工具已关闭后重试，备份目录已完整保留。",
                exception);
        }
    }

    private static void CopyDirectoryRecursive(string sourceDir, string destinationDir)
    {
        Directory.CreateDirectory(PathSupport.ToIoPath(destinationDir));

        foreach (var rawSubdir in Directory.EnumerateDirectories(PathSupport.ToIoPath(sourceDir), "*", SearchOption.AllDirectories))
        {
            var rel = Path.GetRelativePath(sourceDir, Path.GetFullPath(rawSubdir));
            Directory.CreateDirectory(PathSupport.ToIoPath(Path.Combine(destinationDir, rel)));
        }

        foreach (var rawFile in Directory.EnumerateFiles(PathSupport.ToIoPath(sourceDir), "*", SearchOption.AllDirectories))
        {
            var fullSrc = Path.GetFullPath(rawFile);
            var rel = Path.GetRelativePath(sourceDir, fullSrc);
            var dstFile = Path.Combine(destinationDir, rel);
            var dstParent = Path.GetDirectoryName(dstFile);
            if (!string.IsNullOrEmpty(dstParent))
            {
                Directory.CreateDirectory(PathSupport.ToIoPath(dstParent));
            }

            File.Copy(PathSupport.ToIoPath(fullSrc), PathSupport.ToIoPath(dstFile), overwrite: false);
        }
    }

    private static bool? PromptYesNoForCleanup(TextReader input, TextWriter output)
    {
        while (true)
        {
            output.Write("确认将上述非本工具安装的 UE4SS 移动到备份目录吗？（输入 y 确认，直接回车或输入 n 取消）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (trimmed.Equals("y", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("yes", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("是", StringComparison.Ordinal))
            {
                return true;
            }

            if (string.IsNullOrEmpty(trimmed)
                || trimmed.Equals("n", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("no", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("否", StringComparison.Ordinal))
            {
                return false;
            }

            output.WriteLine("输入无效，请输入 y 确认或 n 取消。");
        }
    }

    internal static Ue4ssInstallMarker ReadAndValidateMarker(string markerPath, string win64Dir)
    {
        string rawJson;
        try
        {
            rawJson = File.ReadAllText(PathSupport.ToIoPath(markerPath), Utf8WithoutBom);
        }
        catch (Exception exception)
        {
            throw new IOException($"读取 UE4SS 安装标记文件失败（{markerPath}）：{exception.Message}", exception);
        }

        Ue4ssInstallMarker? marker;
        try
        {
            marker = JsonConvert.DeserializeObject<Ue4ssInstallMarker>(rawJson);
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException($"UE4SS 安装标记文件格式损坏（{markerPath}）：{exception.Message}", exception);
        }

        if (marker is null
            || marker.Format != 1
            || !string.Equals(marker.InstalledBy, Ue4ssRuntimeSupport.InstalledByIdentifier, StringComparison.Ordinal)
            || marker.Files is null
            || marker.Directories is null)
        {
            throw new InvalidDataException($"UE4SS 安装标记文件无效或不受支持（{markerPath}），拒绝卸载。");
        }

        var validatedFiles = new string[marker.Files.Length];
        for (var i = 0; i < marker.Files.Length; i++)
        {
            validatedFiles[i] = ValidateSafeRelativePath(marker.Files[i], "文件", markerPath);
            _ = CombineUnderWin64(win64Dir, validatedFiles[i]);
        }

        var validatedDirs = new string[marker.Directories.Length];
        for (var i = 0; i < marker.Directories.Length; i++)
        {
            validatedDirs[i] = ValidateSafeRelativePath(marker.Directories[i], "目录", markerPath);
            _ = CombineUnderWin64(win64Dir, validatedDirs[i]);
        }

        return marker with { Files = validatedFiles, Directories = validatedDirs };
    }

    private sealed record PlannedZipEntry(string RelativePath, ZipArchiveEntry ZipEntry);

    private static List<PlannedZipEntry> PlanZipExtraction(ZipArchive archive, string zipPath)
    {
        var rawFileEntries = new List<(string NormalizedPath, ZipArchiveEntry Entry)>();
        foreach (var entry in archive.Entries)
        {
            if (string.IsNullOrEmpty(entry.FullName) || entry.FullName.EndsWith('/') || entry.FullName.EndsWith('\\'))
            {
                continue;
            }

            var normalized = ValidateSafeRelativePath(entry.FullName, "压缩包条目", zipPath);
            rawFileEntries.Add((normalized, entry));
        }

        if (rawFileEntries.Count == 0)
        {
            throw new InvalidDataException($"UE4SS 压缩包为空：{zipPath}");
        }

        // Official UE4SS_v4.0.0.zip places dwmapi.dll and ue4ss/ directly at the root.
        // If every entry is wrapped in a single top-level folder containing dwmapi.dll and ue4ss/UE4SS.dll, unwrap it.
        var stripPrefix = string.Empty;
        var hasRootDwmapi = rawFileEntries.Any(e =>
            string.Equals(e.NormalizedPath, "dwmapi.dll", StringComparison.OrdinalIgnoreCase));
        if (!hasRootDwmapi)
        {
            var firstSegments = rawFileEntries
                .Select(e => e.NormalizedPath.Split('/')[0])
                .Distinct(StringComparer.OrdinalIgnoreCase)
                .ToArray();
            if (firstSegments.Length == 1)
            {
                var candidatePrefix = firstSegments[0] + "/";
                var wrappedHasDwmapi = rawFileEntries.Any(e =>
                    string.Equals(e.NormalizedPath, candidatePrefix + "dwmapi.dll", StringComparison.OrdinalIgnoreCase));
                var wrappedHasUe4ss = rawFileEntries.Any(e =>
                    string.Equals(e.NormalizedPath, candidatePrefix + "ue4ss/UE4SS.dll", StringComparison.OrdinalIgnoreCase));
                if (wrappedHasDwmapi && wrappedHasUe4ss)
                {
                    stripPrefix = candidatePrefix;
                }
            }
        }

        var planned = new List<PlannedZipEntry>(rawFileEntries.Count);
        var seen = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        foreach (var (normalizedPath, entry) in rawFileEntries)
        {
            var relative = stripPrefix.Length > 0
                ? normalizedPath[stripPrefix.Length..]
                : normalizedPath;
            relative = ValidateSafeRelativePath(relative, "解压路径", zipPath);
            if (!seen.Add(relative))
            {
                throw new InvalidDataException($"UE4SS 压缩包包含重复条目（{relative}）：{zipPath}");
            }

            planned.Add(new PlannedZipEntry(relative, entry));
        }

        var hasDwmapi = planned.Any(e =>
            string.Equals(e.RelativePath, Ue4ssRuntimeSupport.DwmapiFileName, StringComparison.OrdinalIgnoreCase));
        var hasUe4ssDll = planned.Any(e =>
            string.Equals(e.RelativePath, "ue4ss/UE4SS.dll", StringComparison.OrdinalIgnoreCase));
        if (!hasDwmapi || !hasUe4ssDll)
        {
            throw new InvalidDataException(
                $"UE4SS 压缩包结构不符合预期（缺少 dwmapi.dll 或 ue4ss/UE4SS.dll）：{zipPath}");
        }

        return planned;
    }

    private static string ValidateSafeRelativePath(string rawPath, string label, string source)
    {
        if (string.IsNullOrWhiteSpace(rawPath))
        {
            throw new InvalidDataException($"解析 {label} 失败（{source}）：路径为空");
        }

        var normalized = rawPath.Replace('\\', '/');
        if (normalized.StartsWith('/')
            || normalized.Contains('\0', StringComparison.Ordinal)
            || (normalized.Length >= 2 && normalized[1] == ':'))
        {
            throw new InvalidDataException($"检测到不安全的绝对路径 {label}（{rawPath}，来源：{source}），已拒绝操作。");
        }

        var segments = normalized.Split('/');
        foreach (var segment in segments)
        {
            if (string.IsNullOrEmpty(segment) || segment == "." || segment == "..")
            {
                throw new InvalidDataException($"检测到不安全的相对路径 {label}（{rawPath}，来源：{source}），已拒绝操作。");
            }
        }

        return string.Join('/', segments);
    }

    private static string CombineUnderWin64(string win64Dir, string safeRelativePath)
    {
        var fullWin64 = PathSupport.TrimTrailingSeparators(Path.GetFullPath(win64Dir));
        var combined = SteamLocator.CombineRelativeSubpath(fullWin64, safeRelativePath);
        var fullTarget = Path.GetFullPath(combined);
        if (!PathSupport.IsSameOrDescendantPath(fullWin64, fullTarget)
            || string.Equals(
                PathSupport.TrimTrailingSeparators(fullWin64),
                PathSupport.TrimTrailingSeparators(fullTarget),
                OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
        {
            throw new InvalidDataException($"路径越界（{safeRelativePath}），目标不在 Win64 目录内。");
        }

        return fullTarget;
    }

    private static void EnsureDirectoryTracked(
        string win64Dir,
        string targetDirectory,
        List<string> createdDirs,
        HashSet<string> createdRelDirs)
    {
        var fullWin64 = PathSupport.TrimTrailingSeparators(Path.GetFullPath(win64Dir));
        var fullTarget = PathSupport.TrimTrailingSeparators(Path.GetFullPath(targetDirectory));
        if (string.Equals(
            fullWin64,
            fullTarget,
            OperatingSystem.IsWindows() ? StringComparison.OrdinalIgnoreCase : StringComparison.Ordinal))
        {
            return;
        }

        var rel = PathSupport.NormalizeRelativePath(fullWin64, fullTarget);
        var segments = rel.Split('/', StringSplitOptions.RemoveEmptyEntries);
        var current = fullWin64;
        var currentRel = string.Empty;
        foreach (var segment in segments)
        {
            current = Path.Combine(current, segment);
            currentRel = currentRel.Length == 0 ? segment : $"{currentRel}/{segment}";
            var ioCurrent = PathSupport.ToIoPath(current);
            if (!Directory.Exists(ioCurrent))
            {
                Directory.CreateDirectory(ioCurrent);
                createdDirs.Add(current);
                createdRelDirs.Add(currentRel);
            }
        }
    }

    private static void RollbackWrittenFiles(List<string> writtenFiles, List<string> createdDirs)
    {
        for (var i = writtenFiles.Count - 1; i >= 0; i--)
        {
            try
            {
                var ioPath = PathSupport.ToIoPath(writtenFiles[i]);
                if (File.Exists(ioPath))
                {
                    File.Delete(ioPath);
                }
            }
            catch
            {
                // Best-effort rollback for each file.
            }
        }

        for (var i = createdDirs.Count - 1; i >= 0; i--)
        {
            try
            {
                var ioDir = PathSupport.ToIoPath(createdDirs[i]);
                if (Directory.Exists(ioDir) && !Directory.EnumerateFileSystemEntries(ioDir).Any())
                {
                    Directory.Delete(ioDir, recursive: false);
                }
            }
            catch
            {
                // Best-effort rollback for empty directories.
            }
        }
    }

    private static void ApplyIniSectionOverrides(
        List<string> lines,
        string sectionName,
        Dictionary<string, string> overrides,
        bool createSectionIfMissing)
    {
        var sectionHeaderIndex = -1;
        var sectionEndIndex = lines.Count;

        for (var i = 0; i < lines.Count; i++)
        {
            var trimmed = lines[i].Trim();
            if (trimmed.Length >= 2 && trimmed[0] == '[' && trimmed[^1] == ']')
            {
                var name = trimmed[1..^1].Trim();
                if (sectionHeaderIndex < 0)
                {
                    if (string.Equals(name, sectionName, StringComparison.OrdinalIgnoreCase))
                    {
                        sectionHeaderIndex = i;
                    }
                }
                else
                {
                    sectionEndIndex = i;
                    break;
                }
            }
        }

        if (sectionHeaderIndex < 0)
        {
            if (!createSectionIfMissing)
            {
                return;
            }

            if (lines.Count > 0 && !string.IsNullOrWhiteSpace(lines[^1]))
            {
                lines.Add(string.Empty);
            }

            lines.Add($"[{sectionName}]");
            foreach (var (key, value) in overrides)
            {
                lines.Add($"{key} = {value}");
            }

            return;
        }

        var updatedKeys = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
        for (var i = sectionHeaderIndex + 1; i < sectionEndIndex; i++)
        {
            var trimmed = lines[i].TrimStart();
            if (trimmed.Length == 0 || trimmed[0] == ';' || trimmed[0] == '#')
            {
                continue;
            }

            var eqIndex = lines[i].IndexOf('=');
            if (eqIndex <= 0)
            {
                continue;
            }

            var key = lines[i][..eqIndex].Trim();
            if (overrides.TryGetValue(key, out var newValue))
            {
                lines[i] = $"{key} = {newValue}";
                updatedKeys.Add(key);
            }
        }

        if (!createSectionIfMissing)
        {
            return;
        }

        var insertIndex = sectionEndIndex;
        while (insertIndex > sectionHeaderIndex + 1 && string.IsNullOrWhiteSpace(lines[insertIndex - 1]))
        {
            insertIndex--;
        }

        foreach (var (key, value) in overrides)
        {
            if (!updatedKeys.Contains(key))
            {
                lines.Insert(insertIndex++, $"{key} = {value}");
            }
        }
    }
}

/// <summary>
/// Orchestrates `ffr-export fetch-usmap` and `ffr-export uninstall-ue4ss`.
/// </summary>
internal static class UsmapFetcher
{
    internal const int DefaultTimeoutSeconds = 300;
    internal const string SavedUsmapLinePrefix = "已保存 .usmap：";

    private static readonly TimeSpan DefaultGracefulCloseWait = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DefaultPollInterval = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan DefaultProgressInterval = TimeSpan.FromSeconds(5);
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    internal static FetchUsmapOptions ParseFetchUsmapArgs(string[] args)
    {
        string? gameDirectory = null;
        var timeoutSeconds = DefaultTimeoutSeconds;
        var yes = false;

        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--game":
                    gameDirectory = RequireValue(args, ref index, "--game");
                    break;
                case "--timeout":
                    var rawTimeout = RequireValue(args, ref index, "--timeout");
                    if (!int.TryParse(rawTimeout, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
                        || parsed < 1)
                    {
                        throw new ArgumentException($"--timeout 必须是正整数（单位：秒）：{rawTimeout}");
                    }

                    timeoutSeconds = parsed;
                    break;
                case "--yes":
                    yes = true;
                    break;
                default:
                    throw new ArgumentException($"fetch-usmap 收到未知参数：{args[index]}");
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException("fetch-usmap 需要 --game <游戏目录>");
        }

        return new FetchUsmapOptions(gameDirectory, timeoutSeconds, yes);
    }

    internal static UninstallUe4ssOptions ParseUninstallUe4ssArgs(string[] args)
    {
        string? gameDirectory = null;
        var includeForeign = false;
        var yes = false;
        for (var index = 0; index < args.Length; index++)
        {
            switch (args[index])
            {
                case "--game":
                    gameDirectory = RequireValue(args, ref index, "--game");
                    break;
                case "--include-foreign":
                    includeForeign = true;
                    break;
                case "--yes":
                    yes = true;
                    break;
                default:
                    throw new ArgumentException($"uninstall-ue4ss 收到未知参数：{args[index]}");
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException("uninstall-ue4ss 需要 --game <游戏目录>");
        }

        return new UninstallUe4ssOptions(gameDirectory, includeForeign, yes);
    }

    internal static async Task<int> RunFetchAsync(
        FetchUsmapOptions options,
        TextReader input,
        TextWriter output,
        TextWriter error,
        IUe4ssPackageDownloader downloader,
        IGameLauncher launcher,
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        TimeSpan? pollIntervalOverride = null,
        TimeSpan? progressIntervalOverride = null,
        TimeSpan? gracefulCloseWaitOverride = null,
        Func<DateTimeOffset>? utcNowOverride = null,
        string? knownBuildsBaseDirOverride = null,
        CancellationToken cancellationToken = default)
    {
        var result = await FetchCoreAsync(
            options,
            input,
            output,
            error,
            downloader,
            launcher,
            agpHomeOverride,
            isWindowsOverride,
            pollIntervalOverride,
            progressIntervalOverride,
            gracefulCloseWaitOverride,
            utcNowOverride,
            knownBuildsBaseDirOverride,
            cancellationToken).ConfigureAwait(false);
        return result.ExitCode;
    }

    internal static async Task<(int ExitCode, FetchUsmapResult? Result)> FetchCoreAsync(
        FetchUsmapOptions options,
        TextReader input,
        TextWriter output,
        TextWriter error,
        IUe4ssPackageDownloader downloader,
        IGameLauncher launcher,
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        TimeSpan? pollIntervalOverride = null,
        TimeSpan? progressIntervalOverride = null,
        TimeSpan? gracefulCloseWaitOverride = null,
        Func<DateTimeOffset>? utcNowOverride = null,
        string? knownBuildsBaseDirOverride = null,
        CancellationToken cancellationToken = default)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var utcNow = utcNowOverride ?? (() => DateTimeOffset.UtcNow);
        var pollInterval = pollIntervalOverride ?? DefaultPollInterval;
        var progressInterval = progressIntervalOverride ?? DefaultProgressInterval;
        var gracefulCloseWait = gracefulCloseWaitOverride ?? DefaultGracefulCloseWait;

        output.WriteLine("==> 步骤 1/6：检查运行环境、游戏目录与游戏进程");
        if (!isWindows)
        {
            throw new PlatformNotSupportedException(
                "fetch-usmap 仅支持 Windows 系统（需要通过 Steam 启动 Windows 版游戏，并由 UE4SS 从运行中的内存转储 .usmap）。");
        }

        var gamePaths = GameVersionDetector.ValidateGameDirectory(options.GameDirectory);
        if (launcher.IsGameRunning())
        {
            throw new InvalidOperationException(
                $"检测到游戏正在运行（{Ue4ssRuntimeSupport.GameExeFileName}），请先关闭游戏后再执行 fetch-usmap。");
        }

        var inspection = Ue4ssRuntimeSupport.InspectUe4ssState(gamePaths.GameRoot);
        if (inspection.State == Ue4ssInstallState.PartialUnknown)
        {
            throw new InvalidOperationException(
                $"检查 UE4SS 状态失败：游戏目录 {inspection.Win64Directory} 下仅存在部分文件" +
                $"（dwmapi.dll={(inspection.HasDwmapiDll ? "存在" : "缺失")}，ue4ss/UE4SS.dll={(inspection.HasUe4ssDll ? "存在" : "缺失")}），" +
                "且没有本工具的安装标记文件。为避免覆盖您已有的代理 DLL 或模组文件，已中止操作。");
        }

        var downloadsDir = AgpPaths.GetDownloadsDirectory(agpHomeEnvOverride: agpHomeOverride, isWindowsOverride: isWindows);
        var usmapArchiveDir = AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: agpHomeOverride, isWindowsOverride: isWindows);

        if (!options.Yes)
        {
            PrintConfirmationPlan(output, gamePaths.GameRoot, inspection, downloadsDir, usmapArchiveDir, options.TimeoutSeconds);
            var confirmed = PromptYesNo(input, output);
            if (confirmed is null)
            {
                output.WriteLine("输入流已结束，已取消获取 .usmap。");
                return (2, null);
            }

            if (!confirmed.Value)
            {
                output.WriteLine("已取消获取 .usmap，未对游戏目录做任何修改。");
                return (0, null);
            }
        }

        output.WriteLine("==> 步骤 2/6：准备 UE4SS 运行环境");
        var installedUe4ss = false;
        switch (inspection.State)
        {
            case Ue4ssInstallState.InstalledByUser:
                output.WriteLine(
                    $"检测到游戏目录已存在您自行安装的 UE4SS（{inspection.Ue4ssDirectory}）；将完全保留原有设置，仅临时加入转储模组。");
                break;

            case Ue4ssInstallState.InstalledByFfrExport:
                if (inspection.HasDwmapiDll && inspection.HasUe4ssDll)
                {
                    output.WriteLine(
                        $"检测到本工具先前安装的 UE4SS（版本：{inspection.InstalledVersion ?? HttpsUe4ssPackageDownloader.Ue4ssVersion}），直接复用。");
                }
                else
                {
                    throw new InvalidOperationException(
                        $"检测到本工具的安装标记文件（{inspection.MarkerPath}），但核心文件不完整（dwmapi.dll 或 UE4SS.dll 缺失）。" +
                        "请先运行 ffr-export uninstall-ue4ss 清理后再重试。");
                }

                break;

            case Ue4ssInstallState.NotInstalled:
                var archivePath = await downloader
                    .EnsureVerifiedArchiveAsync(downloadsDir, output, cancellationToken)
                    .ConfigureAwait(false);
                var archiveSha = GameVersionDetector.ComputeFileSha256Lower(archivePath);
                var marker = Ue4ssArchiveInstaller.InstallFromVerifiedZip(
                    archivePath,
                    gamePaths.GameRoot,
                    archiveSha,
                    installedAtUtc: utcNow());
                installedUe4ss = true;
                output.WriteLine(
                    $"已安装 UE4SS {marker.Ue4ssVersion} 到 {inspection.Win64Directory}（共写入 {marker.Files.Length} 个文件，已关闭控制台与调试窗口，已保留 LICENSE）。");
                break;
        }

        var modDeployed = false;
        var gameLaunched = false;

        try
        {
            output.WriteLine("==> 步骤 3/6：部署临时 .usmap 转储模组");
            var modDir = Ue4ssModManager.DeployTemporaryDumpMod(gamePaths.GameRoot);
            modDeployed = true;
            output.WriteLine($"已部署临时转储模组：{modDir}（通过 enabled.txt 启用，未修改 mods.txt）。");

            output.WriteLine("==> 步骤 4/6：通过 Steam 启动游戏并等待 .usmap 转储完成");
            var launchTimeUtc = utcNow();
            output.WriteLine($"正在通过 Steam 启动游戏：{Ue4ssRuntimeSupport.SteamRunGameUri}（超时时间：{options.TimeoutSeconds} 秒）");
            launcher.LaunchGameViaSteam(Ue4ssRuntimeSupport.SteamRunGameUri);
            gameLaunched = true;

            var dumpedUsmapPath = await WaitForUsmapDumpAsync(
                gamePaths.GameRoot,
                inspection.Win64Directory,
                inspection.Ue4ssDirectory,
                launchTimeUtc,
                TimeSpan.FromSeconds(options.TimeoutSeconds),
                pollInterval,
                progressInterval,
                launcher,
                utcNow,
                output,
                cancellationToken).ConfigureAwait(false);

            output.WriteLine("==> 步骤 5/6：归档 .usmap、计算哈希并比对已知版本");
            var usmapInfo = GameVersionDetector.ComputeUsmapHashes(dumpedUsmapPath);
            var steamInfo = GameVersionDetector.ReadSteamBuildInfo(gamePaths.GameRoot, null, error);
            var buildSegment = AgpPaths.SanitizePathSegment(steamInfo.BuildId) ?? "unknown";
            var hashPrefix = usmapInfo.FileSha256.Length >= 8
                ? usmapInfo.FileSha256[..8]
                : usmapInfo.FileSha256;
            var archivedFileName = $"FFRS-{buildSegment}-{hashPrefix}.usmap";

            Directory.CreateDirectory(PathSupport.ToIoPath(usmapArchiveDir));
            var archivedPath = Path.Combine(usmapArchiveDir, archivedFileName);
            CopyFileAtomically(dumpedUsmapPath, archivedPath);

            output.WriteLine($"{SavedUsmapLinePrefix}{archivedPath}");
            output.WriteLine($"原始转储位置：{dumpedUsmapPath}");
            output.WriteLine(
                $"文件哈希（file_sha256）：{usmapInfo.FileSha256}（大小：{usmapInfo.Size} 字节）");
            output.WriteLine(
                $"内容哈希（content_sha256）：{usmapInfo.ContentSha256}（{usmapInfo.TypeCount} 类，{usmapInfo.EnumCount} 枚举）");

            var (knownEntries, knownSource) = KnownBuildsRegistry.Load(knownBuildsBaseDirOverride);
            var comparison = KnownBuildsRegistry.CompareUsmap(knownEntries, knownSource, usmapInfo);
            KnownBuildsRegistry.PrintUsmapComparison(comparison, output);

            var fetchResult = new FetchUsmapResult(
                archivedPath,
                dumpedUsmapPath,
                usmapInfo,
                comparison,
                installedUe4ss);
            return (0, fetchResult);
        }
        finally
        {
            if (modDeployed || gameLaunched)
            {
                output.WriteLine("==> 步骤 6/6：停用转储模组并关闭游戏");
            }

            if (modDeployed)
            {
                Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(gamePaths.GameRoot, message => output.WriteLine(message));
                output.WriteLine("已移除临时转储模组（ue4ss/Mods/FfrUsmapDump）。");
            }

            if (gameLaunched)
            {
                await launcher
                    .CloseGameAsync(gracefulCloseWait, message => output.WriteLine(message), CancellationToken.None)
                    .ConfigureAwait(false);
            }

            if (modDeployed || gameLaunched)
            {
                var retainMessage = inspection.State == Ue4ssInstallState.InstalledByUser
                    ? "已保留您原有的 UE4SS 环境不动。"
                    : "UE4SS 已保留在游戏目录中供下次复用（随时可运行 ffr-export uninstall-ue4ss 或在主窗口点击「清理 UE4SS…」清理）。";
                output.WriteLine(retainMessage);
            }
        }
    }

    private static async Task<string> WaitForUsmapDumpAsync(
        string gameDirectory,
        string win64Dir,
        string ue4ssDir,
        DateTimeOffset launchTimeUtc,
        TimeSpan timeout,
        TimeSpan pollInterval,
        TimeSpan progressInterval,
        IGameLauncher launcher,
        Func<DateTimeOffset> utcNow,
        TextWriter output,
        CancellationToken cancellationToken)
    {
        var donePath = Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDirectory);
        var ue4ssLogPath = Path.Combine(ue4ssDir, "UE4SS.log");
        var wallWatch = Stopwatch.StartNew();
        var nextProgressAt = progressInterval;
        var sawGameRunning = false;

        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();

            if (launcher.IsGameRunning())
            {
                sawGameRunning = true;
            }

            if (TryReadDoneMarker(donePath, out var doneOk, out var doneUsmapFile, out var doneError))
            {
                if (!doneOk)
                {
                    throw new InvalidOperationException(
                        $"UE4SS 转储模组执行失败：{doneError ?? "未知错误"}。请检查日志：{ue4ssLogPath}");
                }

                var candidateUsmap = ResolveDumpedUsmapPath(win64Dir, ue4ssDir, doneUsmapFile, launchTimeUtc);
                if (candidateUsmap is not null && IsReadableNonEmptyFile(candidateUsmap))
                {
                    output.WriteLine($"检测到转储完成标记，已找到最新 .usmap：{candidateUsmap}");
                    return candidateUsmap;
                }
            }

            var elapsed = wallWatch.Elapsed;
            if (elapsed >= timeout)
            {
                var advice = sawGameRunning
                    ? $"游戏已启动，但等待 {Math.Ceiling(timeout.TotalSeconds)} 秒仍未生成新的 .usmap；" +
                      $"可能是 UE4SS 未正常加载或游戏初始化耗时较长，请查看日志：{ue4ssLogPath}（也可尝试增大 --timeout 后重试）。"
                    : $"等待 {Math.Ceiling(timeout.TotalSeconds)} 秒仍未检测到游戏进程（{Ue4ssRuntimeSupport.GameExeFileName}）；" +
                      "游戏可能未能通过 Steam 启动，请确认 Steam 已登录、游戏可正常启动且没有被更新弹窗阻塞后重试。";
                throw new TimeoutException($"等待 .usmap 转储超时：{advice}");
            }

            if (elapsed >= nextProgressAt)
            {
                var elapsedSec = (int)Math.Floor(elapsed.TotalSeconds);
                var totalSec = (int)Math.Ceiling(timeout.TotalSeconds);
                var stateText = sawGameRunning ? "游戏运行中，等待引擎初始化与转储" : "等待 Steam 启动游戏";
                output.WriteLine(string.Create(
                    CultureInfo.InvariantCulture,
                    $"等待 .usmap 转储……已等待 {elapsedSec}/{totalSec} 秒（{stateText}）"));
                while (nextProgressAt <= elapsed)
                {
                    nextProgressAt += progressInterval;
                }
            }

            var remaining = timeout - elapsed;
            var sleepDuration = remaining < pollInterval ? remaining : pollInterval;
            if (sleepDuration > TimeSpan.Zero)
            {
                await Task.Delay(sleepDuration, cancellationToken).ConfigureAwait(false);
            }
        }
    }

    private static bool TryReadDoneMarker(
        string donePath,
        out bool ok,
        out string? usmapFile,
        out string? errorMessage)
    {
        ok = false;
        usmapFile = null;
        errorMessage = null;

        var ioPath = PathSupport.ToIoPath(donePath);
        if (!File.Exists(ioPath))
        {
            return false;
        }

        string text;
        try
        {
            using var stream = new FileStream(ioPath, FileMode.Open, FileAccess.Read, FileShare.ReadWrite);
            using var reader = new StreamReader(stream, Utf8WithoutBom);
            text = reader.ReadToEnd();
        }
        catch (IOException)
        {
            // File may still be flushing/renaming; retry on next poll tick.
            return false;
        }

        if (string.IsNullOrWhiteSpace(text))
        {
            return false;
        }

        try
        {
            var doc = JObject.Parse(text);
            ok = doc["ok"]?.Value<bool>() ?? false;
            usmapFile = doc["usmapFile"]?.Value<string>();
            errorMessage = doc["error"]?.Value<string>();
            return true;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static string? ResolveDumpedUsmapPath(
        string win64Dir,
        string ue4ssDir,
        string? hintedFileName,
        DateTimeOffset launchTimeUtc)
    {
        if (!string.IsNullOrWhiteSpace(hintedFileName))
        {
            var safeName = Path.GetFileName(hintedFileName.Trim());
            if (!string.IsNullOrEmpty(safeName)
                && safeName.EndsWith(".usmap", StringComparison.OrdinalIgnoreCase))
            {
                foreach (var dir in new[] { ue4ssDir, win64Dir })
                {
                    var candidate = Path.Combine(dir, safeName);
                    if (IsNewerThanLaunch(candidate, launchTimeUtc))
                    {
                        return candidate;
                    }
                }
            }
        }

        string? bestPath = null;
        var bestWriteUtc = DateTime.MinValue;
        foreach (var dir in new[] { ue4ssDir, win64Dir })
        {
            var ioDir = PathSupport.ToIoPath(dir);
            if (!Directory.Exists(ioDir))
            {
                continue;
            }

            foreach (var rawFile in Directory.EnumerateFiles(ioDir, "*.usmap", SearchOption.TopDirectoryOnly))
            {
                var fullPath = Path.GetFullPath(rawFile);
                if (!IsNewerThanLaunch(fullPath, launchTimeUtc))
                {
                    continue;
                }

                var writeUtc = File.GetLastWriteTimeUtc(PathSupport.ToIoPath(fullPath));
                if (bestPath is null || writeUtc > bestWriteUtc)
                {
                    bestPath = fullPath;
                    bestWriteUtc = writeUtc;
                }
            }
        }

        return bestPath;
    }

    private static bool IsNewerThanLaunch(string filePath, DateTimeOffset launchTimeUtc)
    {
        var ioPath = PathSupport.ToIoPath(filePath);
        if (!File.Exists(ioPath))
        {
            return false;
        }

        try
        {
            var info = new FileInfo(ioPath);
            if (!info.Exists || info.Length <= 0)
            {
                return false;
            }

            var writeUtc = new DateTimeOffset(DateTime.SpecifyKind(info.LastWriteTimeUtc, DateTimeKind.Utc));
            return writeUtc > launchTimeUtc;
        }
        catch
        {
            return false;
        }
    }

    private static bool IsReadableNonEmptyFile(string filePath)
    {
        try
        {
            var ioPath = PathSupport.ToIoPath(filePath);
            using var stream = new FileStream(ioPath, FileMode.Open, FileAccess.Read, FileShare.Read);
            return stream.Length > 0;
        }
        catch (IOException)
        {
            return false;
        }
    }

    private static void CopyFileAtomically(string sourcePath, string destinationPath)
    {
        var tempPath = $"{destinationPath}.{Environment.ProcessId}.{Guid.NewGuid():N}.tmp";
        try
        {
            File.Copy(PathSupport.ToIoPath(sourcePath), PathSupport.ToIoPath(tempPath), overwrite: true);
            File.Move(PathSupport.ToIoPath(tempPath), PathSupport.ToIoPath(destinationPath), overwrite: true);
            File.SetLastWriteTimeUtc(PathSupport.ToIoPath(destinationPath), DateTime.UtcNow);
        }
        catch
        {
            try
            {
                if (File.Exists(PathSupport.ToIoPath(tempPath)))
                {
                    File.Delete(PathSupport.ToIoPath(tempPath));
                }
            }
            catch
            {
                // Ignore temp cleanup failures.
            }

            throw;
        }
    }

    private static void PrintConfirmationPlan(
        TextWriter output,
        string gameRoot,
        Ue4ssInspection inspection,
        string downloadsDir,
        string usmapArchiveDir,
        int timeoutSeconds)
    {
        output.WriteLine();
        output.WriteLine("即将执行以下操作：");
        output.WriteLine($"  1. 目标游戏目录：{gameRoot}");
        if (inspection.State == Ue4ssInstallState.NotInstalled)
        {
            output.WriteLine(
                $"  2. 从官方 GitHub 下载 UE4SS {HttpsUe4ssPackageDownloader.Ue4ssVersion}（约 8.4 MB，缓存于 {downloadsDir}，校验 SHA-256），并安装到 {inspection.Win64Directory}（关闭控制台与调试窗口）；");
        }
        else if (inspection.State == Ue4ssInstallState.InstalledByUser)
        {
            output.WriteLine("  2. 检测到您已自行安装 UE4SS，保持原有配置不变；");
        }
        else
        {
            output.WriteLine("  2. 复用本工具先前安装的 UE4SS；");
        }

        output.WriteLine($"  3. 部署临时只读模组 ue4ss/Mods/{Ue4ssRuntimeSupport.DumpModName}（通过 enabled.txt 启用，不修改 mods.txt）；");
        output.WriteLine(string.Create(
            CultureInfo.InvariantCulture,
            $"  4. 通过 Steam 启动游戏一次（最长等待 {timeoutSeconds} 秒），在标题画面自动转储 .usmap，不触碰游戏存档；"));
        output.WriteLine($"  5. 将 .usmap 复制归档到 {usmapArchiveDir} 并与 known-builds.json 核对；");
        output.WriteLine("  6. 移除临时模组并自动关闭游戏；UE4SS 将保留供下次使用（随时可用 uninstall-ue4ss 卸载）。");
        output.WriteLine();
    }

    private static bool? PromptYesNo(TextReader input, TextWriter output)
    {
        while (true)
        {
            output.Write("确认继续吗？（输入 y 确认，直接回车或输入 n 取消）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (trimmed.Equals("y", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("yes", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("是", StringComparison.Ordinal))
            {
                return true;
            }

            if (string.IsNullOrEmpty(trimmed)
                || trimmed.Equals("n", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("no", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("否", StringComparison.Ordinal))
            {
                return false;
            }

            output.WriteLine("输入无效，请输入 y 确认或 n 取消。");
        }
    }

    private static string RequireValue(string[] args, ref int index, string option)
    {
        if (index + 1 >= args.Length)
        {
            throw new ArgumentException($"选项 {option} 需要一个值");
        }

        index++;
        return args[index];
    }
}

internal static partial class Program
{
    private static async Task<int> RunFetchUsmapAsync(string[] args)
    {
        var options = UsmapFetcher.ParseFetchUsmapArgs(args);
        var agpResolution = AgpPaths.ResolveWritableAgpRootOrThrow(Console.Out);
        using var logOut = CliLogWriter.Create(
            Console.Out,
            AgpPaths.GetLogsDirectoryForAgpRoot(agpResolution.AgpRoot!));
        using var cts = new CancellationTokenSource();

        ConsoleCancelEventHandler cancelHandler = (_, eventArgs) =>
        {
            eventArgs.Cancel = true;
            logOut.WriteLine("收到中断信号，正在取消并清理临时模组与游戏进程……");
            try
            {
                cts.Cancel();
            }
            catch (ObjectDisposedException)
            {
                // Ignore cancellation race on shutdown.
            }
        };

        Console.CancelKeyPress += cancelHandler;
        try
        {
            var downloader = new HttpsUe4ssPackageDownloader();
            var launcher = new WindowsSteamGameLauncher();
            return await UsmapFetcher.RunFetchAsync(
                options,
                Console.In,
                logOut,
                Console.Error,
                downloader,
                launcher,
                agpHomeOverride: agpResolution.AgpRoot,
                cancellationToken: cts.Token).ConfigureAwait(false);
        }
        catch (OperationCanceledException)
        {
            Console.Error.WriteLine("已取消获取 .usmap，临时模组已停用。");
            return 2;
        }
        finally
        {
            Console.CancelKeyPress -= cancelHandler;
        }
    }

    private static Task<int> RunUninstallUe4ssAsync(string[] args)
    {
        var options = UsmapFetcher.ParseUninstallUe4ssArgs(args);
        var agpResolution = AgpPaths.ResolveWritableAgpRootOrThrow(Console.Out);
        using var logOut = CliLogWriter.Create(
            Console.Out,
            AgpPaths.GetLogsDirectoryForAgpRoot(agpResolution.AgpRoot!));
        var launcher = new WindowsSteamGameLauncher();
        _ = Ue4ssArchiveInstaller.Uninstall(
            options.GameDirectory,
            launcher,
            logOut,
            includeForeign: options.IncludeForeign,
            yes: options.Yes,
            input: Console.In,
            agpHomeOverride: agpResolution.AgpRoot);
        return Task.FromResult(0);
    }
}
