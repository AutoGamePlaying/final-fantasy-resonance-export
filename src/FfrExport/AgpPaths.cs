using System.Runtime.InteropServices;
using System.Text;

namespace FfrExport;

internal enum AgpRootSource
{
    AgpHomeEnv,
    LocalAppData,
    LocalAppDataLow,
    PortableExeDir,
    UserProfile,
    SavedLocationFile,
    UserSelected,
}

internal sealed record AgpWriteProbeResult(
    bool IsWritable,
    string? ErrorMessage = null);

internal sealed record AgpCandidateFailure(
    AgpRootSource Source,
    string Label,
    string Path,
    string Reason);

internal sealed record AgpRootResolution(
    bool Success,
    string? AgpRoot,
    AgpRootSource Source,
    ProcessIntegrityInfo Integrity,
    bool UsedFallback,
    string? FallbackReason,
    string? PrimaryFailureReason,
    IReadOnlyList<AgpCandidateFailure> Failures,
    string? ErrorMessage,
    bool LocationFileSaved = false,
    string? LocationFileSaveError = null,
    bool? IsWindowsContext = null)
{
    internal string? ToolRoot => string.IsNullOrWhiteSpace(AgpRoot)
        ? null
        : AgpPaths.ResolveToolRootForAgpRoot(AgpRoot);
}

/// <summary>
/// Single source of truth for the unified agp\ffr-export\ directory layout shared by
/// both the CLI (ffr-export) and WinForms GUI (ffr-export-gui).
/// Never touches %TEMP% / TMP; probes candidate data roots in priority order using
/// temporary probe files inside &lt;candidate&gt;\ffr-export\tmp\.
/// </summary>
internal static class AgpPaths
{
    internal const string AgpHomeEnvVar = "AGP_HOME";
    internal const string AgpFolderName = "agp";
    internal const string PortableAgpFolderName = "agp-data";
    internal const string SavedLocationFileName = "agp-location.txt";
    internal const string ToolFolderName = "ffr-export";
    internal const string OutputSubfolderName = "output";
    internal const string UsmapSubfolderName = "usmap";
    internal const string DownloadsSubfolderName = "downloads";
    internal const string LogsSubfolderName = "logs";
    internal const string TempSubfolderName = "tmp";
    internal const string Ue4ssBackupSubfolderName = "ue4ss-backup";
    internal const string SettingsFileName = "settings.json";

    private static readonly Guid FolderIdLocalAppDataLow = new("A520A1A4-1780-4FF6-BD18-167343C5AF16");

    internal static bool IsAgpHomeEnvironmentOverrideActive()
    {
        return !string.IsNullOrWhiteSpace(SafeGetEnvironmentVariable(AgpHomeEnvVar));
    }

    /// <summary>
    /// Resolves a writable "agp" data root in strict priority order:
    /// 1. AGP_HOME (explicit override; fails immediately without falling back if unwritable);
    /// 2. Windows: %LOCALAPPDATA%\agp (Linux/macOS: $XDG_DATA_HOME/agp or ~/.local/share/agp);
    /// 3. Windows: %USERPROFILE%\AppData\LocalLow\agp (writable even under Low Integrity Level);
    /// 4. &lt;exeDir&gt;\agp-data (portable mode);
    /// 5. %USERPROFILE%\agp (Linux/macOS: ~/agp);
    /// 6. Saved custom folder from &lt;exeDir&gt;\agp-location.txt (if present).
    /// Every candidate is verified by creating &lt;candidate&gt;\ffr-export\tmp\, writing a small probe file, and deleting it.
    /// </summary>
    internal static AgpRootResolution ResolveWritableAgpRoot(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? localAppDataLowOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null,
        string? executableDirectoryOverride = null,
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null,
        ProcessIntegrityInfo? integrityOverride = null)
    {
        var integrity = integrityOverride ?? ProcessIntegritySupport.DetectCurrent();
        var probe = writeProbeOverride ?? ProbeDirectoryWritable;
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var exeDir = ResolveExecutableDirectory(executableDirectoryOverride);
        var failures = new List<AgpCandidateFailure>();

        // 1. Explicit AGP_HOME: must succeed or fail directly without falling back.
        var rawAgpHome = agpHomeEnvOverride is not null
            ? agpHomeEnvOverride
            : SafeGetEnvironmentVariable(AgpHomeEnvVar);
        if (!string.IsNullOrWhiteSpace(rawAgpHome))
        {
            if (!TryNormalizePath(rawAgpHome, out var normalizedEnvHome))
            {
                var err = L.Format("Agp_EnvInvalidPath", AgpHomeEnvVar, rawAgpHome);
                var invalidPath = L.Get("Common_InvalidPath");
                failures.Add(new AgpCandidateFailure(AgpRootSource.AgpHomeEnv, AgpHomeEnvVar, rawAgpHome, invalidPath));
                return new AgpRootResolution(
                    Success: false,
                    AgpRoot: null,
                    Source: AgpRootSource.AgpHomeEnv,
                    Integrity: integrity,
                    UsedFallback: false,
                    FallbackReason: null,
                    PrimaryFailureReason: invalidPath,
                    Failures: failures,
                    ErrorMessage: err,
                    IsWindowsContext: isWindows);
            }

            var envProbe = InvokeProbeSafely(probe, normalizedEnvHome);
            if (envProbe.IsWritable)
            {
                return new AgpRootResolution(
                    Success: true,
                    AgpRoot: normalizedEnvHome,
                    Source: AgpRootSource.AgpHomeEnv,
                    Integrity: integrity,
                    UsedFallback: false,
                    FallbackReason: null,
                    PrimaryFailureReason: null,
                    Failures: failures,
                    ErrorMessage: null,
                    IsWindowsContext: isWindows);
            }

            var reason = string.IsNullOrWhiteSpace(envProbe.ErrorMessage) ? L.Get("Common_NotWritable") : envProbe.ErrorMessage!;
            failures.Add(new AgpCandidateFailure(AgpRootSource.AgpHomeEnv, AgpHomeEnvVar, normalizedEnvHome, reason));
            var envErr = L.Format("Agp_EnvNotWritable", AgpHomeEnvVar, normalizedEnvHome, reason);
            return new AgpRootResolution(
                Success: false,
                AgpRoot: null,
                Source: AgpRootSource.AgpHomeEnv,
                Integrity: integrity,
                UsedFallback: false,
                FallbackReason: null,
                PrimaryFailureReason: reason,
                Failures: failures,
                ErrorMessage: envErr,
                IsWindowsContext: isWindows);
        }

        // 2..6 Build ordered candidate list.
        var candidates = BuildOrderedFallbackCandidates(
            isWindows,
            localAppDataOverride,
            localAppDataLowOverride,
            xdgDataHomeOverride,
            userProfileOverride,
            exeDir,
            failures);

        var primaryLabel = isWindows ? "%LOCALAPPDATA%" : L.Get("Agp_DefaultUserDataDir");
        string? primaryFailureReason = failures
            .FirstOrDefault(f => f.Source == AgpRootSource.LocalAppData)
            ?.Reason;

        foreach (var candidate in candidates)
        {
            var result = InvokeProbeSafely(probe, candidate.Path);
            if (result.IsWritable)
            {
                var usedFallback = candidate.Source != AgpRootSource.LocalAppData;
                var fallbackReason = usedFallback ? L.Format("Agp_PrimaryNotWritable", primaryLabel) : null;
                return new AgpRootResolution(
                    Success: true,
                    AgpRoot: candidate.Path,
                    Source: candidate.Source,
                    Integrity: integrity,
                    UsedFallback: usedFallback,
                    FallbackReason: fallbackReason,
                    PrimaryFailureReason: primaryFailureReason,
                    Failures: failures,
                    ErrorMessage: null,
                    IsWindowsContext: isWindows);
            }

            var failReason = string.IsNullOrWhiteSpace(result.ErrorMessage) ? L.Get("Common_NotWritable") : result.ErrorMessage!;
            failures.Add(new AgpCandidateFailure(candidate.Source, candidate.Label, candidate.Path, failReason));
            if (candidate.Source == AgpRootSource.LocalAppData && primaryFailureReason is null)
            {
                primaryFailureReason = failReason;
            }
        }

        var allFailedMessage = FormatAllCandidatesFailedCliMessage(failures, integrity, isWindows);
        return new AgpRootResolution(
            Success: false,
            AgpRoot: null,
            Source: AgpRootSource.LocalAppData,
            Integrity: integrity,
            UsedFallback: true,
            FallbackReason: L.Format("Agp_PrimaryNotWritable", primaryLabel),
            PrimaryFailureReason: primaryFailureReason,
            Failures: failures,
            ErrorMessage: allFailedMessage,
            IsWindowsContext: isWindows);
    }

    /// <summary>
    /// Resolves a writable agp root for CLI execution, printing a concise localized fallback/Low IL notice
    /// when applicable and throwing an IOException with actionable guidance if no candidate is writable.
    /// </summary>
    internal static AgpRootResolution ResolveWritableAgpRootOrThrow(
        TextWriter? noticeOutput = null,
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? localAppDataLowOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null,
        string? executableDirectoryOverride = null,
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null,
        ProcessIntegrityInfo? integrityOverride = null)
    {
        var resolution = ResolveWritableAgpRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            localAppDataLowOverride,
            xdgDataHomeOverride,
            userProfileOverride,
            executableDirectoryOverride,
            writeProbeOverride,
            integrityOverride);

        if (!resolution.Success)
        {
            throw new IOException(resolution.ErrorMessage ?? L.Get("Agp_FallbackNoWritable"));
        }

        if (noticeOutput is not null)
        {
            var notice = FormatCliFallbackNotice(resolution);
            if (!string.IsNullOrWhiteSpace(notice))
            {
                noticeOutput.WriteLine(notice);
            }
        }

        return resolution;
    }

    /// <summary>
    /// Validates a manually chosen folder by running the same write probe and best-effort persisting
    /// the path to &lt;exeDir&gt;\agp-location.txt (never failing if the exe directory is read-only).
    /// </summary>
    internal static AgpRootResolution TryAcceptUserSelectedAgpRoot(
        string? rawSelectedDirectory,
        ProcessIntegrityInfo integrity,
        IReadOnlyList<AgpCandidateFailure>? priorFailures = null,
        string? executableDirectoryOverride = null,
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null,
        bool? isWindowsOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var primaryLabel = isWindows ? "%LOCALAPPDATA%" : L.Get("Agp_DefaultUserDataDir");
        var existingFailures = priorFailures is null ? new List<AgpCandidateFailure>() : [.. priorFailures];
        var primaryFailure = existingFailures.FirstOrDefault(f => f.Source == AgpRootSource.LocalAppData)?.Reason;

        if (!TryNormalizePath(rawSelectedDirectory, out var normalized))
        {
            return new AgpRootResolution(
                Success: false,
                AgpRoot: null,
                Source: AgpRootSource.UserSelected,
                Integrity: integrity,
                UsedFallback: true,
                FallbackReason: L.Format("Agp_PrimaryNotWritable", primaryLabel),
                PrimaryFailureReason: primaryFailure,
                Failures: existingFailures,
                ErrorMessage: L.Format("Agp_SelectedInvalidPath", rawSelectedDirectory),
                IsWindowsContext: isWindows);
        }

        var probe = writeProbeOverride ?? ProbeDirectoryWritable;
        var probeResult = InvokeProbeSafely(probe, normalized);
        if (!probeResult.IsWritable)
        {
            var reason = string.IsNullOrWhiteSpace(probeResult.ErrorMessage) ? L.Get("Common_NotWritable") : probeResult.ErrorMessage!;
            existingFailures.Add(new AgpCandidateFailure(AgpRootSource.UserSelected, L.Get("Agp_ManualSelectionLabel"), normalized, reason));
            return new AgpRootResolution(
                Success: false,
                AgpRoot: null,
                Source: AgpRootSource.UserSelected,
                Integrity: integrity,
                UsedFallback: true,
                FallbackReason: L.Format("Agp_PrimaryNotWritable", primaryLabel),
                PrimaryFailureReason: primaryFailure,
                Failures: existingFailures,
                ErrorMessage: L.Format("Agp_SelectedNotWritable", normalized, reason),
                IsWindowsContext: isWindows);
        }

        var exeDir = ResolveExecutableDirectory(executableDirectoryOverride);
        var saved = TrySaveLocationFile(exeDir, normalized, out var saveError);

        return new AgpRootResolution(
            Success: true,
            AgpRoot: normalized,
            Source: AgpRootSource.UserSelected,
            Integrity: integrity,
            UsedFallback: true,
            FallbackReason: L.Format("Agp_PrimaryNotWritable", primaryLabel),
            PrimaryFailureReason: primaryFailure,
            Failures: existingFailures,
            ErrorMessage: null,
            LocationFileSaved: saved,
            LocationFileSaveError: saveError,
            IsWindowsContext: isWindows);
    }

    /// <summary>
    /// Verifies that an agp root candidate is genuinely writable by creating &lt;agpRoot&gt;\ffr-export\tmp\,
    /// writing a small temporary probe file, and deleting it immediately.
    /// </summary>
    internal static AgpWriteProbeResult ProbeDirectoryWritable(string agpRootPath)
    {
        if (!TryNormalizePath(agpRootPath, out var normalizedRoot))
        {
            return new AgpWriteProbeResult(false, L.Get("Common_InvalidPath"));
        }

        var tmpDir = GetTempDirectoryForAgpRoot(normalizedRoot);
        var probeFile = Path.Combine(
            tmpDir,
            $".write-probe-{Environment.ProcessId}-{Guid.NewGuid():N}.tmp");

        try
        {
            var ioTmpDir = PathSupport.ToIoPath(tmpDir);
            if (File.Exists(ioTmpDir))
            {
                return new AgpWriteProbeResult(false, L.Format("Agp_PathOccupiedByFile", tmpDir));
            }

            Directory.CreateDirectory(ioTmpDir);

            var ioProbeFile = PathSupport.ToIoPath(probeFile);
            using (var stream = new FileStream(
                ioProbeFile,
                FileMode.CreateNew,
                FileAccess.Write,
                FileShare.None))
            {
                ReadOnlySpan<byte> payload = "ok\n"u8;
                stream.Write(payload);
                stream.Flush();
            }

            File.Delete(ioProbeFile);
            return new AgpWriteProbeResult(true, null);
        }
        catch (Exception exception)
        {
            TryDeleteFileQuietly(probeFile);
            return new AgpWriteProbeResult(false, $"{exception.GetType().Name}: {exception.Message}");
        }
    }

    private static string ResolveLocalizedFallbackReason(AgpRootResolution resolution)
    {
        if (resolution.IsWindowsContext.HasValue)
        {
            var primaryLabel = resolution.IsWindowsContext.Value
                ? "%LOCALAPPDATA%"
                : L.Get("Agp_DefaultUserDataDir");
            return L.Format("Agp_PrimaryNotWritable", primaryLabel);
        }

        return string.IsNullOrWhiteSpace(resolution.FallbackReason)
            ? L.Format("Agp_PrimaryNotWritable", "%LOCALAPPDATA%")
            : resolution.FallbackReason;
    }

    internal static string? FormatCliFallbackNotice(AgpRootResolution resolution)
    {
        ArgumentNullException.ThrowIfNull(resolution);
        if (!resolution.Success || string.IsNullOrWhiteSpace(resolution.ToolRoot))
        {
            return null;
        }

        var lines = new List<string>();
        if (resolution.UsedFallback)
        {
            var reason = ResolveLocalizedFallbackReason(resolution);
            lines.Add(L.Format("Agp_CliNoticeDataDir", resolution.ToolRoot, reason));
        }

        if (resolution.Integrity.IsLowOrUntrusted)
        {
            lines.Add(L.Get("Agp_CliNoticeLowIl"));
        }

        return lines.Count > 0 ? string.Join(Environment.NewLine, lines) : null;
    }

    internal static string? FormatGuiBannerMessage(AgpRootResolution resolution)
    {
        ArgumentNullException.ThrowIfNull(resolution);
        if (!resolution.Success || string.IsNullOrWhiteSpace(resolution.ToolRoot))
        {
            return null;
        }

        if (resolution.Integrity.IsLowOrUntrusted)
        {
            if (resolution.Source == AgpRootSource.LocalAppDataLow)
            {
                return L.Format("Agp_GuiBannerLowIlLocalLow", resolution.Integrity.DisplayText, resolution.ToolRoot);
            }

            if (resolution.UsedFallback)
            {
                var reason = ResolveLocalizedFallbackReason(resolution);
                return L.Format("Agp_GuiBannerLowIlFallback", resolution.Integrity.DisplayText, resolution.ToolRoot, reason);
            }

            return L.Format("Agp_GuiBannerLowIl", resolution.Integrity.DisplayText);
        }

        if (resolution.UsedFallback)
        {
            var reason = ResolveLocalizedFallbackReason(resolution);
            return L.Format("Agp_CliNoticeDataDir", resolution.ToolRoot, reason);
        }

        return null;
    }

    internal static string FormatAllCandidatesFailedGuiMessage(AgpRootResolution resolution)
    {
        ArgumentNullException.ThrowIfNull(resolution);
        var sb = new StringBuilder();
        sb.AppendLine(L.Get("Agp_GuiAllFailedIntro"));
        sb.AppendLine();

        if (resolution.Integrity.IsLowOrUntrusted)
        {
            sb.AppendLine(L.Format("Agp_GuiAllFailedLowIl1", resolution.Integrity.DisplayText));
            sb.AppendLine(L.Get("Agp_GuiAllFailedLowIl2"));
            sb.AppendLine();
        }
        else
        {
            sb.AppendLine(L.Get("Agp_GuiAllFailedAv1"));
            sb.AppendLine(L.Get("Agp_GuiAllFailedAv2"));
            sb.AppendLine();
        }

        if (resolution.Failures.Count > 0)
        {
            sb.AppendLine(L.Get("Agp_GuiAllFailedTriedHeader"));
            foreach (var failure in resolution.Failures)
            {
                sb.AppendLine(L.Format("Agp_GuiAllFailedTriedItem", failure.Label, failure.Path, failure.Reason));
            }

            sb.AppendLine();
        }

        sb.AppendLine(L.Get("Agp_GuiAllFailedAction"));
        sb.Append(L.Get("Agp_GuiAllFailedLastResort"));
        return sb.ToString();
    }

    internal static string FormatAllCandidatesFailedCliMessage(
        IReadOnlyList<AgpCandidateFailure> failures,
        ProcessIntegrityInfo integrity,
        bool isWindows)
    {
        var sb = new StringBuilder();
        sb.Append(L.Get("Agp_CliAllFailedPrefix"));
        sb.Append(integrity.IsLowOrUntrusted
            ? L.Get("Agp_CliAllFailedLowIl")
            : L.Get("Agp_CliAllFailedAv"));

        sb.Append(isWindows
            ? L.Format("Agp_CliAllFailedEnvWindows", AgpHomeEnvVar)
            : L.Format("Agp_CliAllFailedEnvUnix", AgpHomeEnvVar));

        if (failures.Count > 0)
        {
            sb.Append(L.Get("Agp_CliAllFailedDetailsPrefix"));
            sb.Append(string.Join(L.Get("Common_SemicolonSeparator"), failures.Select(f => $"{f.Label}({f.Path})={f.Reason}")));
            sb.Append(L.Get("Agp_CliAllFailedDetailsSuffix"));
        }

        sb.Append(L.Get("Agp_CliAllFailedLastResort"));
        return sb.ToString();
    }

    internal static string GetSavedLocationFilePath(string? executableDirectoryOverride = null)
    {
        var exeDir = ResolveExecutableDirectory(executableDirectoryOverride);
        return Path.Combine(exeDir, SavedLocationFileName);
    }

    internal static string? TryReadLocationFile(string? executableDirectoryOverride = null)
    {
        try
        {
            var locationPath = GetSavedLocationFilePath(executableDirectoryOverride);
            var ioPath = PathSupport.ToIoPath(locationPath);
            if (!File.Exists(ioPath))
            {
                return null;
            }

            var raw = File.ReadAllText(ioPath, TextOutput.Utf8WithoutBom);
            var firstLine = raw
                .Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries)
                .FirstOrDefault();
            if (string.IsNullOrWhiteSpace(firstLine))
            {
                return null;
            }

            return TryNormalizePath(firstLine, out var normalized) ? normalized : null;
        }
        catch
        {
            return null;
        }
    }

    internal static bool TrySaveLocationFile(
        string? executableDirectoryOverride,
        string agpRootPath,
        out string? errorMessage)
    {
        errorMessage = null;
        try
        {
            var locationPath = GetSavedLocationFilePath(executableDirectoryOverride);
            TextOutput.WriteText(locationPath, agpRootPath + TextOutput.LineEnding);
            return true;
        }
        catch (Exception exception)
        {
            errorMessage = $"{exception.GetType().Name}: {exception.Message}";
            return false;
        }
    }

    /// <summary>
    /// Resolves the root "agp" directory using the writable candidate priority order.
    /// Throws IOException if AGP_HOME is unwritable or no candidate directory is writable.
    /// </summary>
    internal static string ResolveAgpHome(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null,
        string? localAppDataLowOverride = null,
        string? executableDirectoryOverride = null,
        Func<string, AgpWriteProbeResult>? writeProbeOverride = null)
    {
        var resolved = ResolveWritableAgpRoot(
            agpHomeEnvOverride: agpHomeEnvOverride,
            isWindowsOverride: isWindowsOverride,
            localAppDataOverride: localAppDataOverride,
            localAppDataLowOverride: localAppDataLowOverride,
            xdgDataHomeOverride: xdgDataHomeOverride,
            userProfileOverride: userProfileOverride,
            executableDirectoryOverride: executableDirectoryOverride,
            writeProbeOverride: writeProbeOverride);

        if (!resolved.Success || string.IsNullOrWhiteSpace(resolved.AgpRoot))
        {
            throw new IOException(resolved.ErrorMessage ?? L.Get("Agp_ResolveFailedFallback"));
        }

        return resolved.AgpRoot;
    }

    internal static string ResolveToolRootForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(Path.GetFullPath(Path.Combine(agpRoot, ToolFolderName)));
    }

    internal static string GetDefaultOutputRootForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), OutputSubfolderName)));
    }

    internal static string GetUsmapDirectoryForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), UsmapSubfolderName)));
    }

    internal static string GetDownloadsDirectoryForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), DownloadsSubfolderName)));
    }

    internal static string GetLogsDirectoryForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), LogsSubfolderName)));
    }

    internal static string GetTempDirectoryForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), TempSubfolderName)));
    }

    /// <summary>
    /// Ensures the tool tmp subdirectory exists under the resolved data root and redirects
    /// process-local TEMP/TMP environment variables to it on Windows so framework internals
    /// (such as single-file WinForms Application.EnableVisualStyles activation context) never
    /// attempt to write temporary files into system %TEMP%.
    /// </summary>
    internal static bool TryConfigureProcessTempDirectory(
        AgpRootResolution resolution,
        out string? tempDirectory,
        out string? error,
        bool? isWindowsOverride = null)
    {
        tempDirectory = null;
        error = null;
        if (!resolution.Success || string.IsNullOrWhiteSpace(resolution.AgpRoot))
        {
            error = L.Get("Agp_TempRootNotReady");
            return false;
        }

        return TryConfigureProcessTempDirectoryForAgpRoot(
            resolution.AgpRoot,
            out tempDirectory,
            out error,
            isWindowsOverride);
    }

    internal static bool TryConfigureProcessTempDirectoryForAgpRoot(
        string agpRoot,
        out string? tempDirectory,
        out string? error,
        bool? isWindowsOverride = null)
    {
        tempDirectory = null;
        error = null;
        if (!TryNormalizePath(agpRoot, out var normalizedRoot))
        {
            error = L.Get("Agp_TempRootInvalid");
            return false;
        }

        try
        {
            var tmpDir = GetTempDirectoryForAgpRoot(normalizedRoot);
            Directory.CreateDirectory(PathSupport.ToIoPath(tmpDir));
            if (isWindowsOverride ?? OperatingSystem.IsWindows())
            {
                Environment.SetEnvironmentVariable("TEMP", tmpDir);
                Environment.SetEnvironmentVariable("TMP", tmpDir);
            }

            tempDirectory = tmpDir;
            return true;
        }
        catch (Exception exception)
        {
            error = $"{exception.GetType().Name}: {exception.Message}";
            return false;
        }
    }

    internal static string GetUe4ssBackupDirectoryForAgpRoot(string agpRoot)
    {
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), Ue4ssBackupSubfolderName)));
    }

    internal static string GetSettingsFilePathForAgpRoot(string agpRoot)
    {
        return Path.GetFullPath(Path.Combine(ResolveToolRootForAgpRoot(agpRoot), SettingsFileName));
    }

    internal static string ResolveToolRoot(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return ResolveToolRootForAgpRoot(agpHome);
    }

    internal static string GetDefaultOutputRoot(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetDefaultOutputRootForAgpRoot(agpHome);
    }

    internal static string GetDefaultOutputDirectory(
        string gameCode,
        string? steamBuildId,
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var outputRoot = GetDefaultOutputRoot(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        var normalizedCode = SanitizePathSegment(gameCode) ?? "ffr";
        var buildSegment = SanitizePathSegment(steamBuildId) ?? "unknown";
        return PathSupport.TrimTrailingSeparators(
            Path.GetFullPath(Path.Combine(outputRoot, $"{normalizedCode}-{buildSegment}")));
    }

    internal static string GetUsmapDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetUsmapDirectoryForAgpRoot(agpHome);
    }

    internal static string GetDownloadsDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetDownloadsDirectoryForAgpRoot(agpHome);
    }

    internal static string GetLogsDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetLogsDirectoryForAgpRoot(agpHome);
    }

    internal static string GetTempDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetTempDirectoryForAgpRoot(agpHome);
    }

    internal static string GetUe4ssBackupDirectory(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetUe4ssBackupDirectoryForAgpRoot(agpHome);
    }

    internal static string GetSettingsFilePath(
        string? agpHomeEnvOverride = null,
        bool? isWindowsOverride = null,
        string? localAppDataOverride = null,
        string? xdgDataHomeOverride = null,
        string? userProfileOverride = null)
    {
        var agpHome = ResolveAgpHome(
            agpHomeEnvOverride,
            isWindowsOverride,
            localAppDataOverride,
            xdgDataHomeOverride,
            userProfileOverride);
        return GetSettingsFilePathForAgpRoot(agpHome);
    }

    internal static string? SanitizePathSegment(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        var sb = new StringBuilder(value.Length);
        foreach (var ch in value.Trim())
        {
            if (char.IsLetterOrDigit(ch) || ch is '-' or '_' or '.')
            {
                sb.Append(ch);
            }
        }

        return sb.Length > 0 ? sb.ToString() : null;
    }

    internal static bool TryNormalizePath(string? rawPath, out string normalizedPath)
    {
        normalizedPath = string.Empty;
        if (string.IsNullOrWhiteSpace(rawPath))
        {
            return false;
        }

        var trimmed = rawPath.Trim();
        if (trimmed.IndexOf('\0') >= 0)
        {
            return false;
        }

        try
        {
            var full = PathSupport.TrimTrailingSeparators(Path.GetFullPath(trimmed));
            if (string.IsNullOrWhiteSpace(full))
            {
                return false;
            }

            normalizedPath = full;
            return true;
        }
        catch
        {
            return false;
        }
    }

    private sealed record CandidateEntry(AgpRootSource Source, string Label, string Path);

    private static List<CandidateEntry> BuildOrderedFallbackCandidates(
        bool isWindows,
        string? localAppDataOverride,
        string? localAppDataLowOverride,
        string? xdgDataHomeOverride,
        string? userProfileOverride,
        string exeDir,
        List<AgpCandidateFailure> initialFailures)
    {
        var candidates = new List<CandidateEntry>();
        var comparer = isWindows ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        void TryAdd(AgpRootSource source, string label, string? rawPath)
        {
            if (string.IsNullOrWhiteSpace(rawPath))
            {
                return;
            }

            if (TryNormalizePath(rawPath, out var normalized) && seen.Add(normalized))
            {
                candidates.Add(new CandidateEntry(source, label, normalized));
            }
        }

        var userProfile = userProfileOverride is not null
            ? userProfileOverride
            : ResolveUserProfile(isWindows);

        if (isWindows)
        {
            // Candidate 2: %LOCALAPPDATA%\agp
            var localAppData = localAppDataOverride is not null
                ? localAppDataOverride
                : SafeGetFolderPath(Environment.SpecialFolder.LocalApplicationData);
            if (string.IsNullOrWhiteSpace(localAppData) && localAppDataOverride is null)
            {
                localAppData = SafeGetEnvironmentVariable("LOCALAPPDATA");
            }

            if (!string.IsNullOrWhiteSpace(localAppData))
            {
                TryAdd(AgpRootSource.LocalAppData, "%LOCALAPPDATA%\\agp", Path.Combine(localAppData.Trim(), AgpFolderName));
            }
            else
            {
                initialFailures.Add(new AgpCandidateFailure(
                    AgpRootSource.LocalAppData,
                    "%LOCALAPPDATA%\\agp",
                    L.Get("Common_NotSet"),
                    L.Get("Agp_CannotReadLocalAppData")));
            }

            // Candidate 3: %USERPROFILE%\AppData\LocalLow\agp
            var localAppDataLow = localAppDataLowOverride is not null
                ? localAppDataLowOverride
                : ResolveWindowsLocalAppDataLow(userProfile);
            if (!string.IsNullOrWhiteSpace(localAppDataLow))
            {
                TryAdd(
                    AgpRootSource.LocalAppDataLow,
                    "%USERPROFILE%\\AppData\\LocalLow\\agp",
                    Path.Combine(localAppDataLow.Trim(), AgpFolderName));
            }
        }
        else
        {
            // Candidate 2 (Linux/macOS): $XDG_DATA_HOME/agp, then ~/.local/share/agp
            var xdgDataHome = xdgDataHomeOverride is not null
                ? xdgDataHomeOverride
                : SafeGetEnvironmentVariable("XDG_DATA_HOME");
            var addedPrimary = false;
            if (!string.IsNullOrWhiteSpace(xdgDataHome))
            {
                TryAdd(AgpRootSource.LocalAppData, "$XDG_DATA_HOME/agp", Path.Combine(xdgDataHome.Trim(), AgpFolderName));
                addedPrimary = true;
            }

            if (!string.IsNullOrWhiteSpace(userProfile))
            {
                TryAdd(
                    AgpRootSource.LocalAppData,
                    "~/.local/share/agp",
                    Path.Combine(userProfile.Trim(), ".local", "share", AgpFolderName));
                addedPrimary = true;
            }

            if (!addedPrimary)
            {
                initialFailures.Add(new AgpCandidateFailure(
                    AgpRootSource.LocalAppData,
                    "~/.local/share/agp",
                    L.Get("Common_NotSet"),
                    L.Get("Agp_CannotReadXdgOrHome")));
            }
        }

        // Candidate 4 (Windows) / 3 (Unix): <exeDir>\agp-data (portable mode)
        if (!string.IsNullOrWhiteSpace(exeDir))
        {
            TryAdd(
                AgpRootSource.PortableExeDir,
                isWindows ? L.Get("Agp_PortableLabelWindows") : L.Get("Agp_PortableLabelUnix"),
                Path.Combine(exeDir, PortableAgpFolderName));
        }

        // Candidate 5 (Windows) / 4 (Unix): %USERPROFILE%\agp
        if (!string.IsNullOrWhiteSpace(userProfile))
        {
            TryAdd(
                AgpRootSource.UserProfile,
                isWindows ? "%USERPROFILE%\\agp" : "~/agp",
                Path.Combine(userProfile.Trim(), AgpFolderName));
        }

        // Candidate 6: Saved custom location from <exeDir>\agp-location.txt
        var savedLocation = TryReadLocationFile(exeDir);
        if (!string.IsNullOrWhiteSpace(savedLocation))
        {
            TryAdd(AgpRootSource.SavedLocationFile, SavedLocationFileName, savedLocation);
        }

        return candidates;
    }

    private static string ResolveExecutableDirectory(string? executableDirectoryOverride)
    {
        if (!string.IsNullOrWhiteSpace(executableDirectoryOverride)
            && TryNormalizePath(executableDirectoryOverride, out var normalizedOverride))
        {
            return normalizedOverride;
        }

        if (!string.IsNullOrWhiteSpace(AppContext.BaseDirectory)
            && TryNormalizePath(AppContext.BaseDirectory, out var normalizedBase))
        {
            return normalizedBase;
        }

        try
        {
            return PathSupport.TrimTrailingSeparators(Directory.GetCurrentDirectory());
        }
        catch
        {
            return string.Empty;
        }
    }

    private static string? ResolveUserProfile(bool isWindows)
    {
        var profile = SafeGetFolderPath(Environment.SpecialFolder.UserProfile);
        if (!string.IsNullOrWhiteSpace(profile))
        {
            return profile;
        }

        return isWindows
            ? SafeGetEnvironmentVariable("USERPROFILE")
            : SafeGetEnvironmentVariable("HOME");
    }

    private static string? ResolveWindowsLocalAppDataLow(string? userProfile)
    {
        if (OperatingSystem.IsWindows())
        {
            var known = TryGetWindowsKnownFolderPath(FolderIdLocalAppDataLow);
            if (!string.IsNullOrWhiteSpace(known))
            {
                return known;
            }
        }

        if (!string.IsNullOrWhiteSpace(userProfile))
        {
            return Path.Combine(userProfile.Trim(), "AppData", "LocalLow");
        }

        return null;
    }

    private static string? TryGetWindowsKnownFolderPath(Guid folderId)
    {
        var pathPtr = IntPtr.Zero;
        try
        {
            var hr = SHGetKnownFolderPath(in folderId, 0, IntPtr.Zero, out pathPtr);
            if (hr == 0 && pathPtr != IntPtr.Zero)
            {
                return Marshal.PtrToStringUni(pathPtr);
            }

            return null;
        }
        catch
        {
            return null;
        }
        finally
        {
            if (pathPtr != IntPtr.Zero)
            {
                Marshal.FreeCoTaskMem(pathPtr);
            }
        }
    }

    private static AgpWriteProbeResult InvokeProbeSafely(
        Func<string, AgpWriteProbeResult> probe,
        string path)
    {
        try
        {
            return probe(path);
        }
        catch (Exception exception)
        {
            return new AgpWriteProbeResult(false, $"{exception.GetType().Name}: {exception.Message}");
        }
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
            // Ignore probe cleanup errors.
        }
    }

    private static string? SafeGetFolderPath(Environment.SpecialFolder folder)
    {
        try
        {
            return Environment.GetFolderPath(folder);
        }
        catch
        {
            return null;
        }
    }

    private static string? SafeGetEnvironmentVariable(string name)
    {
        try
        {
            return Environment.GetEnvironmentVariable(name);
        }
        catch
        {
            return null;
        }
    }

    [DllImport("shell32.dll")]
    private static extern int SHGetKnownFolderPath(
        in Guid rfid,
        uint dwFlags,
        IntPtr hToken,
        out IntPtr ppszPath);
}
