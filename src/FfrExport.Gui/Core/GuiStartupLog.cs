using System.Diagnostics;
using System.Globalization;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.RegularExpressions;

namespace FfrExport.Gui.Core;

internal sealed record GuiStartupInfo(
    string Version,
    string GitCommit,
    string OsVersion,
    string DotNetVersion,
    string ProcessBitness,
    string CurrentDirectory,
    string ExecutablePath,
    string DpiSetting,
    string Mode,
    string IntegrityLevel = "Unknown",
    string? DataRoot = null);

/// <summary>
/// Active startup diagnostic log session for the GUI process.
/// All write operations are best-effort and never throw back into callers.
/// </summary>
internal sealed class GuiStartupLogSession
{
    private readonly object _writeLock = new();

    internal GuiStartupLogSession(
        string? logFilePath,
        string? logDirectory,
        bool usedFallbackDirectory,
        string? primaryDirectoryError,
        bool isWritable,
        string? lastWriteError)
    {
        LogFilePath = logFilePath;
        LogDirectory = logDirectory;
        UsedFallbackDirectory = usedFallbackDirectory;
        PrimaryDirectoryError = primaryDirectoryError;
        IsWritable = isWritable;
        LastWriteError = lastWriteError;
    }

    internal string? LogFilePath { get; }

    internal string? LogDirectory { get; }

    internal bool UsedFallbackDirectory { get; }

    internal string? PrimaryDirectoryError { get; }

    internal bool IsWritable { get; private set; }

    internal string? LastWriteError { get; private set; }

    internal void WriteInfo(string message)
    {
        AppendLine("INFO", message);
    }

    internal void WriteWarning(string message)
    {
        AppendLine("WARN", message);
    }

    internal void WriteException(string context, Exception exception)
    {
        var formatted = GuiStartupLog.FormatExceptionDetails(context, exception);
        AppendRawText(formatted);
    }

    internal void WriteFatalObject(string context, object? exceptionObject)
    {
        if (exceptionObject is Exception exception)
        {
            WriteException(context, exception);
            return;
        }

        var timestamp = DateTimeOffset.Now.ToString("yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        var text = $"[{timestamp}] [ERROR] 阶段：{context}{TextOutput.LineEnding}"
            + $"非 Exception 未处理异常对象：{exceptionObject?.ToString() ?? "(null)"}{TextOutput.LineEnding}";
        AppendRawText(text);
    }

    internal void AppendLine(string level, string message)
    {
        var timestamp = DateTimeOffset.Now.ToString("yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        var sanitized = GuiStartupLog.SanitizeSingleLine(message);
        AppendRawText($"[{timestamp}] [{level}] {sanitized}{TextOutput.LineEnding}");
    }

    private void AppendRawText(string text)
    {
        if (string.IsNullOrWhiteSpace(LogFilePath))
        {
            return;
        }

        lock (_writeLock)
        {
            try
            {
                TextOutput.AppendText(LogFilePath, text);
            }
            catch (Exception exception)
            {
                IsWritable = false;
                LastWriteError = $"{exception.GetType().Name}: {exception.Message}";
            }
        }
    }
}

/// <summary>
/// Manages GUI startup log files under &lt;selectedAgpRoot&gt;\ffr-export\logs\gui-&lt;yyyyMMdd-HHmmss&gt;.log,
/// retains only the latest 10 log files, and formats fatal crash diagnostics.
/// Never writes to %TEMP%; if no selected data root is writable, logs stay in-memory/dialog only.
/// </summary>
internal static class GuiStartupLog
{
    internal const string AppFolderName = AgpPaths.ToolFolderName;
    internal const string LogsSubfolderName = AgpPaths.LogsSubfolderName;
    internal const string LogFilePrefix = "gui-";
    internal const string LogFileExtension = ".log";
    internal const int DefaultMaxRetainedLogs = 10;

    private const int MaxInnerExceptionDepth = 16;

    // Restrict pruning strictly to files matching our timestamped gui-*.log / cli-*.log naming scheme.
    private static readonly Regex ManagedLogFileRegex = new(
        @"^(?:gui|cli)-\d{8}-\d{6}(?:-[0-9A-Za-z-]+)?\.log$",
        RegexOptions.IgnoreCase | RegexOptions.CultureInvariant | RegexOptions.Compiled);

    internal static GuiStartupInfo CollectStartupInfo(
        Assembly? entryAssembly,
        string? dpiSetting,
        string mode = "gui",
        ProcessIntegrityInfo? integrityInfo = null,
        string? dataRoot = null)
    {
        var (version, gitCommit) = ReadBuildMetadata(entryAssembly);

        string osVersion;
        try
        {
            osVersion = $"{RuntimeInformation.OSDescription.Trim()} ({Environment.OSVersion.VersionString})";
        }
        catch (Exception exception)
        {
            osVersion = $"<unavailable: {exception.Message}>";
        }

        string dotNetVersion;
        try
        {
            dotNetVersion = $"{RuntimeInformation.FrameworkDescription.Trim()} ({Environment.Version})";
        }
        catch (Exception exception)
        {
            dotNetVersion = $"<unavailable: {exception.Message}>";
        }

        string processBitness;
        try
        {
            var bits = Environment.Is64BitProcess ? "64-bit" : "32-bit";
            processBitness = $"{bits} ({RuntimeInformation.ProcessArchitecture})";
        }
        catch (Exception exception)
        {
            processBitness = $"<unavailable: {exception.Message}>";
        }

        string currentDirectory;
        try
        {
            currentDirectory = Directory.GetCurrentDirectory();
        }
        catch (Exception exception)
        {
            currentDirectory = $"<unavailable: {exception.Message}>";
        }

        var executablePath = ResolveExecutablePath();
        var normalizedDpi = string.IsNullOrWhiteSpace(dpiSetting) ? "unknown" : dpiSetting.Trim();
        var normalizedMode = string.IsNullOrWhiteSpace(mode) ? "gui" : mode.Trim();
        var integrity = integrityInfo ?? ProcessIntegritySupport.DetectCurrent();

        return new GuiStartupInfo(
            version,
            gitCommit,
            osVersion,
            dotNetVersion,
            processBitness,
            currentDirectory,
            executablePath,
            normalizedDpi,
            normalizedMode,
            integrity.FormatForLog(),
            dataRoot);
    }

    internal static string GetPrimaryLogDirectory(
        string? localAppDataOverride = null,
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null)
    {
        try
        {
            if (agpHomeOverride is not null)
            {
                if (string.IsNullOrWhiteSpace(agpHomeOverride)
                    || !AgpPaths.TryNormalizePath(agpHomeOverride, out var normalizedAgp))
                {
                    return string.Empty;
                }

                return AgpPaths.GetLogsDirectoryForAgpRoot(normalizedAgp);
            }

            if (localAppDataOverride is not null)
            {
                if (string.IsNullOrWhiteSpace(localAppDataOverride)
                    || !AgpPaths.TryNormalizePath(Path.Combine(localAppDataOverride.Trim(), AgpPaths.AgpFolderName), out var winAgp))
                {
                    return string.Empty;
                }

                return AgpPaths.GetLogsDirectoryForAgpRoot(winAgp);
            }

            var resolved = AgpPaths.ResolveWritableAgpRoot(isWindowsOverride: isWindowsOverride);
            return resolved.Success && !string.IsNullOrWhiteSpace(resolved.AgpRoot)
                ? AgpPaths.GetLogsDirectoryForAgpRoot(resolved.AgpRoot)
                : string.Empty;
        }
        catch
        {
            return string.Empty;
        }
    }

    internal static string BuildLogFileName(DateTimeOffset timestamp, int? collisionSuffix = null)
    {
        var stamp = timestamp.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
        return collisionSuffix.HasValue
            ? string.Create(CultureInfo.InvariantCulture, $"{LogFilePrefix}{stamp}-{collisionSuffix.Value}{LogFileExtension}")
            : $"{LogFilePrefix}{stamp}{LogFileExtension}";
    }

    internal static string FormatStartupLine(GuiStartupInfo info, DateTimeOffset? timestamp = null)
    {
        var ts = (timestamp ?? DateTimeOffset.Now).ToString("yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        var dataRootPart = string.IsNullOrWhiteSpace(info.DataRoot)
            ? string.Empty
            : $" data_root=\"{SanitizeQuoted(info.DataRoot)}\"";
        return string.Create(
            CultureInfo.InvariantCulture,
            $"[{ts}] [START] mode={SanitizeSingleLine(info.Mode)} "
            + $"version={SanitizeSingleLine(info.Version)} "
            + $"git={SanitizeSingleLine(info.GitCommit)} "
            + $"integrity=\"{SanitizeQuoted(info.IntegrityLevel)}\" "
            + $"os=\"{SanitizeQuoted(info.OsVersion)}\" "
            + $"dotnet=\"{SanitizeQuoted(info.DotNetVersion)}\" "
            + $"bits=\"{SanitizeQuoted(info.ProcessBitness)}\" "
            + $"dpi=\"{SanitizeQuoted(info.DpiSetting)}\" "
            + $"cwd=\"{SanitizeQuoted(info.CurrentDirectory)}\" "
            + $"exe=\"{SanitizeQuoted(info.ExecutablePath)}\""
            + $"{dataRootPart}");
    }

    internal static GuiStartupLogSession StartSession(
        GuiStartupInfo startupInfo,
        DateTimeOffset? nowOverride = null,
        AgpRootResolution? agpRootResolution = null,
        string? logsDirectoryOverride = null,
        int maxRetainedLogs = DefaultMaxRetainedLogs,
        Action<string>? directoryCreatorOverride = null)
    {
        var now = nowOverride ?? DateTimeOffset.Now;
        var effectiveResolution = agpRootResolution;
        string? targetDir;
        string? primaryError = null;
        var usedFallback = false;

        if (logsDirectoryOverride is not null)
        {
            targetDir = string.IsNullOrWhiteSpace(logsDirectoryOverride)
                ? null
                : PathSupport.TrimTrailingSeparators(Path.GetFullPath(logsDirectoryOverride));
            if (targetDir is null)
            {
                primaryError = "未指定可写的日志目录";
            }
        }
        else
        {
            effectiveResolution ??= AgpPaths.ResolveWritableAgpRoot();
            usedFallback = effectiveResolution.UsedFallback;
            primaryError = effectiveResolution.PrimaryFailureReason;
            if (effectiveResolution.Success && !string.IsNullOrWhiteSpace(effectiveResolution.AgpRoot))
            {
                targetDir = AgpPaths.GetLogsDirectoryForAgpRoot(effectiveResolution.AgpRoot);
            }
            else
            {
                targetDir = null;
                primaryError = effectiveResolution.ErrorMessage ?? "没有可写的数据目录";
            }
        }

        if (string.IsNullOrWhiteSpace(targetDir))
        {
            return new GuiStartupLogSession(
                logFilePath: null,
                logDirectory: null,
                usedFallbackDirectory: usedFallback,
                primaryDirectoryError: primaryError,
                isWritable: false,
                lastWriteError: primaryError ?? "没有可写的数据目录，未写入日志文件");
        }

        var enrichedInfo = string.IsNullOrWhiteSpace(startupInfo.DataRoot) && effectiveResolution?.ToolRoot is not null
            ? startupInfo with { DataRoot = effectiveResolution.ToolRoot }
            : startupInfo;
        var startupLine = FormatStartupLine(enrichedInfo, now);
        var extraLines = BuildStartupResolutionDiagnosticLines(effectiveResolution, now);

        if (TryCreateLogFile(
            targetDir,
            now,
            startupLine,
            extraLines,
            directoryCreatorOverride,
            out var logPath,
            out var writeError))
        {
            PruneOldLogs(targetDir, logPath, maxRetainedLogs);
            return new GuiStartupLogSession(
                logPath,
                targetDir,
                usedFallbackDirectory: usedFallback,
                primaryDirectoryError: primaryError,
                isWritable: true,
                lastWriteError: null);
        }

        return new GuiStartupLogSession(
            logFilePath: null,
            logDirectory: null,
            usedFallbackDirectory: usedFallback,
            primaryDirectoryError: primaryError ?? writeError,
            isWritable: false,
            lastWriteError: writeError ?? "无法创建启动日志文件");
    }

    /// <summary>
    /// Deletes older gui-*.log files in logDirectory so at most maxRetainedLogs remain,
    /// always keeping keepFilePath when specified. Never throws on locked or inaccessible files.
    /// </summary>
    internal static int PruneOldLogs(
        string logDirectory,
        string? keepFilePath = null,
        int maxRetainedLogs = DefaultMaxRetainedLogs)
    {
        if (string.IsNullOrWhiteSpace(logDirectory) || maxRetainedLogs <= 0)
        {
            return 0;
        }

        var normalizedKeep = NormalizePathForComparison(keepFilePath);
        var entries = new List<(string FullPath, string FileName, DateTime LastWriteUtc, bool IsKeep)>();

        try
        {
            var ioDir = PathSupport.ToIoPath(logDirectory);
            if (!Directory.Exists(ioDir))
            {
                return 0;
            }

            foreach (var rawPath in Directory.EnumerateFiles(ioDir, $"*{LogFileExtension}", SearchOption.TopDirectoryOnly))
            {
                var fileName = Path.GetFileName(rawPath);
                if (!ManagedLogFileRegex.IsMatch(fileName))
                {
                    continue;
                }

                string fullPath;
                try
                {
                    fullPath = Path.GetFullPath(rawPath);
                }
                catch
                {
                    continue;
                }

                var lastWriteUtc = DateTime.MinValue;
                try
                {
                    lastWriteUtc = File.GetLastWriteTimeUtc(PathSupport.ToIoPath(fullPath));
                }
                catch
                {
                    // Treat unreadable metadata as oldest.
                }

                var isKeep = normalizedKeep is not null
                    && string.Equals(NormalizePathForComparison(fullPath), normalizedKeep, StringComparison.Ordinal);
                entries.Add((fullPath, fileName, lastWriteUtc, isKeep));
            }
        }
        catch
        {
            return 0;
        }

        if (entries.Count <= maxRetainedLogs)
        {
            return 0;
        }

        var ordered = entries
            .OrderByDescending(item => item.IsKeep)
            .ThenByDescending(item => item.LastWriteUtc)
            .ThenByDescending(item => item.FileName, StringComparer.OrdinalIgnoreCase)
            .ToList();

        var deleted = 0;
        for (var i = maxRetainedLogs; i < ordered.Count; i++)
        {
            if (ordered[i].IsKeep)
            {
                continue;
            }

            try
            {
                File.Delete(PathSupport.ToIoPath(ordered[i].FullPath));
                deleted++;
            }
            catch
            {
                // Ignore files locked by another running instance.
            }
        }

        return deleted;
    }

    internal static string FormatExceptionDetails(
        string context,
        Exception exception,
        DateTimeOffset? timestamp = null)
    {
        ArgumentNullException.ThrowIfNull(exception);

        var ts = (timestamp ?? DateTimeOffset.Now).ToString("yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        var sb = new StringBuilder();
        sb.AppendLine($"[{ts}] [ERROR] 阶段：{SanitizeSingleLine(context)}");
        AppendExceptionBlock(sb, exception, depth: 0);
        return sb.ToString();
    }

    internal static string FormatCrashDialogMessage(
        Exception exception,
        string? logFilePath,
        string? logError = null,
        string headline = "窗口程序启动失败")
    {
        ArgumentNullException.ThrowIfNull(exception);

        var sb = new StringBuilder();
        sb.AppendLine($"{headline}。");
        sb.AppendLine();
        sb.AppendLine($"异常信息：{exception.GetType().Name}: {exception.Message}");

        if (exception.InnerException is not null)
        {
            sb.AppendLine(
                $"内部异常：{exception.InnerException.GetType().Name}: {exception.InnerException.Message}");
        }

        sb.AppendLine();
        if (!string.IsNullOrWhiteSpace(logFilePath))
        {
            sb.AppendLine($"日志文件：{logFilePath}");
            sb.Append("请将上述日志文件发送给开发者，以便定位问题。");
        }
        else
        {
            var reason = string.IsNullOrWhiteSpace(logError) ? "日志目录不可写" : logError;
            sb.AppendLine($"日志文件：未能写入日志文件（{reason}）");
            sb.Append("请将本错误窗口截图发送给开发者，以便定位问题。");
        }

        return sb.ToString();
    }

    internal static string FormatSelfTestFailureMessage(
        Exception exception,
        string? logFilePath,
        string? logError = null)
    {
        ArgumentNullException.ThrowIfNull(exception);

        var logLine = !string.IsNullOrWhiteSpace(logFilePath)
            ? $"日志文件：{logFilePath}"
            : $"日志文件：未能写入（{logError ?? "未知原因"}）";

        var sb = new StringBuilder();
        sb.AppendLine($"窗口程序自检失败：{exception.GetType().FullName}: {exception.Message}");
        sb.AppendLine(logLine);
        sb.Append(FormatExceptionDetails("self-test", exception));
        return sb.ToString();
    }

    internal static string SanitizeSingleLine(string? value)
    {
        if (string.IsNullOrEmpty(value))
        {
            return string.Empty;
        }

        return value.Replace('\r', ' ').Replace('\n', ' ').Trim();
    }

    private static List<string> BuildStartupResolutionDiagnosticLines(
        AgpRootResolution? resolution,
        DateTimeOffset timestamp)
    {
        var lines = new List<string>();
        if (resolution is null)
        {
            return lines;
        }

        var ts = timestamp.ToString("yyyy-MM-dd HH:mm:ss.fff zzz", CultureInfo.InvariantCulture);
        lines.Add(
            $"[{ts}] [INFO] 进程完整性级别：{SanitizeSingleLine(resolution.Integrity.DisplayTextZh)} " +
            $"({SanitizeSingleLine(resolution.Integrity.FormatForLog())})；数据目录来源={resolution.Source}；路径={SanitizeSingleLine(resolution.ToolRoot)}");

        if (resolution.Failures.Count > 0)
        {
            foreach (var failure in resolution.Failures)
            {
                lines.Add(
                    $"[{ts}] [WARN] 跳过不可写候选目录：{SanitizeSingleLine(failure.Label)} " +
                    $"({SanitizeSingleLine(failure.Path)})：{SanitizeSingleLine(failure.Reason)}");
            }
        }

        if (resolution.Source == AgpRootSource.UserSelected && !resolution.LocationFileSaved)
        {
            lines.Add(
                $"[{ts}] [WARN] 未能保存 {AgpPaths.SavedLocationFileName}（下次启动时需重新选择数据目录）：" +
                $"{SanitizeSingleLine(resolution.LocationFileSaveError ?? "程序目录不可写")}");
        }

        return lines;
    }

    private static string SanitizeQuoted(string? value)
    {
        return SanitizeSingleLine(value).Replace('"', '\'');
    }

    private static void AppendExceptionBlock(StringBuilder sb, Exception exception, int depth)
    {
        sb.AppendLine($"异常类型：{exception.GetType().FullName ?? exception.GetType().Name}");
        sb.AppendLine($"异常消息：{exception.Message}");
        sb.AppendLine(string.Create(CultureInfo.InvariantCulture, $"HResult：0x{exception.HResult:X8}"));
        sb.AppendLine("堆栈跟踪：");
        sb.AppendLine(string.IsNullOrWhiteSpace(exception.StackTrace) ? "(无堆栈信息)" : exception.StackTrace);

        if (depth >= MaxInnerExceptionDepth)
        {
            if (exception.InnerException is not null)
            {
                sb.AppendLine("...（InnerException 嵌套层数过多，已截断）");
            }

            return;
        }

        if (exception is AggregateException aggregate && aggregate.InnerExceptions.Count > 1)
        {
            for (var i = 0; i < aggregate.InnerExceptions.Count; i++)
            {
                sb.AppendLine(string.Create(
                    CultureInfo.InvariantCulture,
                    $"--- AggregateException.InnerExceptions[{i}] (depth {depth + 1}) ---"));
                AppendExceptionBlock(sb, aggregate.InnerExceptions[i], depth + 1);
            }

            return;
        }

        if (exception.InnerException is not null)
        {
            sb.AppendLine(string.Create(
                CultureInfo.InvariantCulture,
                $"--- InnerException (depth {depth + 1}) ---"));
            AppendExceptionBlock(sb, exception.InnerException, depth + 1);
        }
    }

    private static bool TryCreateLogFile(
        string directory,
        DateTimeOffset now,
        string startupLine,
        IReadOnlyList<string> extraLines,
        Action<string>? directoryCreatorOverride,
        out string? createdFilePath,
        out string? errorMessage)
    {
        createdFilePath = null;
        errorMessage = null;

        try
        {
            if (directoryCreatorOverride is not null)
            {
                directoryCreatorOverride(directory);
            }
            else
            {
                Directory.CreateDirectory(PathSupport.ToIoPath(directory));
            }

            var sb = new StringBuilder();
            sb.Append(startupLine).Append(TextOutput.LineEnding);
            foreach (var extra in extraLines)
            {
                if (!string.IsNullOrWhiteSpace(extra))
                {
                    sb.Append(extra).Append(TextOutput.LineEnding);
                }
            }

            var content = sb.ToString();

            for (var attempt = 0; attempt < 100; attempt++)
            {
                var fileName = attempt == 0 ? BuildLogFileName(now) : BuildLogFileName(now, attempt + 1);
                var candidatePath = Path.Combine(directory, fileName);
                try
                {
                    TextOutput.WriteNewTextFile(candidatePath, content);
                    createdFilePath = candidatePath;
                    return true;
                }
                catch (IOException) when (File.Exists(PathSupport.ToIoPath(candidatePath)))
                {
                    // Another instance created the same-second filename; try next suffix.
                }
            }

            var uniqueName = $"{LogFilePrefix}{now.ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture)}-{Guid.NewGuid().ToString("N")[..8]}{LogFileExtension}";
            var uniquePath = Path.Combine(directory, uniqueName);
            TextOutput.WriteNewTextFile(uniquePath, content);

            createdFilePath = uniquePath;
            return true;
        }
        catch (Exception exception)
        {
            errorMessage = $"{directory} ({exception.GetType().Name}: {exception.Message})";
            return false;
        }
    }

    private static (string Version, string GitCommit) ReadBuildMetadata(Assembly? entryAssembly)
    {
        try
        {
            var assembly = entryAssembly ?? typeof(GuiStartupLog).Assembly;
            var informational = assembly.GetCustomAttribute<AssemblyInformationalVersionAttribute>()?.InformationalVersion;
            var version = !string.IsNullOrWhiteSpace(informational)
                ? informational.Split('+')[0]
                : assembly.GetName().Version?.ToString(3) ?? "0.1.0";

            var gitCommit = assembly
                .GetCustomAttributes<AssemblyMetadataAttribute>()
                .FirstOrDefault(attr => string.Equals(attr.Key, "GitCommit", StringComparison.Ordinal))
                ?.Value;

            return (version, string.IsNullOrWhiteSpace(gitCommit) ? "unknown" : gitCommit);
        }
        catch
        {
            return ("unknown", "unknown");
        }
    }

    private static string ResolveExecutablePath()
    {
        try
        {
            if (!string.IsNullOrWhiteSpace(Environment.ProcessPath))
            {
                return Environment.ProcessPath;
            }
        }
        catch
        {
            // Fall through to Process / BaseDirectory.
        }

        try
        {
            using var current = Process.GetCurrentProcess();
            if (!string.IsNullOrWhiteSpace(current.MainModule?.FileName))
            {
                return current.MainModule.FileName;
            }
        }
        catch
        {
            // Fall through to BaseDirectory.
        }

        return string.IsNullOrWhiteSpace(AppContext.BaseDirectory)
            ? "<unknown>"
            : AppContext.BaseDirectory;
    }

    private static string? NormalizePathForComparison(string? path)
    {
        if (string.IsNullOrWhiteSpace(path))
        {
            return null;
        }

        try
        {
            var normalized = PathSupport.TrimTrailingSeparators(Path.GetFullPath(path));
            return OperatingSystem.IsWindows() ? normalized.ToUpperInvariant() : normalized;
        }
        catch
        {
            return OperatingSystem.IsWindows() ? path.ToUpperInvariant() : path;
        }
    }
}
