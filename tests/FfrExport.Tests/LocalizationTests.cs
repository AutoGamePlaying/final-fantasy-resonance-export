using System.Globalization;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using System.Xml.Linq;
using FfrExport.Gui.Core;
using Xunit;

namespace FfrExport.Tests;

public sealed class LocalizationTests
{
    private static readonly Regex PlaceholderRegex = new(
        @"(?<!\{)\{(\d+)(?:[^}]*)\}(?!\})",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private static readonly Regex ResourceCallRegex = new(
        @"\b(?:L|Localization)\s*\.\s*(?:Get|Format|GetForLanguage)\s*\(\s*""([A-Za-z0-9_]+)""",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    [Fact]
    public void ResxFiles_HaveIdenticalKeySetsMatchingPlaceholdersAndCoverEverySourceReference()
    {
        var repoRoot = TestHarness.FindRepoRoot();
        var resDir = Path.Combine(repoRoot, "src", "FfrExport", "Resources");

        var enEntries = LoadResxDictionary(Path.Combine(resDir, "Strings.resx"));
        var zhEntries = LoadResxDictionary(Path.Combine(resDir, "Strings.zh-Hans.resx"));
        var jaEntries = LoadResxDictionary(Path.Combine(resDir, "Strings.ja.resx"));

        Assert.True(enEntries.Count >= 500, $"Expected rich resource catalog, found {enEntries.Count} entries.");

        var enKeys = enEntries.Keys.ToHashSet(StringComparer.Ordinal);
        var zhKeys = zhEntries.Keys.ToHashSet(StringComparer.Ordinal);
        var jaKeys = jaEntries.Keys.ToHashSet(StringComparer.Ordinal);

        Assert.Empty(enKeys.Except(zhKeys));
        Assert.Empty(zhKeys.Except(enKeys));
        Assert.Empty(enKeys.Except(jaKeys));
        Assert.Empty(jaKeys.Except(enKeys));
        Assert.Equal(enEntries.Count, zhEntries.Count);
        Assert.Equal(enEntries.Count, jaEntries.Count);

        foreach (var key in enEntries.Keys.OrderBy(k => k, StringComparer.Ordinal))
        {
            var enVal = enEntries[key];
            var zhVal = zhEntries[key];
            var jaVal = jaEntries[key];

            Assert.False(string.IsNullOrWhiteSpace(enVal), $"Strings.resx key '{key}' is empty.");
            Assert.False(string.IsNullOrWhiteSpace(zhVal), $"Strings.zh-Hans.resx key '{key}' is empty.");
            Assert.False(string.IsNullOrWhiteSpace(jaVal), $"Strings.ja.resx key '{key}' is empty.");

            var enPlaceholders = ExtractPlaceholderIndices(enVal);
            var zhPlaceholders = ExtractPlaceholderIndices(zhVal);
            var jaPlaceholders = ExtractPlaceholderIndices(jaVal);

            Assert.True(
                enPlaceholders.SetEquals(zhPlaceholders),
                $"Placeholder mismatch for '{key}' between en ({string.Join(",", enPlaceholders)}) and zh-Hans ({string.Join(",", zhPlaceholders)}).");
            Assert.True(
                enPlaceholders.SetEquals(jaPlaceholders),
                $"Placeholder mismatch for '{key}' between en ({string.Join(",", enPlaceholders)}) and ja ({string.Join(",", jaPlaceholders)}).");
        }

        // Verify every L.Get / L.Format / Localization.Get* call in src/ references an existing resource key.
        var srcRoot = Path.Combine(repoRoot, "src");
        var csFiles = Directory.EnumerateFiles(srcRoot, "*.cs", SearchOption.AllDirectories)
            .Where(path =>
                !path.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                && !path.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();

        var referencedKeys = new HashSet<string>(StringComparer.Ordinal);
        foreach (var file in csFiles)
        {
            var content = File.ReadAllText(file, Encoding.UTF8);
            foreach (Match match in ResourceCallRegex.Matches(content))
            {
                var key = match.Groups[1].Value;
                referencedKeys.Add(key);
                Assert.True(
                    enKeys.Contains(key),
                    $"Missing resource key '{key}' referenced in {Path.GetRelativePath(repoRoot, file)}.");
            }
        }

        Assert.True(referencedKeys.Count >= 400, $"Expected source files to reference localized keys, found {referencedKeys.Count}.");

        // Verify runtime ResourceManager loads distinct strings for all 3 cultures.
        string[] sampleKeys =
        [
            "Common_ErrorPrefix",
            "Wizard_Title",
            "Gui_WindowTitle",
            "BuildDb_Step1Title",
            "KnownBuilds_Matched",
        ];
        foreach (var key in sampleKeys)
        {
            var en = Localization.GetForLanguage(key, "en");
            var zh = Localization.GetForLanguage(key, "zh-Hans");
            var ja = Localization.GetForLanguage(key, "ja");
            Assert.Equal(enEntries[key], en);
            Assert.Equal(zhEntries[key], zh);
            Assert.Equal(jaEntries[key], ja);
            Assert.NotEqual(en, zh);
            Assert.NotEqual(en, ja);
            Assert.NotEqual(zh, ja);
        }
    }

    [Fact]
    public async Task LanguageSelection_FollowsStrictPriorityAndNeverFollowsOsLocale()
    {
        using var workspace = new TempWorkspace("ffr-lang-priority");

        // 1. In-process priority matrix: CLI --lang > AGP_LANG > settings.json > default "en".
        Assert.Equal("en", Localization.ResolveLanguage(null, string.Empty, null));
        Assert.Equal("ja", Localization.ResolveLanguage(null, string.Empty, "ja"));
        Assert.Equal("zh-Hans", Localization.ResolveLanguage(null, "zh-Hans", "ja"));
        Assert.Equal("en", Localization.ResolveLanguage("en", "zh-Hans", "ja"));
        Assert.Equal("zh-Hans", Localization.ResolveLanguage("ZH-HANS", "ja", "en"));
        Assert.Equal("ja", Localization.ResolveLanguage(null, "invalid-lang", "ja"));
        Assert.Equal("en", Localization.ResolveLanguage(null, "invalid-lang", "invalid-saved"));

        // OS UI culture does not override default "en".
        var previousUiCulture = CultureInfo.CurrentUICulture;
        try
        {
            CultureInfo.CurrentUICulture = CultureInfo.GetCultureInfo("ja");
            Assert.Equal("en", Localization.ResolveLanguage(null, string.Empty, null));
            CultureInfo.CurrentUICulture = CultureInfo.GetCultureInfo("zh-Hans");
            Assert.Equal("en", Localization.ResolveLanguage(null, string.Empty, null));
        }
        finally
        {
            CultureInfo.CurrentUICulture = previousUiCulture;
        }

        // 2. End-to-end CLI priority tests.
        // 2a. Default (no --lang, no AGP_LANG, no settings.json) -> English.
        var defaultHelp = await TestHarness.RunCliAsync(["--help"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, defaultHelp.ExitCode);
        Assert.Contains("Usage:", defaultHelp.Stdout, StringComparison.Ordinal);

        // 2b. Saved settings.json language ("ja") takes effect when --lang and AGP_LANG are absent.
        var settingsPath = GuiSettingsStore.GetDefaultSettingsPath(agpHomeOverride: workspace.AgpHomeDir);
        GuiSettingsStore.Save(new GuiSettings(null, false, "ja"), settingsPath);

        var savedHelp = await TestHarness.RunCliAsync(["--help"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, savedHelp.ExitCode);
        Assert.Contains("使い方:", savedHelp.Stdout, StringComparison.Ordinal);

        // 2c. AGP_LANG overrides settings.json.
        var envHelp = await TestHarness.RunCliAsync(
            ["--help"],
            agpHome: workspace.AgpHomeDir,
            extraEnv: new Dictionary<string, string?> { [Localization.LangEnvVar] = "zh-Hans" });
        Assert.Equal(0, envHelp.ExitCode);
        Assert.Contains("用法：", envHelp.Stdout, StringComparison.Ordinal);

        // 2d. CLI --lang overrides both AGP_LANG and settings.json.
        var cliOverrideHelp = await TestHarness.RunCliAsync(
            ["--lang", "en", "--help"],
            agpHome: workspace.AgpHomeDir,
            extraEnv: new Dictionary<string, string?> { [Localization.LangEnvVar] = "zh-Hans" });
        Assert.Equal(0, cliOverrideHelp.ExitCode);
        Assert.Contains("Usage:", cliOverrideHelp.Stdout, StringComparison.Ordinal);

        // 2e. Unsupported --lang value exits cleanly with exit code 2.
        var badLang = await TestHarness.RunCliAsync(["--lang", "fr", "--help"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(2, badLang.ExitCode);
        Assert.Contains("fr", badLang.Stderr, StringComparison.Ordinal);

        // 2f. `locres --lang <game-langs>` still targets game localization while global `--lang ja` selects UI language.
        var paksDir = Path.Combine(workspace.DepsDir, "ffr", "files", "FFRS", "Content", "Paks");
        Directory.CreateDirectory(paksDir);
        var enLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string> { ["Title"] = "Hello" },
        });
        await File.WriteAllBytesAsync(
            Path.Combine(paksDir, "Fixture-Windows.pak"),
            SyntheticFixtureBuilder.BuildPak([new PakFileEntry("FFRS/Content/Localization/Game/en/Game.locres", enLocres)]));

        var locresJaUi = await TestHarness.RunCliAsync(
            ["--lang", "ja", "locres", "--lang", "de", "--out", Path.Combine(workspace.DepsDir, "tmp", "locres-de")],
            ffrDeps: workspace.DepsDir,
            agpHome: workspace.AgpHomeDir);
        Assert.Equal(2, locresJaUi.ExitCode);
        Assert.Contains("エラー:", locresJaUi.Stderr, StringComparison.Ordinal);
        Assert.Contains("de", locresJaUi.Stderr, StringComparison.Ordinal);
    }

    [Fact]
    public async Task ProgressProtocol_EmitsOnlyWithProgressJsonFlagAndIsLanguageIndependent()
    {
        // 1. Unit tests for protocol formatting, parsing, and rejection of human-readable progress lines.
        var lineWithMsg = ProgressProtocol.FormatProgressLine(2, 5, "Export \"tables\" \\ JSON");
        Assert.True(ProgressProtocol.IsMachineProtocolLine(lineWithMsg));
        Assert.True(ProgressProtocol.TryParseProgressLine(lineWithMsg, out var step, out var total, out var msg));
        Assert.Equal(2, step);
        Assert.Equal(5, total);
        Assert.Equal("Export \"tables\" \\ JSON", msg);

        var lineNoMsg = ProgressProtocol.FormatProgressLine(1, 3);
        Assert.True(ProgressProtocol.TryParseProgressLine(lineNoMsg, out step, out total, out msg));
        Assert.Equal(1, step);
        Assert.Equal(3, total);
        Assert.Equal(string.Empty, msg);

        var usmapLine = ProgressProtocol.FormatUsmapLine(@"D:\Games\My Folder\FFRS.usmap");
        Assert.True(ProgressProtocol.IsMachineProtocolLine(usmapLine));
        Assert.True(ProgressProtocol.TryParseUsmapLine(usmapLine, out var parsedUsmap));
        Assert.Equal(@"D:\Games\My Folder\FFRS.usmap", parsedUsmap);

        Assert.False(ProgressProtocol.IsMachineProtocolLine("==> Step 1/5: Detect game version"));
        Assert.False(ProgressProtocol.IsMachineProtocolLine("==> 步骤 1/5：识别游戏版本与输入哈希"));
        Assert.False(ProgressProtocol.IsMachineProtocolLine("==> ステップ 1/5：ゲームバージョンと入力ハッシュを検出"));
        Assert.False(BuildCliCommand.TryParseStepProgress("==> Step 1/5: Detect game version", out _));
        Assert.False(BuildCliCommand.TryParseStepProgress("==> 步骤 1/5：识别游戏版本与输入哈希", out _));
        Assert.False(BuildCliCommand.TryParseStepProgress("==> ステップ 1/5：ゲームバージョンと入力ハッシュを検出", out _));
        Assert.False(ProgressProtocol.TryParseProgressLine("@@progress {not-json}", out _, out _, out _));
        Assert.False(ProgressProtocol.TryParseProgressLine("@@progress {\"step\":0,\"total\":5}", out _, out _, out _));
        Assert.False(ProgressProtocol.TryParseProgressLine("@@progress {\"step\":6,\"total\":5}", out _, out _, out _));
        Assert.False(ProgressProtocol.TryParseUsmapLine("@@usmap {\"path\":\"\"}", out _));

        // 2. End-to-end build-db: omitted without --progress-json; emitted identically across en / zh-Hans / ja with --progress-json.
        using var workspace = new TempWorkspace("ffr-progress-protocol");
        var (gameDir, usmapPath, fakeDbExe) = await CreateMinimalBuildDbFixtureAsync(workspace.Root);

        var outNoFlag = Path.Combine(workspace.Root, "out-no-flag");
        var noFlagRes = await TestHarness.RunCliAsync(
            ["--lang", "en", "build-db", "--game", gameDir, "--usmap", usmapPath, "--out", outNoFlag, "--ffr-db", fakeDbExe],
            agpHome: workspace.AgpHomeDir);
        Assert.True(noFlagRes.ExitCode == 0, $"stdout:\n{noFlagRes.Stdout}\nstderr:\n{noFlagRes.Stderr}");
        Assert.DoesNotContain(ProgressProtocol.ProgressPrefix, noFlagRes.Stdout, StringComparison.Ordinal);

        string[] languages = ["en", "zh-Hans", "ja"];
        string[] expectedHumanPrefixes = ["==> Step 1/5:", "==> 步骤 1/5：", "==> ステップ 1/5:"];

        for (var i = 0; i < languages.Length; i++)
        {
            var lang = languages[i];
            var outDir = Path.Combine(workspace.Root, $"out-{lang}");
            var result = await TestHarness.RunCliAsync(
                [
                    "--lang", lang,
                    "build-db",
                    "--game", gameDir,
                    "--usmap", usmapPath,
                    "--out", outDir,
                    "--ffr-db", fakeDbExe,
                    "--progress-json",
                ],
                agpHome: workspace.AgpHomeDir);
            Assert.True(result.ExitCode == 0, $"lang={lang}\nstdout:\n{result.Stdout}\nstderr:\n{result.Stderr}");
            Assert.Contains(expectedHumanPrefixes[i], result.Stdout, StringComparison.Ordinal);

            var lines = result.Stdout.Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries);
            var parsedSteps = new List<StepProgress>();
            foreach (var line in lines)
            {
                if (BuildCliCommand.TryParseStepProgress(line, out var parsed))
                {
                    parsedSteps.Add(parsed);
                }
            }

            Assert.Equal([1, 2, 3, 4, 5], parsedSteps.Select(s => s.CurrentStep).ToArray());
            Assert.All(parsedSteps, s => Assert.Equal(5, s.TotalSteps));
            Assert.All(parsedSteps, s => Assert.False(string.IsNullOrWhiteSpace(s.Title)));
        }
    }

    [Fact]
    public async Task EnglishSmokeTest_CliHelpErrorsWizardBuildDbAndGuiCore()
    {
        using var workspace = new TempWorkspace("ffr-en-smoke");

        // CLI default (en) help, unknown subcommand, and wizard EOF
        var help = await TestHarness.RunCliAsync(["--help"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, help.ExitCode);
        Assert.Contains("FINAL FANTASY RESONANCE", help.Stdout, StringComparison.Ordinal);
        Assert.Contains("Usage:", help.Stdout, StringComparison.Ordinal);
        Assert.Contains("--lang", help.Stdout, StringComparison.Ordinal);

        var unknown = await TestHarness.RunCliAsync(["--lang", "en", "no-such-cmd"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(2, unknown.ExitCode);
        Assert.Contains("Error: Unknown subcommand: no-such-cmd", unknown.Stderr, StringComparison.Ordinal);

        var wizardEof = await TestHarness.RunCliAsync(["--lang", "en"], stdinInput: string.Empty, agpHome: workspace.AgpHomeDir);
        Assert.Equal(2, wizardEof.ExitCode);
        Assert.Contains("Asset Export & Database Build Wizard", wizardEof.Stdout, StringComparison.Ordinal);

        // GUI core components under English language scope
        using var langScope = Localization.UseLanguageScope("en");
        Assert.Equal("Low (Low IL)", ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Low).DisplayText);
        Assert.Equal("Matches registered build", KnownBuildsRegistry.MessageMatched);

        var emptyUsmap = MappingDiscovery.DiscoverUsmap(
            Path.Combine(workspace.Root, "MissingGame"),
            agpUsmapDirectoryOverride: Path.Combine(workspace.AgpHomeDir, "ffr-export", "usmap"));
        Assert.Contains("Fetch .usmap", emptyUsmap.NotFoundGuidance, StringComparison.Ordinal);

        var reportDir = Path.Combine(workspace.Root, "en-report");
        WriteSyntheticBuildReport(reportDir, "matched");
        var report = BuildReportReader.ReadFromOutputDirectory(reportDir);
        Assert.Equal("Matches registered build", report.VerdictTitle);
        Assert.Contains(".usmap only", report.VerdictExplanation, StringComparison.OrdinalIgnoreCase);
    }

    [Fact]
    public async Task JapaneseSmokeTest_CliHelpErrorsWizardBuildDbAndGuiCore()
    {
        using var workspace = new TempWorkspace("ffr-ja-smoke");

        // CLI Japanese via --lang ja and AGP_LANG=ja
        var help = await TestHarness.RunCliAsync(["--lang", "ja", "--help"], agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, help.ExitCode);
        Assert.Contains("FINAL FANTASY RESONANCE", help.Stdout, StringComparison.Ordinal);
        Assert.Contains("使い方:", help.Stdout, StringComparison.Ordinal);
        Assert.Contains("--lang", help.Stdout, StringComparison.Ordinal);

        var unknown = await TestHarness.RunCliAsync(
            ["no-such-cmd"],
            agpHome: workspace.AgpHomeDir,
            extraEnv: new Dictionary<string, string?> { [Localization.LangEnvVar] = "ja" });
        Assert.Equal(2, unknown.ExitCode);
        Assert.Contains("エラー:", unknown.Stderr, StringComparison.Ordinal);
        Assert.Contains("no-such-cmd", unknown.Stderr, StringComparison.Ordinal);

        var wizardEof = await TestHarness.RunCliAsync(["--lang", "ja"], stdinInput: string.Empty, agpHome: workspace.AgpHomeDir);
        Assert.Equal(2, wizardEof.ExitCode);
        Assert.Contains("アセットエクスポート＆データベース構築ウィザード", wizardEof.Stdout, StringComparison.Ordinal);

        // GUI core components under Japanese language scope
        using var langScope = Localization.UseLanguageScope("ja");
        Assert.Equal("低整合性 (Low IL)", ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Low).DisplayText);
        Assert.Equal("登録済みビルドと一致", KnownBuildsRegistry.MessageMatched);

        var emptyUsmap = MappingDiscovery.DiscoverUsmap(
            Path.Combine(workspace.Root, "MissingGame"),
            agpUsmapDirectoryOverride: Path.Combine(workspace.AgpHomeDir, "ffr-export", "usmap"));
        Assert.Contains(".usmap を自動取得", emptyUsmap.NotFoundGuidance, StringComparison.Ordinal);

        var reportDir = Path.Combine(workspace.Root, "ja-report");
        WriteSyntheticBuildReport(reportDir, "matched");
        var report = BuildReportReader.ReadFromOutputDirectory(reportDir);
        Assert.Equal("登録済みビルドと一致", report.VerdictTitle);
        Assert.Contains(".usmap", report.VerdictExplanation, StringComparison.Ordinal);
    }

    private static Dictionary<string, string> LoadResxDictionary(string path)
    {
        Assert.True(File.Exists(path), $"Resource file not found: {path}");
        var doc = XDocument.Load(path);
        var root = doc.Root ?? throw new InvalidDataException($"Empty XML root: {path}");
        var dict = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var data in root.Elements("data"))
        {
            var name = data.Attribute("name")?.Value;
            Assert.False(string.IsNullOrWhiteSpace(name), $"Unnamed <data> entry in {path}");
            Assert.True(dict.TryAdd(name!, data.Element("value")?.Value ?? string.Empty), $"Duplicate key '{name}' in {path}");
        }

        return dict;
    }

    private static HashSet<int> ExtractPlaceholderIndices(string value)
    {
        var result = new HashSet<int>();
        foreach (Match match in PlaceholderRegex.Matches(value))
        {
            result.Add(int.Parse(match.Groups[1].Value, CultureInfo.InvariantCulture));
        }

        return result;
    }

    private static void WriteSyntheticBuildReport(string directory, string status)
    {
        Directory.CreateDirectory(directory);
        File.WriteAllText(
            Path.Combine(directory, "ffr.sqlite.build.json"),
            JsonSerializer.Serialize(new
            {
                format = 1,
                command = "build-db",
                status = "completed",
                inputs = new { jmaps = Array.Empty<object>() },
                database = new
                {
                    file_name = "ffr.sqlite",
                    size = 1024L,
                    file_sha256 = "aaaa000000000000000000000000000000000000000000000000000000000001",
                    content_sha256 = "bbbb000000000000000000000000000000000000000000000000000000000002",
                },
                known_build_check = new
                {
                    status,
                    source = "embedded",
                    matched_registered_at = "2026-11-26",
                    file_sha256_mismatch_only = false,
                    table_diffs = Array.Empty<object>(),
                },
                export_exit_codes = new { datatable = 0, assets = 0, maps = 0 },
                timings_ms = new { total = 1000L },
            }),
            new UTF8Encoding(false));
    }

    private static async Task<(string GameDir, string UsmapPath, string FakeDbExe)> CreateMinimalBuildDbFixtureAsync(string root)
    {
        var gameDir = Path.Combine(root, "Synthetic Game");
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");
        var binDir = Path.Combine(gameDir, "FFRS", "Binaries", "Win64");
        Directory.CreateDirectory(paksDir);
        Directory.CreateDirectory(binDir);

        var peBytes = SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource(
            fileVersion: (1, 0, 0, 0),
            productVersion: (1, 0, 0, 0),
            stringFileVersion: "1.0.0.0",
            stringProductVersion: "1.0.0.0",
            sectionName: ".link",
            pe32Plus: true);
        await File.WriteAllBytesAsync(Path.Combine(binDir, "FFRS-Win64-Shipping.exe"), peBytes);

        var arBytes = SyntheticFixtureBuilder.BuildAssetRegistry([
            new("/Game/Textures/T_Sample", "/Game/Textures", "T_Sample", "Texture2D"),
        ]);
        var zhLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Demo"] = new Dictionary<string, string> { ["Title"] = "示例" },
        });
        var enLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Demo"] = new Dictionary<string, string> { ["Title"] = "Sample" },
        });
        var jaLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Demo"] = new Dictionary<string, string> { ["Title"] = "サンプル" },
        });
        var pakBytes = SyntheticFixtureBuilder.BuildPak([
            new("FFRS/AssetRegistry.bin", arBytes),
            new("FFRS/Content/Localization/Game/zh-Hans/Game.locres", zhLocres),
            new("FFRS/Content/Localization/Game/en/Game.locres", enLocres),
            new("FFRS/Content/Localization/Game/ja/Game.locres", jaLocres),
            new("FFRS/Config/DefaultGame.ini", Encoding.UTF8.GetBytes("[/Script/Test]\nKey=Value\n")),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pakBytes);

        var usmapPath = Path.Combine(root, "Mappings.usmap");
        await File.WriteAllBytesAsync(usmapPath, SyntheticFixtureBuilder.BuildUsmap());

        var sourceDir = Path.Combine(AppContext.BaseDirectory, "fake-ffr-db");
        var fakeDbDir = Path.Combine(root, "fake-db");
        Directory.CreateDirectory(fakeDbDir);
        foreach (var srcFile in Directory.EnumerateFiles(sourceDir, "*", SearchOption.AllDirectories))
        {
            var relative = Path.GetRelativePath(sourceDir, srcFile);
            var dstFile = Path.Combine(fakeDbDir, relative);
            var dstParent = Path.GetDirectoryName(dstFile);
            if (!string.IsNullOrEmpty(dstParent))
            {
                Directory.CreateDirectory(dstParent);
            }

            File.Copy(srcFile, dstFile, overwrite: true);
        }

        var exeName = OperatingSystem.IsWindows() ? "FfrExport.FakeFfrDb.exe" : "FfrExport.FakeFfrDb";
        var exePath = Path.Combine(fakeDbDir, exeName);
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(
                exePath,
                UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute
                | UnixFileMode.GroupRead | UnixFileMode.GroupExecute
                | UnixFileMode.OtherRead | UnixFileMode.OtherExecute);
        }

        return (gameDir, usmapPath, exePath);
    }
}
