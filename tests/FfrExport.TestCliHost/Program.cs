using System.Reflection;
using System.Runtime.ExceptionServices;
using System.Text;

namespace FfrExport.TestCliHost;

/// <summary>
/// Test CLI host that forces UTF-8 standard output and error streams before invoking FfrExport,
/// preventing Windows redirected-pipe code page fallback (CP437/CP1252) from corrupting Chinese text.
/// </summary>
internal static class Program
{
    public static async Task<int> Main(string[] args)
    {
        var utf8NoBom = new UTF8Encoding(false);
        Console.SetIn(new StreamReader(Console.OpenStandardInput(), utf8NoBom));
        Console.SetOut(new StreamWriter(Console.OpenStandardOutput(), utf8NoBom) { AutoFlush = true });
        Console.SetError(new StreamWriter(Console.OpenStandardError(), utf8NoBom) { AutoFlush = true });

        if (args.Length > 0 && string.Equals(args[0], "--fixture-export", StringComparison.Ordinal))
        {
            return await FixtureExportRunner.RunAsync(args[1..]);
        }

        var assembly = Assembly.Load("FfrExport");
        var entryPoint = assembly.EntryPoint
            ?? throw new InvalidOperationException("Could not find entry point for FfrExport.");

        object? result;
        try
        {
            result = entryPoint.Invoke(null, [args]);
        }
        catch (TargetInvocationException exception) when (exception.InnerException is not null)
        {
            ExceptionDispatchInfo.Capture(exception.InnerException).Throw();
            throw;
        }

        return result switch
        {
            int code => code,
            Task<int> task => await task,
            _ => 0,
        };
    }
}
