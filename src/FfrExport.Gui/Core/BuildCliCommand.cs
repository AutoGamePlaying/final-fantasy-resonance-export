using System.Diagnostics;
using System.Globalization;
using System.Text;

namespace FfrExport.Gui.Core;

internal sealed record BuildCommandRequest(
    string GameDirectory,
    string UsmapPath,
    IReadOnlyList<string> JmapPaths,
    string OutputDirectory,
    string? SteamBuildId,
    bool Force,
    string? Language = null,
    bool ProgressJson = true);

internal sealed record StepProgress(
    int CurrentStep,
    int TotalSteps,
    string Title,
    int Percent);

/// <summary>
/// Assembles ffr-export CLI arguments, parses language-independent machine-readable progress lines from stdout,
/// and executes the child CLI process with UTF-8 pipes and whole-process-tree cancellation.
/// </summary>
internal static class BuildCliCommand
{
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

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

        if (request.ProgressJson)
        {
            args.Add("--progress-json");
        }

        if (!string.IsNullOrWhiteSpace(request.Language))
        {
            args.Add("--lang");
            args.Add(request.Language.Trim());
        }

        return args;
    }

    internal static IReadOnlyList<string> BuildFetchUsmapArguments(
        string gameDirectory,
        int? timeoutSeconds = null,
        bool yes = true,
        string? language = null,
        bool progressJson = true)
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

        if (progressJson)
        {
            args.Add("--progress-json");
        }

        if (!string.IsNullOrWhiteSpace(language))
        {
            args.Add("--lang");
            args.Add(language.Trim());
        }

        return args;
    }

    internal static IReadOnlyList<string> BuildUninstallUe4ssArguments(
        string gameDirectory,
        bool includeForeign = false,
        bool yes = false,
        string? language = null)
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

        if (!string.IsNullOrWhiteSpace(language))
        {
            args.Add("--lang");
            args.Add(language.Trim());
        }

        return args;
    }

    internal static bool TryParseFetchedUsmapPath(string? line, out string usmapPath)
    {
        return ProgressProtocol.TryParseUsmapLine(line, out usmapPath);
    }

    internal static bool TryParseStepProgress(string? line, out StepProgress progress)
    {
        progress = default!;
        if (!ProgressProtocol.TryParseProgressLine(line, out var current, out var total, out var title))
        {
            return false;
        }

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
            ?? throw new InvalidOperationException(L.Format("GuiCore_LaunchCliFailed", executablePath));

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
