using System.Text.RegularExpressions;
using Xunit;

namespace FfrExport.Tests;

public sealed class SourceSafetyTests
{
    private static readonly string[] ForbiddenTokens =
    [
        "OodleHelper",
        "DownloadOodle",
        "HttpClient",
        "WebClient",
    ];

    private static IReadOnlyList<(string RelativePath, string Content)> ReadSourceAndScriptFiles()
    {
        var root = TestHarness.FindRepoRoot();
        var targets = new List<string>();
        var srcDir = Path.Combine(root, "src", "FfrExport");
        var scriptsDir = Path.Combine(root, "scripts");

        foreach (var path in Directory.EnumerateFiles(srcDir, "*", SearchOption.TopDirectoryOnly))
        {
            var ext = Path.GetExtension(path);
            if (ext is ".cs" or ".csproj")
            {
                targets.Add(path);
            }
        }

        foreach (var path in Directory.EnumerateFiles(scriptsDir, "*.sh", SearchOption.TopDirectoryOnly))
        {
            targets.Add(path);
        }

        Assert.True(targets.Count >= 8, "应至少扫描到 src/FfrExport 与 scripts 下的全部源码和脚本");

        return targets
            .OrderBy(path => path, StringComparer.Ordinal)
            .Select(path => (Path.GetRelativePath(root, path).Replace('\\', '/'), File.ReadAllText(path)))
            .ToArray();
    }

    [Fact]
    public void SourcesAndScripts_NeverInitializeProprietaryOodleOrUseWebClients()
    {
        foreach (var (relativePath, content) in ReadSourceAndScriptFiles())
        {
            foreach (var forbidden in ForbiddenTokens)
            {
                Assert.False(
                    content.Contains(forbidden, StringComparison.Ordinal),
                    $"{relativePath} 不得包含 {forbidden}");
            }
        }
    }

    [Fact]
    public void SourcesAndScripts_ContainNoMachineSpecificAbsolutePaths()
    {
        var linuxRootRegex = new Regex(@"/root/", RegexOptions.CultureInvariant);
        var windowsDriveRegex = new Regex(@"[A-Za-z]:[\\/]", RegexOptions.CultureInvariant);

        foreach (var (relativePath, content) in ReadSourceAndScriptFiles())
        {
            Assert.DoesNotMatch(linuxRootRegex, content);
            Assert.DoesNotMatch(windowsDriveRegex, content);
        }
    }

    [Fact]
    public void ProjectFile_PinsRequiredCue4ParseVersionAndStrictSettings()
    {
        var root = TestHarness.FindRepoRoot();
        var csproj = File.ReadAllText(Path.Combine(root, "src", "FfrExport", "FfrExport.csproj"));

        Assert.Matches(@"<PackageReference Include=""CUE4Parse"" Version=""1\.2\.2\.202611"" />", csproj);
        Assert.Matches(@"<PackageReference Include=""Microsoft\.Bcl\.Memory"" Version=""9\.0\.14"" />", csproj);
        Assert.Contains("<Nullable>enable</Nullable>", csproj, StringComparison.Ordinal);
        Assert.Contains("<TreatWarningsAsErrors>true</TreatWarningsAsErrors>", csproj, StringComparison.Ordinal);
    }

    [Fact]
    public void Subcommands_EnforceManifestAndOutputDirectoryGuards()
    {
        var root = TestHarness.FindRepoRoot();
        var srcDir = Path.Combine(root, "src", "FfrExport");

        var program = File.ReadAllText(Path.Combine(srcDir, "Program.cs"));
        Assert.Contains("RequireExportManifest(outputPath, outputArgument)", program, StringComparison.Ordinal);
        Assert.Contains("Path.Combine(outputPath, \"manifest.json\")", program, StringComparison.Ordinal);
        Assert.Contains("manifest.Property(\"cue4ParseVersion\") is null", program, StringComparison.Ordinal);
        Assert.Contains("case \"locres\":", program, StringComparison.Ordinal);
        Assert.Contains("new FTextLocalizationResource(", program, StringComparison.Ordinal);
        Assert.Contains("DefaultLanguages = [\"zh-Hans\", \"en\", \"ja\"]", program, StringComparison.Ordinal);
        Assert.Contains("PrepareOutputDirectory(outputPath, outputArgument, force, \"locres\")", program, StringComparison.Ordinal);
        Assert.Contains("EnsureManifestMatchesCommand(", program, StringComparison.Ordinal);
        Assert.Contains("DepsEnvVar = \"FFR_DEPS\"", program, StringComparison.Ordinal);

        var inventory = File.ReadAllText(Path.Combine(srcDir, "Inventory.cs"));
        Assert.Contains("new FAssetRegistryState(", inventory, StringComparison.Ordinal);
        Assert.Contains("PrepareOutputDirectory(outputPath, outputArgument, force, \"inventory\")", inventory, StringComparison.Ordinal);

        var assetExport = File.ReadAllText(Path.Combine(srcDir, "AssetExport.cs"));
        Assert.Contains("PrepareResumableOutputDirectory(outputPath, outputArgument, force, \"export-assets\")", assetExport, StringComparison.Ordinal);
        Assert.Contains("EnsureManifestMatchesCommand(", assetExport, StringComparison.Ordinal);
        Assert.Contains("IsOtherCommandSubdirectory(", assetExport, StringComparison.Ordinal);

        var mapExport = File.ReadAllText(Path.Combine(srcDir, "MapExport.cs"));
        Assert.Contains("RequireDepsPath(options.OutputArgument, \"--out\")", mapExport, StringComparison.Ordinal);
        Assert.Contains("EnsureManifestMatchesCommand(manifest, outputArgument, \"export-maps\")", mapExport, StringComparison.Ordinal);
        Assert.Contains("command = \"export-maps\"", mapExport, StringComparison.Ordinal);
    }

    [Fact]
    public void Scripts_UseLocalDepsDotnetAndNugetAndAllowDotnetRootOverride()
    {
        var root = TestHarness.FindRepoRoot();
        var runScript = File.ReadAllText(Path.Combine(root, "scripts", "run.sh"));
        var checkScript = File.ReadAllText(Path.Combine(root, "scripts", "check.sh"));

        foreach (var script in new[] { runScript, checkScript })
        {
            Assert.Contains("DOTNET_ROOT", script, StringComparison.Ordinal);
            Assert.Contains(".deps/dotnet", script, StringComparison.Ordinal);
            Assert.Contains(".deps/nuget", script, StringComparison.Ordinal);
            Assert.DoesNotContain("~/.dotnet", script, StringComparison.Ordinal);
        }

        Assert.Contains("format FfrExport.sln --verify-no-changes", checkScript, StringComparison.Ordinal);
        Assert.Contains("build FfrExport.sln -c Release", checkScript, StringComparison.Ordinal);
        Assert.Contains("test FfrExport.sln", checkScript, StringComparison.Ordinal);
    }
}
