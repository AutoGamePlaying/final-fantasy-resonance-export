using System.Text;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class SteamLocatorAndWizardTests
{
    [Fact]
    public void ParseLibraryFoldersVdf_HandlesMultipleLibrariesEscapedBackslashesCommentsAndLegacyFormat()
    {
        // Modern Steam libraryfolders.vdf with multiple libraries, escaped backslashes, spaces, comments,
        // and a nested "apps" block containing a decoy "path" entry at depth 3.
        const string modernVdf = """
            // Synthetic Steam library folders manifest
            "libraryfolders"
            {
            	"0"
            	{
            		"path"		"C:\\Program Files (x86)\\Steam"
            		"label"		""
            		"contentid"		"111111111"
            		"apps"
            		{
            			"path"		"Z:\\Decoy\\MustIgnore"
            			"228980"		"123456"
            		}
            	}
            	"1"
            	{
            		"path"		"D:\\Games\\Steam Library"
            		"label"		"Games"
            		"apps"
            		{
            			"3259780"		"14000000000"
            		}
            	}
            	"2"
            	{
            		"path"		"E:\\Fast SSD\\New\\Tab\\Steam"
            	}
            }
            """;

        var modernPaths = SteamLocator.ParseLibraryFoldersVdf(modernVdf);
        Assert.Equal(
            [
                @"C:\Program Files (x86)\Steam",
                @"D:\Games\Steam Library",
                @"E:\Fast SSD\New\Tab\Steam",
            ],
            modernPaths);

        // Legacy libraryfolders.vdf format (direct numeric keys at depth 1)
        const string legacyVdf = """
            "LibraryFolders"
            {
            	"TimeNextStatsReport"		"1600000000"
            	"ContentStatsID"		"-987654321"
            	"1"		"D:\\Legacy Steam\\Library"
            	"2"		"F:\\Extra Library"
            }
            """;

        var legacyPaths = SteamLocator.ParseLibraryFoldersVdf(legacyVdf);
        Assert.Equal(
            [
                @"D:\Legacy Steam\Library",
                @"F:\Extra Library",
            ],
            legacyPaths);

        Assert.Empty(SteamLocator.ParseLibraryFoldersVdf("   \n\t  "));
    }

    [Fact]
    public void SteamLocator_FindsSupportedGameAcrossSyntheticSteamLibrariesAndRootStrategies()
    {
        using var workspace = new TempWorkspace("ffr-steam-locator");

        // Create a synthetic Steam installation tree with two libraries:
        // - Library 0 (Steam root): has an unrelated game + a broken 3259780 manifest missing Paks.
        // - Library 1 (secondary library with spaces): has valid appmanifest_3259780.acf + FFRS/Content/Paks.
        // - Library 2 (nonexistent path): must be ignored gracefully.
        var steamRoot = Path.Combine(workspace.Root, "Program Files (x86)", "Steam");
        var secondaryLibrary = Path.Combine(workspace.Root, "Secondary Drive", "Steam Library");
        var missingLibrary = Path.Combine(workspace.Root, "Missing Drive", "SteamLibrary");

        var rootSteamApps = Path.Combine(steamRoot, "steamapps");
        var secondarySteamApps = Path.Combine(secondaryLibrary, "steamapps");
        var gameDir = Path.Combine(secondarySteamApps, "common", "FINAL FANTASY RESONANCE");
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");

        Directory.CreateDirectory(Path.Combine(rootSteamApps, "common", "OtherGame"));
        Directory.CreateDirectory(paksDir);

        var vdfContent = $$"""
            "libraryfolders"
            {
            	"0"
            	{
            		"path"		"{{EscapeVdfString(steamRoot)}}"
            	}
            	"1"
            	{
            		"path"		"{{EscapeVdfString(secondaryLibrary)}}"
            	}
            	"2"
            	{
            		"path"		"{{EscapeVdfString(missingLibrary)}}"
            	}
            }
            """;
        File.WriteAllText(Path.Combine(rootSteamApps, "libraryfolders.vdf"), vdfContent, Encoding.UTF8);

        // Unrelated app in root library
        File.WriteAllText(
            Path.Combine(rootSteamApps, "appmanifest_228980.acf"),
            "\"AppState\" { \"appid\" \"228980\" \"name\" \"Steamworks Common Redistributables\" \"installdir\" \"OtherGame\" \"buildid\" \"100\" }",
            Encoding.UTF8);

        // Valid 共鸣 manifest in secondary library
        const string resonanceAcf = """
            "AppState"
            {
            	"appid"		"3259780"
            	"Universe"		"1"
            	"name"		"FINAL FANTASY RESONANCE"
            	"installdir"		"FINAL FANTASY RESONANCE"
            	"buildid"		"24250053"
            	"UserConfig"
            	{
            		"buildid"		"99999999"
            	}
            }
            """;
        File.WriteAllText(Path.Combine(secondarySteamApps, "appmanifest_3259780.acf"), resonanceAcf, Encoding.UTF8);

        // 1. Windows discovery prefers registry SteamPath when present
        var winFoundFromRegistry = SteamLocator.DiscoverGames(
            isWindowsOverride: true,
            windowsRegistryReader: () => steamRoot,
            defaultWindowsCandidatesOverride: [Path.Combine(workspace.Root, "unused-default")]);
        var game = Assert.Single(winFoundFromRegistry);
        Assert.Equal(3259780, game.Spec.AppId);
        Assert.Equal("ffr", game.Spec.Code);
        Assert.Equal("FINAL FANTASY RESONANCE", game.DisplayName);
        Assert.Equal(PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDir)), game.GameDirectory);
        Assert.Equal("24250053", game.SteamBuildId);

        // 2. Windows discovery falls back to default path when registry is missing or points nowhere
        var winFoundFromFallback = SteamLocator.DiscoverGames(
            isWindowsOverride: true,
            windowsRegistryReader: () => null,
            defaultWindowsCandidatesOverride: [steamRoot]);
        Assert.Single(winFoundFromFallback);
        Assert.Equal("24250053", winFoundFromFallback[0].SteamBuildId);

        // 3. Linux discovery checks ~/.steam/steam and ~/.local/share/Steam and deduplicates identical games
        var fakeHome = Path.Combine(workspace.Root, "fake-home");
        var linuxSteam1 = Path.Combine(fakeHome, ".steam", "steam");
        var linuxSteam2 = Path.Combine(fakeHome, ".local", "share", "Steam");
        Directory.CreateDirectory(Path.Combine(linuxSteam1, "steamapps"));
        Directory.CreateDirectory(Path.Combine(linuxSteam2, "steamapps"));
        File.WriteAllText(Path.Combine(linuxSteam1, "steamapps", "libraryfolders.vdf"), vdfContent, Encoding.UTF8);
        File.WriteAllText(Path.Combine(linuxSteam2, "steamapps", "libraryfolders.vdf"), vdfContent, Encoding.UTF8);

        var linuxFound = SteamLocator.DiscoverGames(
            isWindowsOverride: false,
            homeDirectoryProvider: () => fakeHome);
        var linuxGame = Assert.Single(linuxFound);
        Assert.Equal(game.GameDirectory, linuxGame.GameDirectory);

        // 4. Manifest without FFRS/Content/Paks is ignored
        Directory.Delete(paksDir, recursive: true);
        var afterMissingPaks = SteamLocator.DiscoverGamesInRoots([steamRoot]);
        Assert.Empty(afterMissingPaks);
    }

    [Fact]
    public void CleanUserPathInput_StripsSurroundingQuotesAndPreservesSpaces()
    {
        Assert.Equal(
            @"D:\Games\FINAL FANTASY RESONANCE",
            InteractiveWizard.CleanUserPathInput("\"D:\\Games\\FINAL FANTASY RESONANCE\""));
        Assert.Equal(
            @"C:\Users\Player\My Dumps\FFRS.usmap",
            InteractiveWizard.CleanUserPathInput("  \"C:\\Users\\Player\\My Dumps\\FFRS.usmap\"  "));
        Assert.Equal(
            "/tmp/path with spaces/file.usmap",
            InteractiveWizard.CleanUserPathInput("'/tmp/path with spaces/file.usmap'"));
        Assert.Equal(
            "nested quotes path",
            InteractiveWizard.CleanUserPathInput("  \"' nested quotes path '\"  "));
        Assert.Equal(
            "unmatched'quote",
            InteractiveWizard.CleanUserPathInput("unmatched'quote"));
        Assert.Equal(
            string.Empty,
            InteractiveWizard.CleanUserPathInput("   \"\"   "));
        Assert.Equal(
            string.Empty,
            InteractiveWizard.CleanUserPathInput(null));
    }

    [Fact]
    public async Task InteractiveWizard_FullFlowWithRedirectedStdin_SelectsGameStripsQuotesAndInvokesBuildDbEntry()
    {
        using var langScope = Localization.UseLanguageScope("zh-Hans");
        using var workspace = new TempWorkspace("ffr-wizard-flow");

        // Synthetic Steam library + game + usmap in directories with spaces
        var steamRoot = Path.Combine(workspace.Root, "Steam Root");
        var steamappsDir = Path.Combine(steamRoot, "steamapps");
        var gameDir = Path.Combine(steamappsDir, "common", "FINAL FANTASY RESONANCE");
        Directory.CreateDirectory(Path.Combine(gameDir, "FFRS", "Content", "Paks"));

        var acfPath = Path.Combine(steamappsDir, "appmanifest_3259780.acf");
        await File.WriteAllTextAsync(
            acfPath,
            """
            "AppState"
            {
            	"appid"		"3259780"
            	"name"		"FINAL FANTASY RESONANCE"
            	"installdir"		"FINAL FANTASY RESONANCE"
            	"buildid"		"24250053"
            }
            """);

        var mappingsDir = Path.Combine(workspace.Root, "My Mappings Dir");
        Directory.CreateDirectory(mappingsDir);
        var usmapPath = Path.Combine(mappingsDir, "Title Dump.usmap");
        await File.WriteAllBytesAsync(usmapPath, [1, 2, 3]);

        var exeDir = Path.Combine(workspace.Root, "Portable App");
        Directory.CreateDirectory(exeDir);
        var fakeFfrDbPath = Path.Combine(exeDir, "ffr-db");
        await File.WriteAllTextAsync(fakeFfrDbPath, "#!/bin/sh\n");

        // Redirected standard input:
        // 1) "1" -> choose Steam game #1
        // 2) quoted usmap path (with surrounding spaces & double quotes)
        // 3) "" -> accept default output directory (<agp>/ffr-export/output/ffr-24250053)
        // 4) "" -> confirm start
        // 5) "" -> press Enter to exit
        var stdinText = string.Join(
            Environment.NewLine,
            [
                "1",
                $"  \"{usmapPath}\"  ",
                "",
                "",
                "",
            ]) + Environment.NewLine;

        string[]? capturedBuildDbArgs = null;
        Task<int> FakeBuildDbRunner(string[] args)
        {
            capturedBuildDbArgs = args;
            var parsed = Program.ParseBuildDbArgs(args);
            Directory.CreateDirectory(parsed.OutputArgument);
            File.WriteAllBytes(Path.Combine(parsed.OutputArgument, "ffr.sqlite"), Encoding.ASCII.GetBytes("SQLite format 3\0"));

            var syntheticReport = new
            {
                format = 1,
                command = "build-db",
                status = "completed",
                cue4ParseVersion = "1.2.2.202611",
                database = new
                {
                    file_name = "ffr.sqlite",
                    file_sha256 = "aaaa000000000000000000000000000000000000000000000000000000000001",
                    content_sha256 = "bbbb000000000000000000000000000000000000000000000000000000000002",
                },
                known_build_check = new
                {
                    status = "matched",
                    message = KnownBuildsRegistry.MessageMatched,
                    source = "file",
                    matched_registered_at = "2026-11-25",
                    file_sha256_mismatch_only = false,
                    table_diffs = Array.Empty<object>(),
                },
                timings_ms = new
                {
                    export_datatable = 100,
                    export_locres = 50,
                    export_inventory = 50,
                    export_assets = 200,
                    export_maps = 100,
                    ffr_db_build = 300,
                    ffr_db_hash = 100,
                    total = 950,
                },
            };
            File.WriteAllText(
                Path.Combine(parsed.OutputArgument, "ffr.sqlite.build.json"),
                JsonConvert.SerializeObject(syntheticReport, Formatting.Indented));
            return Task.FromResult(0);
        }

        var stdout = new StringWriter();
        var stderr = new StringWriter();
        using var stdin = new StringReader(stdinText);

        var exitCode = await InteractiveWizard.RunAsync(
            stdin,
            stdout,
            stderr,
            FakeBuildDbRunner,
            () => SteamLocator.DiscoverGamesInRoots([steamRoot]),
            () => fakeFfrDbPath,
            exeDir,
            isWindowsOverride: false,
            agpHomeOverride: workspace.AgpHomeDir);

        Assert.Equal(0, exitCode);
        Assert.Empty(stderr.ToString());
        Assert.NotNull(capturedBuildDbArgs);

        var options = Program.ParseBuildDbArgs(capturedBuildDbArgs);
        var expectedDefaultOut = Path.Combine(workspace.AgpHomeDir, "ffr-export", "output", "ffr-24250053");
        Assert.Equal(PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDir)), options.GameDirectory);
        Assert.Equal(Path.GetFullPath(usmapPath), options.UsmapPath);
        Assert.DoesNotContain("--jmap", capturedBuildDbArgs);
        Assert.Equal(PathSupport.TrimTrailingSeparators(Path.GetFullPath(expectedDefaultOut)), options.OutputArgument);
        Assert.Equal("24250053", options.SteamBuildId);
        Assert.Equal(fakeFfrDbPath, options.ExplicitFfrDbPath);
        Assert.False(options.Force);

        var outputText = stdout.ToString();
        Assert.DoesNotContain(".jmap", outputText, StringComparison.OrdinalIgnoreCase);
        Assert.Contains("FINAL FANTASY RESONANCE", outputText, StringComparison.Ordinal);
        Assert.Contains("24250053", outputText, StringComparison.Ordinal);
        Assert.Contains("UE4SS", outputText, StringComparison.Ordinal);
        Assert.Contains(Path.Combine(expectedDefaultOut, "ffr.sqlite"), outputText, StringComparison.Ordinal);
        Assert.Contains("aaaa000000000000000000000000000000000000000000000000000000000001", outputText, StringComparison.Ordinal);
        Assert.Contains("bbbb000000000000000000000000000000000000000000000000000000000002", outputText, StringComparison.Ordinal);
        Assert.Contains(KnownBuildsRegistry.MessageMatched, outputText, StringComparison.Ordinal);
        Assert.Contains("按回车退出...", outputText, StringComparison.Ordinal);
    }

    [Fact]
    public async Task InteractiveWizard_HandlesMissingFfrDbManualGameFallbackAndRealBuildDbEntry()
    {
        using var langScope = Localization.UseLanguageScope("zh-Hans");
        using var workspace = new TempWorkspace("ffr-wizard-real-entry");

        // 1. Missing ffr-db reports clear Chinese message mentioning ffr-db.exe and waits for Enter
        var missingStdout = new StringWriter();
        var missingStderr = new StringWriter();
        using (var missingStdin = new StringReader(Environment.NewLine))
        {
            var missingExit = await InteractiveWizard.RunAsync(
                missingStdin,
                missingStdout,
                missingStderr,
                _ => throw new InvalidOperationException("不应调用 build-db"),
                () => [],
                () => throw new FileNotFoundException("找不到 ffr-db 可执行文件"),
                workspace.Root,
                isWindowsOverride: true);

            Assert.Equal(2, missingExit);
            Assert.Contains("ffr-db.exe", missingStderr.ToString(), StringComparison.Ordinal);
            Assert.Contains("ffr-export.exe", missingStderr.ToString(), StringComparison.Ordinal);
            Assert.Contains("下一步建议", missingStderr.ToString(), StringComparison.Ordinal);
            Assert.Contains("按回车退出...", missingStdout.ToString(), StringComparison.Ordinal);
        }

        // 2. End-to-end through Program.DefaultBuildDbRunner (the exact build-db code path)
        //    with 0 auto-detected Steam games -> manual quoted game dir, custom quoted output dir.
        var gameDir = Path.Combine(workspace.Root, "Manual Game Dir");
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

        var usmapPath = Path.Combine(workspace.Root, "Mappings With Spaces", "Fixture.usmap");
        Directory.CreateDirectory(Path.GetDirectoryName(usmapPath)!);
        await File.WriteAllBytesAsync(usmapPath, SyntheticFixtureBuilder.BuildUsmap());

        var customOutDir = Path.Combine(workspace.Root, "Custom Output Dir");
        var fakeDbExe = CopyFakeFfrDbTo(Path.Combine(workspace.Root, "bin"));

        var manualInput = string.Join(
            Environment.NewLine,
            [
                Path.Combine(workspace.Root, "nonexistent-dir"), // triggers re-prompt
                $"\"{gameDir}\"",
                Path.Combine(workspace.Root, "missing.usmap"), // triggers re-prompt
                $"\"{usmapPath}\"",
                $"\"{customOutDir}\"",
                "y", // confirm
                "", // press Enter to exit
            ]) + Environment.NewLine;

        var realStdout = new StringWriter();
        var realStderr = new StringWriter();
        using var realStdin = new StringReader(manualInput);

        var realExit = await InteractiveWizard.RunAsync(
            realStdin,
            realStdout,
            realStderr,
            Program.DefaultBuildDbRunner,
            () => [],
            () => fakeDbExe,
            workspace.Root,
            isWindowsOverride: false,
            agpHomeOverride: workspace.AgpHomeDir);

        Assert.Equal(0, realExit);
        Assert.True(File.Exists(Path.Combine(customOutDir, "ffr.sqlite")));
        var buildMeta = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(customOutDir, "ffr.sqlite.build.json")));
        Assert.Equal("completed", buildMeta["status"]?.Value<string>());

        var summaryText = realStdout.ToString();
        Assert.Contains("找不到目录", summaryText, StringComparison.Ordinal);
        Assert.Contains("找不到文件", summaryText, StringComparison.Ordinal);
        Assert.DoesNotContain(".jmap", summaryText, StringComparison.OrdinalIgnoreCase);
        Assert.Contains(Path.Combine(customOutDir, "ffr.sqlite"), summaryText, StringComparison.Ordinal);
        Assert.Contains("deadbeef00000000000000000000000000000000000000000000000000000001", summaryText, StringComparison.Ordinal);
        Assert.Contains(KnownBuildsRegistry.MessageNewGameVersion, summaryText, StringComparison.Ordinal);
        Assert.Contains("按回车退出...", summaryText, StringComparison.Ordinal);
    }

    private static string EscapeVdfString(string raw)
    {
        return raw.Replace("\\", "\\\\", StringComparison.Ordinal).Replace("\"", "\\\"", StringComparison.Ordinal);
    }

    private static string CopyFakeFfrDbTo(string directory)
    {
        var sourceDir = Path.Combine(AppContext.BaseDirectory, "fake-ffr-db");
        Directory.CreateDirectory(directory);
        foreach (var srcFile in Directory.EnumerateFiles(sourceDir, "*", SearchOption.AllDirectories))
        {
            var relative = Path.GetRelativePath(sourceDir, srcFile);
            var dstFile = Path.Combine(directory, relative);
            var dstParent = Path.GetDirectoryName(dstFile);
            if (!string.IsNullOrEmpty(dstParent))
            {
                Directory.CreateDirectory(dstParent);
            }

            File.Copy(srcFile, dstFile, overwrite: true);
        }

        var exeName = OperatingSystem.IsWindows() ? "FfrExport.FakeFfrDb.exe" : "FfrExport.FakeFfrDb";
        var exePath = Path.Combine(directory, exeName);
        if (!OperatingSystem.IsWindows())
        {
            File.SetUnixFileMode(
                exePath,
                UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute
                | UnixFileMode.GroupRead | UnixFileMode.GroupExecute
                | UnixFileMode.OtherRead | UnixFileMode.OtherExecute);
        }

        return exePath;
    }
}
