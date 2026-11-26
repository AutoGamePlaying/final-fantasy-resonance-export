using System.Text.RegularExpressions;
using Xunit;

namespace FfrExport.Tests;

public sealed class SourceSafetyTests
{
    private static readonly string[] AlwaysForbiddenTokens =
    [
        "OodleHelper",
        "DownloadOodle",
        "WebClient",
    ];

    private const string AllowedHttpClientFile = "src/FfrExport/Ue4ssDownloader.cs";

    private static readonly HashSet<string> AllowedDirectTextOutputFiles = new(StringComparer.Ordinal)
    {
        "src/FfrExport/TextOutput.cs",
        "src/FfrExport/TextOutput.Newtonsoft.cs",
    };

    private static readonly (string Name, Regex Pattern)[] ForbiddenDirectTextWritePatterns =
    [
        ("File.WriteAllText/Async", new Regex(@"\bFile\s*\.\s*WriteAllText(?:Async)?\b", RegexOptions.CultureInvariant)),
        ("File.WriteAllLines/Async", new Regex(@"\bFile\s*\.\s*WriteAllLines(?:Async)?\b", RegexOptions.CultureInvariant)),
        ("File.AppendAllText/Async", new Regex(@"\bFile\s*\.\s*AppendAllText(?:Async)?\b", RegexOptions.CultureInvariant)),
        ("File.AppendAllLines/Async", new Regex(@"\bFile\s*\.\s*AppendAllLines(?:Async)?\b", RegexOptions.CultureInvariant)),
        ("File.CreateText", new Regex(@"\bFile\s*\.\s*CreateText\b", RegexOptions.CultureInvariant)),
        ("File.AppendText", new Regex(@"\bFile\s*\.\s*AppendText\b", RegexOptions.CultureInvariant)),
        ("new StreamWriter", new Regex(@"\bnew\s+StreamWriter\b", RegexOptions.CultureInvariant)),
        ("new JsonTextWriter", new Regex(@"\bnew\s+JsonTextWriter\b", RegexOptions.CultureInvariant)),
        ("new Utf8JsonWriter", new Regex(@"\bnew\s+Utf8JsonWriter\b", RegexOptions.CultureInvariant)),
        ("JsonConvert.SerializeObject", new Regex(@"\bJsonConvert\s*\.\s*SerializeObject\b", RegexOptions.CultureInvariant)),
        ("JsonSerializer.Serialize", new Regex(@"\bJsonSerializer\s*\.\s*Serialize(?:Async|ToUtf8Bytes|ToDocument|ToElement|ToNode)?\b", RegexOptions.CultureInvariant)),
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
    public void SourcesAndScripts_NeverInitializeProprietaryOodleOrUseUnauthorizedNetworkClients()
    {
        var sawDownloader = false;
        foreach (var (relativePath, content) in ReadSourceAndScriptFiles())
        {
            foreach (var forbidden in AlwaysForbiddenTokens)
            {
                Assert.False(
                    content.Contains(forbidden, StringComparison.Ordinal),
                    $"{relativePath} 不得包含 {forbidden}");
            }

            if (string.Equals(relativePath, AllowedHttpClientFile, StringComparison.Ordinal))
            {
                sawDownloader = true;
                Assert.Contains(
                    "https://github.com/UE4SS-RE/RE-UE4SS/releases/download/v4.0.0/UE4SS_v4.0.0.zip",
                    content,
                    StringComparison.Ordinal);
                Assert.Contains(
                    "251ad0d1381a241750e036f5918cbf2b5d7ec58ebc608cb2dd09d459d0c81e45",
                    content,
                    StringComparison.Ordinal);
            }
            else
            {
                Assert.False(
                    content.Contains("HttpClient", StringComparison.Ordinal),
                    $"{relativePath} 不得包含 HttpClient（仅允许 {AllowedHttpClientFile} 联网下载官方 UE4SS）");
            }
        }

        Assert.True(sawDownloader, $"应扫描到 {AllowedHttpClientFile}");
    }

    [Fact]
    public void SourcesAndScripts_ContainNoMachineSpecificAbsolutePaths()
    {
        var linuxRootRegex = new Regex(@"/root/", RegexOptions.CultureInvariant);
        // Match Windows drive paths (e.g. C:\ or D:/) without false-matching URI schemes like https:// or steam://.
        var windowsDriveRegex = new Regex(@"(?<![A-Za-z0-9+.-])[A-Za-z]:[\\/]", RegexOptions.CultureInvariant);

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

    [Fact]
    public void AllSources_RouteTextWritesAndJsonSerializationThroughTextOutputHelper()
    {
        var root = TestHarness.FindRepoRoot();
        var srcRoot = Path.Combine(root, "src");
        var csFiles = Directory.EnumerateFiles(srcRoot, "*.cs", SearchOption.AllDirectories)
            .Where(path =>
                !path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                && !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();

        Assert.True(csFiles.Length >= 15, "应扫描到 src/FfrExport 与 src/FfrExport.Gui 下的全部 C# 源码");

        var seenAllowed = new HashSet<string>(StringComparer.Ordinal);
        foreach (var path in csFiles)
        {
            var relativePath = Path.GetRelativePath(root, path).Replace('\\', '/');
            var content = File.ReadAllText(path);

            if (AllowedDirectTextOutputFiles.Contains(relativePath))
            {
                seenAllowed.Add(relativePath);
                continue;
            }

            foreach (var (name, pattern) in ForbiddenDirectTextWritePatterns)
            {
                Assert.False(
                    pattern.IsMatch(content),
                    $"{relativePath} 不得直接调用 {name}；请改用公共 TextOutput 辅助函数以统一 UTF-8 无 BOM 与 LF (\\n) 换行");
            }
        }

        Assert.Equal(AllowedDirectTextOutputFiles.Count, seenAllowed.Count);

        var textOutputCs = File.ReadAllText(Path.Combine(root, "src", "FfrExport", "TextOutput.cs"));
        Assert.Contains("LineEnding = \"\\n\"", textOutputCs, StringComparison.Ordinal);
        Assert.Contains("NewLine = LineEnding", textOutputCs, StringComparison.Ordinal);
        Assert.Contains("encoderShouldEmitUTF8Identifier: false", textOutputCs, StringComparison.Ordinal);

        var guiCsproj = File.ReadAllText(Path.Combine(root, "src", "FfrExport.Gui", "FfrExport.Gui.csproj"));
        Assert.Contains(@"..\FfrExport\TextOutput.cs", guiCsproj, StringComparison.Ordinal);
    }
}
