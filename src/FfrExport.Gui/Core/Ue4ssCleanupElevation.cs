using System.ComponentModel;
using System.Diagnostics;
using System.Globalization;

namespace FfrExport.Gui.Core;

internal sealed record ElevatedCleanupRequest(
    string GameDirectory,
    bool IncludeForeign,
    string ActiveAgpRoot,
    string ResultFilePath,
    string? Language = null);

internal sealed record ElevatedCleanupRunResult(
    bool UacCancelled,
    int? ExitCode,
    string ResultFilePath,
    Ue4ssCleanupResultDocument? Result,
    string? ReadError,
    string? LaunchError);

/// <summary>
/// Non-WinForms orchestration helpers for retrying UE4SS cleanup with elevation (UAC "runas").
/// Builds elevated CLI arguments (--agp-home, --result-file, --yes), handles UAC cancellation
/// (Win32 ERROR_CANCELLED = 1223), reads the structured JSON result file written by the elevated CLI,
/// and formats human-readable lines for the GUI log and summary dialogs.
/// </summary>
internal static class Ue4ssCleanupElevation
{
    /// <summary>
    /// Win32 ERROR_CANCELLED: returned in Win32Exception.NativeErrorCode when the user clicks "No" on UAC.
    /// </summary>
    internal const int ErrorCancelled = 1223;

    internal static string BuildElevatedResultFilePath(string activeAgpRoot, DateTimeOffset? nowOverride = null)
    {
        if (string.IsNullOrWhiteSpace(activeAgpRoot))
        {
            throw new ArgumentException(L.Get("GuiCore_PathEmpty"), nameof(activeAgpRoot));
        }

        var logsDir = AgpPaths.GetLogsDirectoryForAgpRoot(activeAgpRoot);
        var stamp = (nowOverride ?? DateTimeOffset.Now).ToString("yyyyMMdd-HHmmss", CultureInfo.InvariantCulture);
        for (var attempt = 0; attempt < 50; attempt++)
        {
            var fileName = attempt == 0
                ? $"uninstall-elevated-{stamp}.json"
                : string.Create(CultureInfo.InvariantCulture, $"uninstall-elevated-{stamp}-{attempt + 1}.json");
            var candidate = Path.Combine(logsDir, fileName);
            if (!File.Exists(PathSupport.ToIoPath(candidate)))
            {
                return candidate;
            }
        }

        return Path.Combine(logsDir, $"uninstall-elevated-{stamp}-{Guid.NewGuid().ToString("N")[..8]}.json");
    }

    internal static IReadOnlyList<string> BuildElevatedUninstallArguments(ElevatedCleanupRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);
        if (string.IsNullOrWhiteSpace(request.GameDirectory))
        {
            throw new ArgumentException(L.Get("UninstallUe4ss_RequiresGame"), nameof(request));
        }

        if (string.IsNullOrWhiteSpace(request.ActiveAgpRoot))
        {
            throw new ArgumentException(L.Format("UninstallUe4ss_EmptyOptionValue", "--agp-home"), nameof(request));
        }

        if (string.IsNullOrWhiteSpace(request.ResultFilePath))
        {
            throw new ArgumentException(L.Format("UninstallUe4ss_EmptyOptionValue", "--result-file"), nameof(request));
        }

        return BuildCliCommand.BuildUninstallUe4ssArguments(
            gameDirectory: request.GameDirectory,
            includeForeign: request.IncludeForeign,
            yes: true,
            language: request.Language,
            agpHome: request.ActiveAgpRoot,
            resultFile: request.ResultFilePath);
    }

    internal static ProcessStartInfo CreateElevatedStartInfo(
        string executablePath,
        IReadOnlyList<string> arguments,
        string? workingDirectory = null)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = executablePath,
            WorkingDirectory = string.IsNullOrWhiteSpace(workingDirectory)
                ? Path.GetDirectoryName(executablePath) ?? Directory.GetCurrentDirectory()
                : workingDirectory,
            UseShellExecute = true,
            Verb = "runas",
            WindowStyle = ProcessWindowStyle.Hidden,
        };

        foreach (var arg in arguments)
        {
            startInfo.ArgumentList.Add(arg);
        }

        return startInfo;
    }

    internal static bool TryReadResultFile(
        string resultFilePath,
        out Ue4ssCleanupResultDocument? result,
        out string? errorMessage)
    {
        result = null;
        errorMessage = null;

        if (string.IsNullOrWhiteSpace(resultFilePath))
        {
            errorMessage = L.Get("GuiCore_PathEmpty");
            return false;
        }

        var ioPath = PathSupport.ToIoPath(resultFilePath);
        if (!File.Exists(ioPath))
        {
            errorMessage = L.Format("Gui_CleanupElevatedResultReadFailed", resultFilePath, L.Get("Common_Missing"));
            return false;
        }

        string rawJson;
        try
        {
            rawJson = File.ReadAllText(ioPath, TextOutput.Utf8WithoutBom);
        }
        catch (Exception exception)
        {
            errorMessage = L.Format("Gui_CleanupElevatedResultReadFailed", resultFilePath, exception.Message);
            return false;
        }

        if (!ProgressProtocol.TryDeserializeUe4ssCleanupResult(rawJson, out var parsed, out var parseError) || parsed is null)
        {
            errorMessage = L.Format(
                "Gui_CleanupElevatedResultReadFailed",
                resultFilePath,
                parseError ?? L.Get("Common_UnknownError"));
            return false;
        }

        result = parsed;
        return true;
    }

    internal static async Task<ElevatedCleanupRunResult> RunElevatedAsync(
        string executablePath,
        ElevatedCleanupRequest request,
        Func<ProcessStartInfo, Task<int>>? processRunnerOverride = null,
        string? workingDirectory = null)
    {
        ArgumentNullException.ThrowIfNull(request);

        var args = BuildElevatedUninstallArguments(request);
        var startInfo = CreateElevatedStartInfo(executablePath, args, workingDirectory);

        try
        {
            var resultParent = Path.GetDirectoryName(request.ResultFilePath);
            if (!string.IsNullOrWhiteSpace(resultParent))
            {
                Directory.CreateDirectory(PathSupport.ToIoPath(resultParent));
            }
        }
        catch
        {
            // The elevated CLI also creates the directory atomically when writing the result file.
        }

        int exitCode;
        try
        {
            exitCode = processRunnerOverride is not null
                ? await processRunnerOverride(startInfo).ConfigureAwait(false)
                : await RunProcessAsync(startInfo).ConfigureAwait(false);
        }
        catch (Win32Exception win32Exception) when (win32Exception.NativeErrorCode == ErrorCancelled)
        {
            return new ElevatedCleanupRunResult(
                UacCancelled: true,
                ExitCode: null,
                ResultFilePath: request.ResultFilePath,
                Result: null,
                ReadError: null,
                LaunchError: null);
        }
        catch (Exception exception)
        {
            return new ElevatedCleanupRunResult(
                UacCancelled: false,
                ExitCode: null,
                ResultFilePath: request.ResultFilePath,
                Result: null,
                ReadError: null,
                LaunchError: exception.Message);
        }

        var readOk = TryReadResultFile(request.ResultFilePath, out var resultDoc, out var readError);
        return new ElevatedCleanupRunResult(
            UacCancelled: false,
            ExitCode: exitCode,
            ResultFilePath: request.ResultFilePath,
            Result: readOk ? resultDoc : null,
            ReadError: readOk ? null : readError,
            LaunchError: null);
    }

    internal static IReadOnlyList<string> FormatResultForLog(ElevatedCleanupRunResult runResult)
    {
        ArgumentNullException.ThrowIfNull(runResult);

        if (runResult.UacCancelled)
        {
            return [L.Get("Gui_CleanupUacCancelledBody")];
        }

        if (!string.IsNullOrWhiteSpace(runResult.LaunchError))
        {
            return [L.Format("Gui_CleanupElevatedLaunchFailed", runResult.LaunchError)];
        }

        var lines = new List<string>
        {
            L.Format("Gui_CleanupElevatedResultHeader", runResult.ExitCode ?? 2),
        };

        if (runResult.Result is not null)
        {
            var doc = runResult.Result;
            lines.Add(L.Format("Gui_CleanupElevatedSummaryCounts", doc.Status, doc.RemovedFiles, doc.RemovedDirs));
            if (!string.IsNullOrWhiteSpace(doc.BackupDirectory))
            {
                lines.Add(L.Format("Gui_CleanupElevatedSummaryBackup", doc.BackupDirectory));
            }

            if (!string.IsNullOrWhiteSpace(doc.BlockedPath))
            {
                lines.Add(L.Format("Gui_CleanupElevatedSummaryBlocked", doc.BlockedPath, doc.Reason ?? L.Get("Common_Unknown")));
            }

            if (doc.Messages is { Length: > 0 })
            {
                foreach (var messageLine in doc.Messages)
                {
                    if (!string.IsNullOrWhiteSpace(messageLine))
                    {
                        lines.Add(messageLine);
                    }
                }
            }
            else if (!string.IsNullOrWhiteSpace(doc.Message))
            {
                lines.Add(doc.Message);
            }
        }
        else if (!string.IsNullOrWhiteSpace(runResult.ReadError))
        {
            lines.Add(runResult.ReadError);
        }
        else
        {
            lines.Add(L.Format("Gui_CleanupElevatedResultMissing", runResult.ExitCode ?? 2, runResult.ResultFilePath));
        }

        return lines;
    }

    internal static string FormatResultSummaryForDialog(ElevatedCleanupRunResult runResult)
    {
        ArgumentNullException.ThrowIfNull(runResult);

        if (runResult.UacCancelled)
        {
            return L.Get("Gui_CleanupUacCancelledBody");
        }

        if (!string.IsNullOrWhiteSpace(runResult.LaunchError))
        {
            return L.Format("Gui_CleanupElevatedLaunchFailed", runResult.LaunchError);
        }

        if (runResult.Result is not null)
        {
            if (runResult.Result.Messages is { Length: > 0 })
            {
                return string.Join(Environment.NewLine, runResult.Result.Messages);
            }

            if (!string.IsNullOrWhiteSpace(runResult.Result.Message))
            {
                return runResult.Result.Message;
            }
        }

        if (!string.IsNullOrWhiteSpace(runResult.ReadError))
        {
            return runResult.ReadError;
        }

        return runResult.ExitCode == 0
            ? L.Get("Gui_CleanupCompleteDefaultBody")
            : L.Format("Gui_CleanupFailedExitBody", runResult.ExitCode ?? 2);
    }

    private static async Task<int> RunProcessAsync(ProcessStartInfo startInfo)
    {
        using var process = Process.Start(startInfo)
            ?? throw new InvalidOperationException(L.Format("GuiCore_LaunchCliFailed", startInfo.FileName));
        await process.WaitForExitAsync().ConfigureAwait(false);
        return process.ExitCode;
    }
}
