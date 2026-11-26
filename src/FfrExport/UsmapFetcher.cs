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
    bool Yes,
    bool ProgressJson = false);

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
                L.Format("Ue4ss_SteamLaunchFailed", steamUri, exception.Message),
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
            throw new InvalidOperationException(L.Format("Ue4ss_MissingEmbeddedLua", EmbeddedMainLuaResourceName));
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
            throw new IOException(L.Format("Ue4ss_CleanOldDoneMarkerFailed", donePath, exception.Message), exception);
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
            throw new ArgumentException(L.Get("Ue4ss_ModNameEmpty"), nameof(modName));
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
            throw new DirectoryNotFoundException(L.Format("Ue4ss_InstallWin64Missing", win64Dir));
        }

        using var archive = ZipFile.OpenRead(PathSupport.ToIoPath(zipPath));
        var plannedEntries = PlanZipExtraction(archive, zipPath);

        // Refuse to overwrite any pre-existing files in Win64 before writing anything.
        foreach (var entry in plannedEntries)
        {
            var destination = CombineUnderWin64(win64Dir, entry.RelativePath);
            if (File.Exists(PathSupport.ToIoPath(destination)))
            {
                throw new IOException(L.Format("Ue4ss_InstallRefuseOverwrite", destination));
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
                L.Format("Ue4ss_UninstallGameRunning", Ue4ssRuntimeSupport.GameExeFileName));
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
            output.WriteLine(L.Format("Ue4ss_UninstallOwnDone", removedFiles, removedDirs));
            if (remaining > 0)
            {
                output.WriteLine(L.Format("Ue4ss_UninstallOwnRemaining", remaining, inspection.Ue4ssDirectory));
            }

            output.WriteLine(L.Get("Ue4ss_UninstallUsmapSafeNote"));
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
                    output.WriteLine(L.Format(
                        "Ue4ss_UninstallForeignCleanedRemnants",
                        Ue4ssRuntimeSupport.MarkerFileName,
                        remnantFiles,
                        remnantDirs));
                }
                else
                {
                    output.WriteLine(L.Format(
                        "Ue4ss_UninstallForeignNoRemnants",
                        Ue4ssRuntimeSupport.MarkerFileName));
                }

                output.WriteLine(L.Get("Ue4ss_UninstallForeignHint"));
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
                output.WriteLine(L.Get("Ue4ss_UninstallForeignWarning"));
                output.WriteLine(Ue4ssRuntimeSupport.FormatCleanupPreviewText(plan, includeForeign: true, backupDir));
                output.WriteLine();

                var confirmed = PromptYesNoForCleanup(input ?? TextReader.Null, output);
                if (confirmed is not true)
                {
                    output.WriteLine(L.Get("Ue4ss_UninstallCancelled"));
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

            output.WriteLine(L.Format("Ue4ss_UninstallForeignBackedUp", backupDir));
            if (cleanedFiles > 0 || cleanedDirs > 0)
            {
                output.WriteLine(L.Format("Ue4ss_UninstallForeignAlsoCleaned", cleanedFiles, cleanedDirs));
            }

            output.WriteLine(L.Format("Ue4ss_UninstallForeignRestoreNote", backupDir, inspection.Win64Directory));
            output.WriteLine(L.Get("Ue4ss_UninstallUsmapSafeNote"));

            return new Ue4ssUninstallResult(
                Ue4ssUninstallOutcome.BackedUpForeignInstall,
                cleanedFiles,
                cleanedDirs,
                RemainingFilesInUe4ss: 0,
                BackupDirectory: backupDir);
        }

        // Case 3: No UE4SS installed; clean any stray remnants of ours if present, otherwise report not found.
        var (strayFiles, strayDirs) = Ue4ssRuntimeSupport.CleanOurRemnants(
            gamePaths.GameRoot,
            removeEmptyParentDirs: true,
            log: message => output.WriteLine(message));
        if (strayFiles > 0 || strayDirs > 0)
        {
            output.WriteLine(L.Format("Ue4ss_UninstallRemnantsOnly", strayFiles, strayDirs));
            return new Ue4ssUninstallResult(
                Ue4ssUninstallOutcome.RemovedRemnantsOnly,
                strayFiles,
                strayDirs,
                CountFilesInDirectory(inspection.Ue4ssDirectory),
                BackupDirectory: null);
        }

        output.WriteLine(L.Get("Ue4ss_UninstallNotFound"));
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

            throw new IOException(L.Format("Ue4ss_BackupCopyFailed", backupDir, exception.Message), exception);
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
            throw new IOException(L.Format("Ue4ss_BackupRemoveOrigFailed", backupDir, exception.Message), exception);
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
            output.Write(L.Get("Ue4ss_CleanupConfirmPrompt"));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (trimmed.Equals("y", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("yes", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("是", StringComparison.Ordinal)
                || trimmed.Equals("はい", StringComparison.Ordinal))
            {
                return true;
            }

            if (string.IsNullOrEmpty(trimmed)
                || trimmed.Equals("n", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("no", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("否", StringComparison.Ordinal)
                || trimmed.Equals("いいえ", StringComparison.Ordinal))
            {
                return false;
            }

            output.WriteLine(L.Get("Ue4ss_YesNoInvalid"));
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
            throw new IOException(L.Format("Ue4ss_MarkerReadFailed", markerPath, exception.Message), exception);
        }

        Ue4ssInstallMarker? marker;
        try
        {
            marker = JsonConvert.DeserializeObject<Ue4ssInstallMarker>(rawJson);
        }
        catch (JsonException exception)
        {
            throw new InvalidDataException(L.Format("Ue4ss_MarkerCorrupt", markerPath, exception.Message), exception);
        }

        if (marker is null
            || marker.Format != 1
            || !string.Equals(marker.InstalledBy, Ue4ssRuntimeSupport.InstalledByIdentifier, StringComparison.Ordinal)
            || marker.Files is null
            || marker.Directories is null)
        {
            throw new InvalidDataException(L.Format("Ue4ss_MarkerInvalid", markerPath));
        }

        var validatedFiles = new string[marker.Files.Length];
        for (var i = 0; i < marker.Files.Length; i++)
        {
            validatedFiles[i] = ValidateSafeRelativePath(marker.Files[i], L.Get("Ue4ss_LabelFile"), markerPath);
            _ = CombineUnderWin64(win64Dir, validatedFiles[i]);
        }

        var validatedDirs = new string[marker.Directories.Length];
        for (var i = 0; i < marker.Directories.Length; i++)
        {
            validatedDirs[i] = ValidateSafeRelativePath(marker.Directories[i], L.Get("Ue4ss_LabelDirectory"), markerPath);
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

            var normalized = ValidateSafeRelativePath(entry.FullName, L.Get("Ue4ss_LabelZipEntry"), zipPath);
            rawFileEntries.Add((normalized, entry));
        }

        if (rawFileEntries.Count == 0)
        {
            throw new InvalidDataException(L.Format("Ue4ss_ZipEmpty", zipPath));
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
            relative = ValidateSafeRelativePath(relative, L.Get("Ue4ss_LabelExtractPath"), zipPath);
            if (!seen.Add(relative))
            {
                throw new InvalidDataException(L.Format("Ue4ss_ZipDuplicateEntry", relative, zipPath));
            }

            planned.Add(new PlannedZipEntry(relative, entry));
        }

        var hasDwmapi = planned.Any(e =>
            string.Equals(e.RelativePath, Ue4ssRuntimeSupport.DwmapiFileName, StringComparison.OrdinalIgnoreCase));
        var hasUe4ssDll = planned.Any(e =>
            string.Equals(e.RelativePath, "ue4ss/UE4SS.dll", StringComparison.OrdinalIgnoreCase));
        if (!hasDwmapi || !hasUe4ssDll)
        {
            throw new InvalidDataException(L.Format("Ue4ss_ZipUnexpectedLayout", zipPath));
        }

        return planned;
    }

    private static string ValidateSafeRelativePath(string rawPath, string label, string source)
    {
        if (string.IsNullOrWhiteSpace(rawPath))
        {
            throw new InvalidDataException(L.Format("Ue4ss_PathEmpty", label, source));
        }

        var normalized = rawPath.Replace('\\', '/');
        if (normalized.StartsWith('/')
            || normalized.Contains('\0', StringComparison.Ordinal)
            || (normalized.Length >= 2 && normalized[1] == ':'))
        {
            throw new InvalidDataException(L.Format("Ue4ss_UnsafeAbsolutePath", label, rawPath, source));
        }

        var segments = normalized.Split('/');
        foreach (var segment in segments)
        {
            if (string.IsNullOrEmpty(segment) || segment == "." || segment == "..")
            {
                throw new InvalidDataException(L.Format("Ue4ss_UnsafeRelativePath", label, rawPath, source));
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
            throw new InvalidDataException(L.Format("Ue4ss_PathEscapesWin64", safeRelativePath));
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

    private static readonly TimeSpan DefaultGracefulCloseWait = TimeSpan.FromSeconds(15);
    private static readonly TimeSpan DefaultPollInterval = TimeSpan.FromMilliseconds(500);
    private static readonly TimeSpan DefaultProgressInterval = TimeSpan.FromSeconds(5);
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    private static void WriteStep(TextWriter output, int step, int total, string title, bool progressJson)
    {
        if (progressJson)
        {
            output.WriteLine(ProgressProtocol.FormatProgressLine(step, total, title));
        }

        output.WriteLine(L.Format("BuildDb_StepLine", step, total, title));
    }

    internal static FetchUsmapOptions ParseFetchUsmapArgs(string[] args)
    {
        string? gameDirectory = null;
        var timeoutSeconds = DefaultTimeoutSeconds;
        var yes = false;
        var progressJson = false;

        for (var index = 0; index < args.Length; index++)
        {
            var token = args[index];
            if (token.StartsWith("--lang=", StringComparison.Ordinal))
            {
                var langValue = token["--lang=".Length..];
                if (!Localization.TryNormalizeLanguageCode(langValue, out _))
                {
                    throw new ArgumentException(L.Format("Common_UnsupportedLanguage", langValue));
                }

                continue;
            }

            switch (token)
            {
                case "--game":
                    gameDirectory = RequireValue(args, ref index, "--game");
                    break;
                case "--timeout":
                    var rawTimeout = RequireValue(args, ref index, "--timeout");
                    if (!int.TryParse(rawTimeout, NumberStyles.Integer, CultureInfo.InvariantCulture, out var parsed)
                        || parsed < 1)
                    {
                        throw new ArgumentException(L.Format("FetchUsmap_TimeoutInvalid", rawTimeout));
                    }

                    timeoutSeconds = parsed;
                    break;
                case "--yes":
                    yes = true;
                    break;
                case "--progress-json":
                    progressJson = true;
                    break;
                case "--lang":
                    var rawLang = RequireValue(args, ref index, "--lang");
                    if (!Localization.TryNormalizeLanguageCode(rawLang, out _))
                    {
                        throw new ArgumentException(L.Format("Common_UnsupportedLanguage", rawLang));
                    }

                    break;
                default:
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "fetch-usmap", token));
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException(L.Get("FetchUsmap_RequiresGame"));
        }

        return new FetchUsmapOptions(gameDirectory, timeoutSeconds, yes, progressJson);
    }

    internal static UninstallUe4ssOptions ParseUninstallUe4ssArgs(string[] args)
    {
        string? gameDirectory = null;
        var includeForeign = false;
        var yes = false;
        for (var index = 0; index < args.Length; index++)
        {
            var token = args[index];
            if (token.StartsWith("--lang=", StringComparison.Ordinal))
            {
                var langValue = token["--lang=".Length..];
                if (!Localization.TryNormalizeLanguageCode(langValue, out _))
                {
                    throw new ArgumentException(L.Format("Common_UnsupportedLanguage", langValue));
                }

                continue;
            }

            switch (token)
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
                case "--lang":
                    var rawLang = RequireValue(args, ref index, "--lang");
                    if (!Localization.TryNormalizeLanguageCode(rawLang, out _))
                    {
                        throw new ArgumentException(L.Format("Common_UnsupportedLanguage", rawLang));
                    }

                    break;
                default:
                    throw new ArgumentException(L.Format("Cli_UnknownArg", "uninstall-ue4ss", token));
            }
        }

        if (string.IsNullOrWhiteSpace(gameDirectory))
        {
            throw new ArgumentException(L.Get("UninstallUe4ss_RequiresGame"));
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

        WriteStep(output, 1, 6, L.Get("FetchUsmap_Step1Title"), options.ProgressJson);
        if (!isWindows)
        {
            throw new PlatformNotSupportedException(L.Get("FetchUsmap_WindowsOnly"));
        }

        var gamePaths = GameVersionDetector.ValidateGameDirectory(options.GameDirectory);
        if (launcher.IsGameRunning())
        {
            throw new InvalidOperationException(L.Format("FetchUsmap_GameAlreadyRunning", Ue4ssRuntimeSupport.GameExeFileName));
        }

        var inspection = Ue4ssRuntimeSupport.InspectUe4ssState(gamePaths.GameRoot);
        if (inspection.State == Ue4ssInstallState.PartialUnknown)
        {
            throw new InvalidOperationException(L.Format(
                "FetchUsmap_PartialUnknownUe4ss",
                inspection.Win64Directory,
                inspection.HasDwmapiDll ? L.Get("Common_Present") : L.Get("Common_Missing"),
                inspection.HasUe4ssDll ? L.Get("Common_Present") : L.Get("Common_Missing")));
        }

        var downloadsDir = AgpPaths.GetDownloadsDirectory(agpHomeEnvOverride: agpHomeOverride, isWindowsOverride: isWindows);
        var usmapArchiveDir = AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: agpHomeOverride, isWindowsOverride: isWindows);

        if (!options.Yes)
        {
            PrintConfirmationPlan(output, gamePaths.GameRoot, inspection, downloadsDir, usmapArchiveDir, options.TimeoutSeconds);
            var confirmed = PromptYesNo(input, output);
            if (confirmed is null)
            {
                output.WriteLine(L.Get("FetchUsmap_InputClosedCancelled"));
                return (2, null);
            }

            if (!confirmed.Value)
            {
                output.WriteLine(L.Get("FetchUsmap_CancelledNoChanges"));
                return (0, null);
            }
        }

        WriteStep(output, 2, 6, L.Get("FetchUsmap_Step2Title"), options.ProgressJson);
        var installedUe4ss = false;
        switch (inspection.State)
        {
            case Ue4ssInstallState.InstalledByUser:
                output.WriteLine(L.Format("FetchUsmap_ReuseUserUe4ss", inspection.Ue4ssDirectory));
                break;

            case Ue4ssInstallState.InstalledByFfrExport:
                if (inspection.HasDwmapiDll && inspection.HasUe4ssDll)
                {
                    output.WriteLine(L.Format(
                        "FetchUsmap_ReuseOwnUe4ss",
                        inspection.InstalledVersion ?? HttpsUe4ssPackageDownloader.Ue4ssVersion));
                }
                else
                {
                    throw new InvalidOperationException(L.Format("FetchUsmap_CorruptOwnUe4ss", inspection.MarkerPath));
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
                output.WriteLine(L.Format(
                    "FetchUsmap_InstalledUe4ss",
                    marker.Ue4ssVersion,
                    inspection.Win64Directory,
                    marker.Files.Length));
                break;
        }

        var modDeployed = false;
        var gameLaunched = false;

        try
        {
            WriteStep(output, 3, 6, L.Get("FetchUsmap_Step3Title"), options.ProgressJson);
            var modDir = Ue4ssModManager.DeployTemporaryDumpMod(gamePaths.GameRoot);
            modDeployed = true;
            output.WriteLine(L.Format("FetchUsmap_DeployedMod", modDir));

            WriteStep(output, 4, 6, L.Get("FetchUsmap_Step4Title"), options.ProgressJson);
            var launchTimeUtc = utcNow();
            output.WriteLine(L.Format(
                "FetchUsmap_LaunchingSteam",
                Ue4ssRuntimeSupport.SteamRunGameUri,
                options.TimeoutSeconds));
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

            WriteStep(output, 5, 6, L.Get("FetchUsmap_Step5Title"), options.ProgressJson);
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

            if (options.ProgressJson)
            {
                output.WriteLine(ProgressProtocol.FormatUsmapLine(archivedPath));
            }

            output.WriteLine(L.Format("FetchUsmap_SavedUsmapLine", archivedPath));
            output.WriteLine(L.Format("FetchUsmap_SourceDumpPath", dumpedUsmapPath));
            output.WriteLine(L.Format("FetchUsmap_FileShaLine", usmapInfo.FileSha256, usmapInfo.Size));
            output.WriteLine(L.Format(
                "FetchUsmap_ContentShaLine",
                usmapInfo.ContentSha256,
                usmapInfo.TypeCount,
                usmapInfo.EnumCount));

            var (knownEntries, knownSource) = KnownBuildsRegistry.Load(knownBuildsBaseDirOverride, error);
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
                WriteStep(output, 6, 6, L.Get("FetchUsmap_Step6Title"), options.ProgressJson);
            }

            if (modDeployed)
            {
                Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(gamePaths.GameRoot, message => output.WriteLine(message));
                output.WriteLine(L.Get("FetchUsmap_RemovedTempMod"));
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
                    ? L.Get("FetchUsmap_RetainUserUe4ss")
                    : L.Get("FetchUsmap_RetainOwnUe4ss");
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
                    throw new InvalidOperationException(L.Format(
                        "FetchUsmap_ModFailed",
                        doneError ?? L.Get("Common_UnknownError"),
                        ue4ssLogPath));
                }

                var candidateUsmap = ResolveDumpedUsmapPath(win64Dir, ue4ssDir, doneUsmapFile, launchTimeUtc);
                if (candidateUsmap is not null && IsReadableNonEmptyFile(candidateUsmap))
                {
                    output.WriteLine(L.Format("FetchUsmap_DoneMarkerFound", candidateUsmap));
                    return candidateUsmap;
                }
            }

            var elapsed = wallWatch.Elapsed;
            if (elapsed >= timeout)
            {
                var timeoutSeconds = (int)Math.Ceiling(timeout.TotalSeconds);
                var advice = sawGameRunning
                    ? L.Format("FetchUsmap_TimeoutGameRunning", timeoutSeconds, ue4ssLogPath)
                    : L.Format("FetchUsmap_TimeoutGameNotSeen", timeoutSeconds, Ue4ssRuntimeSupport.GameExeFileName);
                throw new TimeoutException(L.Format("FetchUsmap_TimeoutError", advice));
            }

            if (elapsed >= nextProgressAt)
            {
                var elapsedSec = (int)Math.Floor(elapsed.TotalSeconds);
                var totalSec = (int)Math.Ceiling(timeout.TotalSeconds);
                var stateText = sawGameRunning
                    ? L.Get("FetchUsmap_StateGameRunning")
                    : L.Get("FetchUsmap_StateWaitingSteam");
                output.WriteLine(L.Format("FetchUsmap_WaitingProgress", elapsedSec, totalSec, stateText));
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
        output.WriteLine(L.Get("FetchUsmap_PlanHeader"));
        output.WriteLine(L.Format("FetchUsmap_PlanStep1", gameRoot));
        if (inspection.State == Ue4ssInstallState.NotInstalled)
        {
            output.WriteLine(L.Format(
                "FetchUsmap_PlanStep2Install",
                HttpsUe4ssPackageDownloader.Ue4ssVersion,
                downloadsDir,
                inspection.Win64Directory));
        }
        else if (inspection.State == Ue4ssInstallState.InstalledByUser)
        {
            output.WriteLine(L.Get("FetchUsmap_PlanStep2User"));
        }
        else
        {
            output.WriteLine(L.Get("FetchUsmap_PlanStep2Reuse"));
        }

        output.WriteLine(L.Format("FetchUsmap_PlanStep3", Ue4ssRuntimeSupport.DumpModName));
        output.WriteLine(L.Format("FetchUsmap_PlanStep4", timeoutSeconds));
        output.WriteLine(L.Format("FetchUsmap_PlanStep5", usmapArchiveDir));
        output.WriteLine(L.Get("FetchUsmap_PlanStep6"));
        output.WriteLine();
    }

    private static bool? PromptYesNo(TextReader input, TextWriter output)
    {
        while (true)
        {
            output.Write(L.Get("FetchUsmap_ConfirmPrompt"));
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (trimmed.Equals("y", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("yes", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("是", StringComparison.Ordinal)
                || trimmed.Equals("はい", StringComparison.Ordinal))
            {
                return true;
            }

            if (string.IsNullOrEmpty(trimmed)
                || trimmed.Equals("n", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("no", StringComparison.OrdinalIgnoreCase)
                || trimmed.Equals("否", StringComparison.Ordinal)
                || trimmed.Equals("いいえ", StringComparison.Ordinal))
            {
                return false;
            }

            output.WriteLine(L.Get("Ue4ss_YesNoInvalid"));
        }
    }

    private static string RequireValue(string[] args, ref int index, string option)
    {
        if (index + 1 >= args.Length)
        {
            throw new ArgumentException(L.Format("Common_OptionRequiresValue", option));
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
            logOut.WriteLine(L.Get("FetchUsmap_CtrlCInterrupt"));
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
            Console.Error.WriteLine(L.Get("FetchUsmap_CancelledAfterInterrupt"));
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
