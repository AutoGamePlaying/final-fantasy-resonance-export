using System.Diagnostics;
using System.Globalization;
using System.Text;
using System.Text.RegularExpressions;

namespace FfrExport.Gui.Core;

internal sealed record BuildCommandRequest(
    string GameDirectory,
    string UsmapPath,
    IReadOnlyList<string> JmapPaths,
    string OutputDirectory,
    string? SteamBuildId,
    bool Force);

internal sealed record StepProgress(
    int CurrentStep,
    int TotalSteps,
    string Title,
    int Percent);

/// <summary>
/// Assembles ffr-export build-db CLI arguments, parses step progress markers from stdout,
/// and executes the child CLI process with UTF-8 pipes and whole-process-tree cancellation.
/// </summary>
internal static class BuildCliCommand
{
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    private static readonly Regex StepLineRegex = new(
        @"^\s*==>\s*步骤\s*(\d+)\s*/\s*(\d+)(?:\s*[：:]\s*(.*))?$",
        RegexOptions.Compiled | RegexOptions.CultureInvariant);

    internal static IReadOnlyList<string> BuildArguments(BuildCommandRequest request)
    {
        ArgumentNullException.ThrowIfNull(request);

        var args = new List<string>
        {
            "build-db",
            "--game",
            request.GameDirectory,
            "--usmap",
            request.UsmapPath,
        };

        foreach (var jmap in request.JmapPaths)
        {
            if (!string.IsNullOrWhiteSpace(jmap))
            {
                args.Add("--jmap");
                args.Add(jmap);
            }
        }

        args.Add("--out");
        args.Add(request.OutputDirectory);

        if (!string.IsNullOrWhiteSpace(request.SteamBuildId))
        {
            args.Add("--steam-buildid");
            args.Add(request.SteamBuildId.Trim());
        }

        if (request.Force)
        {
            args.Add("--force");
        }

        return args;
    }

    internal static IReadOnlyList<string> BuildFetchUsmapArguments(
        string gameDirectory,
        int? timeoutSeconds = null,
        bool yes = true)
    {
        var args = new List<string>
        {
            "fetch-usmap",
            "--game",
            gameDirectory,
        };

        if (timeoutSeconds.HasValue && timeoutSeconds.Value > 0)
        {
            args.Add("--timeout");
            args.Add(timeoutSeconds.Value.ToString(CultureInfo.InvariantCulture));
        }

        if (yes)
        {
            args.Add("--yes");
        }

        return args;
    }

    internal static IReadOnlyList<string> BuildUninstallUe4ssArguments(
        string gameDirectory,
        bool includeForeign = false,
        bool yes = false)
    {
        var args = new List<string>
        {
            "uninstall-ue4ss",
            "--game",
            gameDirectory,
        };

        if (includeForeign)
        {
            args.Add("--include-foreign");
        }

        if (yes)
        {
            args.Add("--yes");
        }

        return args;
    }

    internal static bool TryParseFetchedUsmapPath(string? line, out string usmapPath)
    {
        usmapPath = string.Empty;
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        const string prefix = "已保存 .usmap：";
        var trimmed = line.Trim();
        if (!trimmed.StartsWith(prefix, StringComparison.Ordinal))
        {
            return false;
        }

        var path = trimmed[prefix.Length..].Trim();
        if (string.IsNullOrWhiteSpace(path))
        {
            return false;
        }

        usmapPath = path;
        return true;
    }

    internal static bool TryParseStepProgress(string? line, out StepProgress progress)
    {
        progress = default!;
        if (string.IsNullOrWhiteSpace(line))
        {
            return false;
        }

        var match = StepLineRegex.Match(line);
        if (!match.Success)
        {
            return false;
        }

        if (!int.TryParse(match.Groups[1].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var current)
            || !int.TryParse(match.Groups[2].Value, NumberStyles.Integer, CultureInfo.InvariantCulture, out var total)
            || current <= 0
            || total <= 0
            || current > total)
        {
            return false;
        }

        var title = match.Groups[3].Success ? match.Groups[3].Value.Trim() : string.Empty;
        // Reserve 90%..100% for final completion after the last step exits.
        var percent = Math.Clamp((int)Math.Round(current * 90.0 / total, MidpointRounding.AwayFromZero), 1, 95);
        progress = new StepProgress(current, total, title, percent);
        return true;
    }

    internal static ProcessStartInfo CreateStartInfo(
        string executablePath,
        IReadOnlyList<string> arguments,
        string? workingDirectory = null,
        string? agpHomeOverride = null)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = executablePath,
            WorkingDirectory = string.IsNullOrWhiteSpace(workingDirectory)
                ? Path.GetDirectoryName(executablePath) ?? Directory.GetCurrentDirectory()
                : workingDirectory,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Utf8WithoutBom,
            StandardErrorEncoding = Utf8WithoutBom,
            UseShellExecute = false,
            CreateNoWindow = true,
        };

        foreach (var arg in arguments)
        {
            startInfo.ArgumentList.Add(arg);
        }

        if (!string.IsNullOrWhiteSpace(agpHomeOverride))
        {
            startInfo.Environment[AgpPaths.AgpHomeEnvVar] = agpHomeOverride;
        }

        return startInfo;
    }

    internal static async Task<int> RunAsync(
        string executablePath,
        IReadOnlyList<string> arguments,
        Action<string> onStdoutLine,
        Action<string> onStderrLine,
        CancellationToken cancellationToken,
        string? workingDirectory = null,
        string? agpHomeOverride = null)
    {
        cancellationToken.ThrowIfCancellationRequested();

        var startInfo = CreateStartInfo(executablePath, arguments, workingDirectory, agpHomeOverride);
        using var process = Process.Start(startInfo)
            ?? throw new InvalidOperationException($"无法启动命令行程序：{executablePath}");

        var gate = new object();
        var disposed = false;

        void KillTreeSafely()
        {
            lock (gate)
            {
                if (disposed)
                {
                    return;
                }

                try
                {
                    if (!process.HasExited)
                    {
                        process.Kill(entireProcessTree: true);
                    }
                }
                catch
                {
                    // Ignore races where the child process exited between HasExited and Kill.
                }
            }
        }

        using var cancellationRegistration = cancellationToken.Register(KillTreeSafely);

        try
        {
            var stdoutTask = PumpLinesAsync(process.StandardOutput, onStdoutLine);
            var stderrTask = PumpLinesAsync(process.StandardError, onStderrLine);

            if (cancellationToken.IsCancellationRequested)
            {
                KillTreeSafely();
            }

            try
            {
                await process.WaitForExitAsync(cancellationToken).ConfigureAwait(false);
            }
            catch (OperationCanceledException)
            {
                KillTreeSafely();
                using var waitAfterKill = new CancellationTokenSource(TimeSpan.FromSeconds(5));
                try
                {
                    await process.WaitForExitAsync(waitAfterKill.Token).ConfigureAwait(false);
                }
                catch
                {
                    // Proceed even if a stubborn grandchild takes longer to tear down.
                }

                throw;
            }

            // Ensure cancellation is still reported if the cancellation callback killed the process
            // before WaitForExitAsync observed the token.
            cancellationToken.ThrowIfCancellationRequested();

            await Task.WhenAll(stdoutTask, stderrTask).ConfigureAwait(false);
            return process.ExitCode;
        }
        finally
        {
            lock (gate)
            {
                disposed = true;
            }
        }
    }

    private static async Task PumpLinesAsync(StreamReader reader, Action<string> onLine)
    {
        try
        {
            while (true)
            {
                var line = await reader.ReadLineAsync().ConfigureAwait(false);
                if (line is null)
                {
                    break;
                }

                onLine(line);
            }
        }
        catch (ObjectDisposedException)
        {
            // Stream closed during cancellation shutdown.
        }
        catch (IOException)
        {
            // Pipe closed during cancellation shutdown.
        }
    }
}
