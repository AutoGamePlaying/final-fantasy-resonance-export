using System.Diagnostics;
using System.Runtime.CompilerServices;
using System.Text;

namespace FfrExport.Tests;

internal sealed record CliResult(int ExitCode, string Stdout, string Stderr);

internal sealed class TempWorkspace : IDisposable
{
    public string Root { get; }
    public string DepsDir { get; }
    public string AgpHomeDir { get; }

    public TempWorkspace(string prefix)
    {
        Root = Path.Combine(Path.GetTempPath(), $"{prefix}-{Environment.ProcessId}-{Guid.NewGuid():N}");
        DepsDir = Path.Combine(Root, ".deps");
        AgpHomeDir = Path.Combine(Root, "agp-home");
        Directory.CreateDirectory(DepsDir);
        Directory.CreateDirectory(AgpHomeDir);
    }

    public void Dispose()
    {
        try
        {
            if (Directory.Exists(Root))
            {
                Directory.Delete(Root, recursive: true);
            }
        }
        catch
        {
            // Ignore transient cleanup failures in temporary directories.
        }
    }
}

internal static class TestHarness
{
    private static readonly string ProcessTempAgpHome = Path.Combine(
        Path.GetTempPath(),
        $"ffr-export-test-agp-{Environment.ProcessId}-{Guid.NewGuid():N}");

    /// <summary>
    /// Guarantees that tests never read from or write to the host machine's real %LOCALAPPDATA% / ~/.local/share.
    /// </summary>
    [ModuleInitializer]
    internal static void InitializeIsolatedAgpHome()
    {
        if (string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable(AgpPaths.AgpHomeEnvVar)))
        {
            Directory.CreateDirectory(ProcessTempAgpHome);
            Environment.SetEnvironmentVariable(AgpPaths.AgpHomeEnvVar, ProcessTempAgpHome);
        }
    }

    public static string FindRepoRoot()
    {
        var dir = new DirectoryInfo(AppContext.BaseDirectory);
        while (dir is not null)
        {
            if (File.Exists(Path.Combine(dir.FullName, "FfrExport.sln")))
            {
                return dir.FullName;
            }

            dir = dir.Parent;
        }

        throw new InvalidOperationException("找不到仓库根目录（未找到 FfrExport.sln）");
    }

    private static string ResolveDotnetExecutable()
    {
        var dotnetRoot = Environment.GetEnvironmentVariable("DOTNET_ROOT");
        if (!string.IsNullOrWhiteSpace(dotnetRoot))
        {
            var candidate = Path.Combine(dotnetRoot, OperatingSystem.IsWindows() ? "dotnet.exe" : "dotnet");
            if (File.Exists(candidate))
            {
                return candidate;
            }
        }

        var repoDotnet = Path.Combine(FindRepoRoot(), ".deps", "dotnet", OperatingSystem.IsWindows() ? "dotnet.exe" : "dotnet");
        if (File.Exists(repoDotnet))
        {
            return repoDotnet;
        }

        if (!string.IsNullOrWhiteSpace(Environment.ProcessPath)
            && Path.GetFileNameWithoutExtension(Environment.ProcessPath).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
        {
            return Environment.ProcessPath;
        }

        return "dotnet";
    }

    public static async Task<CliResult> RunCliAsync(
        IReadOnlyList<string> args,
        string? ffrDeps = null,
        string? workingDirectory = null,
        string? pathEnvOverride = null,
        string? stdinInput = null,
        string? agpHome = null,
        IReadOnlyDictionary<string, string?>? extraEnv = null)
    {
        var cliHostDll = Path.Combine(AppContext.BaseDirectory, "cli-host", "FfrExport.TestCliHost.dll");
        if (!File.Exists(cliHostDll))
        {
            throw new FileNotFoundException($"测试 CLI 宿主产物不存在：{cliHostDll}");
        }

        var dotnetExe = ResolveDotnetExecutable();
        var startInfo = new ProcessStartInfo
        {
            FileName = dotnetExe,
            WorkingDirectory = workingDirectory ?? FindRepoRoot(),
            RedirectStandardInput = stdinInput is not null,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        if (stdinInput is not null)
        {
            startInfo.StandardInputEncoding = Encoding.UTF8;
        }

        startInfo.ArgumentList.Add("exec");
        startInfo.ArgumentList.Add(cliHostDll);
        foreach (var arg in args)
        {
            startInfo.ArgumentList.Add(arg);
        }

        // Ensure child .NET apphost processes (such as FfrExport.FakeFfrDb) locate the active runtime.
        if (string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("DOTNET_ROOT"))
            && Path.IsPathRooted(dotnetExe))
        {
            var dotnetDir = Path.GetDirectoryName(dotnetExe);
            if (!string.IsNullOrEmpty(dotnetDir))
            {
                startInfo.Environment["DOTNET_ROOT"] = dotnetDir;
            }
        }

        // Always isolate AGP_HOME to a temporary directory for child CLI processes.
        var effectiveAgpHome = !string.IsNullOrWhiteSpace(agpHome)
            ? agpHome
            : Environment.GetEnvironmentVariable(AgpPaths.AgpHomeEnvVar) ?? ProcessTempAgpHome;
        Directory.CreateDirectory(effectiveAgpHome);
        startInfo.Environment[AgpPaths.AgpHomeEnvVar] = effectiveAgpHome;

        if (ffrDeps is null)
        {
            startInfo.Environment.Remove(Program.DepsEnvVar);
        }
        else
        {
            startInfo.Environment[Program.DepsEnvVar] = ffrDeps;
        }

        if (pathEnvOverride is not null)
        {
            startInfo.Environment["PATH"] = pathEnvOverride;
        }

        // Isolate AGP_LANG so host environment never leaks into child CLI tests.
        startInfo.Environment.Remove(Localization.LangEnvVar);

        if (extraEnv is not null)
        {
            foreach (var (key, value) in extraEnv)
            {
                if (value is null)
                {
                    startInfo.Environment.Remove(key);
                }
                else
                {
                    startInfo.Environment[key] = value;
                }
            }
        }

        using var process = Process.Start(startInfo)
            ?? throw new InvalidOperationException("无法启动 FfrExport 子进程");

        if (stdinInput is not null)
        {
            await process.StandardInput.WriteAsync(stdinInput);
            process.StandardInput.Close();
        }

        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();

        return new CliResult(process.ExitCode, await stdoutTask, await stderrTask);
    }
}
