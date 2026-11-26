using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace FfrExport.FakeFfrDb;

/// <summary>
/// Cross-platform synthetic ffr-db test helper executable used by FfrExport.Tests.
/// Implements --version, build, and hash --json without external script interpreters.
/// </summary>
internal static class Program
{
    private const int ExtendedPathThreshold = 240;

    public static int Main(string[] args)
    {
        var utf8NoBom = new UTF8Encoding(false);
        Console.SetOut(new StreamWriter(Console.OpenStandardOutput(), utf8NoBom) { AutoFlush = true });
        Console.SetError(new StreamWriter(Console.OpenStandardError(), utf8NoBom) { AutoFlush = true });

        if (args.Length == 0)
        {
            Console.Error.WriteLine("missing subcommand");
            return 2;
        }

        return args[0] switch
        {
            "--version" => RunVersion(),
            "build" => RunBuild(args[1..]),
            "hash" => RunHash(args[1..]),
            _ => Fail($"unknown command: {args[0]}"),
        };
    }

    private static int RunVersion()
    {
        Console.WriteLine("ffr-db 0.1.0-fake");
        return 0;
    }

    private static int RunBuild(string[] args)
    {
        var required = new Dictionary<string, string>(StringComparer.Ordinal);
        var i = 0;
        while (i < args.Length)
        {
            var flag = args[i];
            if (flag is "--export" or "--full" or "--inventory" or "--maps" or "--out")
            {
                if (i + 1 >= args.Length)
                {
                    return Fail($"missing value for {flag}");
                }

                required[flag] = args[i + 1];
                i += 2;
            }
            else
            {
                return Fail($"unexpected flag: {flag}");
            }
        }

        foreach (var key in new[] { "--export", "--full", "--inventory", "--maps" })
        {
            if (!required.TryGetValue(key, out var dirPath) || !Directory.Exists(ToIoPath(dirPath)))
            {
                return Fail($"missing dir for {key}");
            }
        }

        if (!required.TryGetValue("--out", out var outPath) || string.IsNullOrWhiteSpace(outPath))
        {
            return Fail("missing --out");
        }

        var fullOutPath = Path.GetFullPath(outPath);
        var parentDir = Path.GetDirectoryName(fullOutPath);
        if (!string.IsNullOrEmpty(parentDir))
        {
            Directory.CreateDirectory(ToIoPath(parentDir));
        }

        var payload = Encoding.ASCII.GetBytes("SQLite format 3\0synthetic-test-db");
        File.WriteAllBytes(ToIoPath(fullOutPath), payload);
        Console.WriteLine($"构建完成：{outPath}");
        return 0;
    }

    private static int RunHash(string[] args)
    {
        if (args.Length < 2 || !args.Contains("--json", StringComparer.Ordinal))
        {
            return Fail("expected --json");
        }

        var dbPath = args[0];
        var bytes = File.ReadAllBytes(ToIoPath(dbPath));
        var fileSha = Convert.ToHexStringLower(SHA256.HashData(bytes));

        var report = new
        {
            format = 1,
            file_sha256 = fileSha,
            content_sha256 = "deadbeef00000000000000000000000000000000000000000000000000000001",
            schema_version = 1,
            sqlite_version = "3.49.1",
            tables = new[]
            {
                new
                {
                    name = "unit",
                    rows = 643,
                    sha256 = "1111111111111111111111111111111111111111111111111111111111111111",
                },
                new
                {
                    name = "text",
                    rows = 3,
                    sha256 = "2222222222222222222222222222222222222222222222222222222222222222",
                },
            },
        };

        Console.WriteLine(JsonSerializer.Serialize(report));
        return 0;
    }

    private static int Fail(string message)
    {
        Console.Error.WriteLine(message);
        return 2;
    }

    private static string ToIoPath(string path)
    {
        if (!OperatingSystem.IsWindows())
        {
            return path;
        }

        var fullPath = Path.GetFullPath(path);
        if (fullPath.Length < ExtendedPathThreshold
            || fullPath.StartsWith(@"\\?\", StringComparison.Ordinal)
            || fullPath.StartsWith(@"\\.\", StringComparison.Ordinal))
        {
            return fullPath;
        }

        if (fullPath.StartsWith(@"\\", StringComparison.Ordinal))
        {
            return @"\\?\UNC\" + fullPath[2..];
        }

        return @"\\?\" + fullPath;
    }
}
