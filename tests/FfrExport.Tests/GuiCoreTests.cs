using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using FfrExport.Gui.Core;
using Xunit;

namespace FfrExport.Tests;

public sealed class GuiCoreTests
{
    [Fact]
    public void GameScanner_DiscoversSteamGamesAndValidatesManualDirectory()
    {
        using var workspace = new TempWorkspace("ffr-gui-scanner");

        var steamRoot = Path.Combine(workspace.Root, "Steam");
        var steamappsDir = Path.Combine(steamRoot, "steamapps");
        var gameDir = Path.Combine(steamappsDir, "common", "FINAL FANTASY RESONANCE");
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");
        var win64Dir = Path.Combine(gameDir, "FFRS", "Binaries", "Win64");
        Directory.CreateDirectory(paksDir);
        Directory.CreateDirectory(win64Dir);
        File.WriteAllBytes(Path.Combine(win64Dir, "FFRS-Win64-Shipping.exe"), [0x4D, 0x5A]);

        File.WriteAllText(
            Path.Combine(steamappsDir, "appmanifest_3259780.acf"),
            """
            "AppState"
            {
            	"appid"		"3259780"
            	"name"		"FINAL FANTASY RESONANCE"
            	"installdir"		"FINAL FANTASY RESONANCE"
            	"buildid"		"24250053"
            }
            """,
            Encoding.UTF8);

        var discovered = GameScanner.ScanSteamGamesInRoots([steamRoot]);
        var entry = Assert.Single(discovered);
        Assert.Equal("ffr", entry.GameCode);
        Assert.Equal("FINAL FANTASY RESONANCE", entry.DisplayName);
        Assert.Equal("24250053", entry.SteamBuildId);
        Assert.Equal(PathSupport.TrimTrailingSeparators(Path.GetFullPath(gameDir)), entry.GameDirectory);
        Assert.False(entry.IsManual);

        // Manual validation accepts quoted paths and reads steamapps manifest buildid.
        var manualOk = GameScanner.ValidateManualGameDirectory($"  \"{gameDir}\"  ");
        Assert.True(manualOk.IsValid);
        Assert.NotNull(manualOk.Entry);
        Assert.True(manualOk.Entry.IsManual);
        Assert.Equal("24250053", manualOk.Entry.SteamBuildId);

        // Missing executable is rejected with a clear Chinese error.
        var missingExeDir = Path.Combine(workspace.Root, "BrokenGame");
        Directory.CreateDirectory(Path.Combine(missingExeDir, "FFRS", "Content", "Paks"));
        var missingExe = GameScanner.ValidateManualGameDirectory(missingExeDir);
        Assert.False(missingExe.IsValid);
        Assert.Contains("FFRS-Win64-Shipping.exe", missingExe.ErrorMessage, StringComparison.Ordinal);

        // Missing Paks directory is rejected.
        var emptyDir = Path.Combine(workspace.Root, "NotAGame");
        Directory.CreateDirectory(emptyDir);
        var missingPaks = GameScanner.ValidateManualGameDirectory(emptyDir);
        Assert.False(missingPaks.IsValid);
        Assert.Contains("FFRS/Content/Paks", missingPaks.ErrorMessage, StringComparison.Ordinal);
    }

    [Fact]
    public void MappingDiscovery_FollowsUe4ssThenWin64PriorityPicksNewestUsmapAndListsJmaps()
    {
        using var workspace = new TempWorkspace("ffr-gui-mappings");

        var gameDir = Path.Combine(workspace.Root, "Game");
        var win64Dir = Path.Combine(gameDir, "FFRS", "Binaries", "Win64");
        var ue4ssDir = Path.Combine(win64Dir, "ue4ss");
        Directory.CreateDirectory(ue4ssDir);

        var emptyAgpUsmapDir = Path.Combine(workspace.AgpHomeDir, "ffr-export", "usmap");

        // 1. None found -> Chinese one-click guidance
        var emptyResult = MappingDiscovery.DiscoverUsmap(gameDir, agpUsmapDirectoryOverride: emptyAgpUsmapDir);
        Assert.Null(emptyResult.Selected);
        Assert.Empty(emptyResult.Candidates);
        Assert.Contains("一键获取 .usmap", emptyResult.NotFoundGuidance, StringComparison.Ordinal);

        // 2. Only Win64 has .usmap -> fallback finds newest in Win64
        var win64Old = Path.Combine(win64Dir, "old.usmap");
        var win64New = Path.Combine(win64Dir, "new.usmap");
        File.WriteAllBytes(win64Old, [1]);
        File.WriteAllBytes(win64New, [2]);
        File.SetLastWriteTimeUtc(win64Old, new DateTime(2026, 11, 20, 8, 0, 0, DateTimeKind.Utc));
        File.SetLastWriteTimeUtc(win64New, new DateTime(2026, 11, 24, 12, 30, 0, DateTimeKind.Utc));

        var win64Result = MappingDiscovery.DiscoverUsmap(gameDir, agpUsmapDirectoryOverride: emptyAgpUsmapDir);
        Assert.NotNull(win64Result.Selected);
        Assert.Equal("new.usmap", win64Result.Selected.FileName);
        Assert.Equal("Win64", win64Result.Selected.SourceLabel);
        Assert.Equal(2, win64Result.Candidates.Count);

        // 3. Once ue4ss/ has .usmap files, ue4ss/ takes priority over Win64 and picks the newest inside ue4ss/.
        var ue4ssOld = Path.Combine(ue4ssDir, "dump-older.usmap");
        var ue4ssNew = Path.Combine(ue4ssDir, "dump-latest.usmap");
        File.WriteAllBytes(ue4ssOld, [3]);
        File.WriteAllBytes(ue4ssNew, [4]);
        File.SetLastWriteTimeUtc(ue4ssOld, new DateTime(2026, 11, 21, 9, 0, 0, DateTimeKind.Utc));
        File.SetLastWriteTimeUtc(ue4ssNew, new DateTime(2026, 11, 25, 16, 45, 0, DateTimeKind.Utc));

        var ue4ssResult = MappingDiscovery.DiscoverUsmap(gameDir, agpUsmapDirectoryOverride: emptyAgpUsmapDir);
        Assert.NotNull(ue4ssResult.Selected);
        Assert.Equal("dump-latest.usmap", ue4ssResult.Selected.FileName);
        Assert.Equal("ue4ss", ue4ssResult.Selected.SourceLabel);
        Assert.Equal(2, ue4ssResult.Candidates.Count);
        Assert.Null(ue4ssResult.NotFoundGuidance);

        // 3b. Unified agp/ffr-export/usmap/ is also searched and wins when newest or equal-mtime.
        Directory.CreateDirectory(emptyAgpUsmapDir);
        var agpUsmap = Path.Combine(emptyAgpUsmapDir, "FFRS-24250053-af38d4a0.usmap");
        File.WriteAllBytes(agpUsmap, [5]);
        File.SetLastWriteTimeUtc(agpUsmap, new DateTime(2026, 11, 26, 10, 0, 0, DateTimeKind.Utc));

        var agpResult = MappingDiscovery.DiscoverUsmap(gameDir, agpUsmapDirectoryOverride: emptyAgpUsmapDir);
        Assert.NotNull(agpResult.Selected);
        Assert.Equal("FFRS-24250053-af38d4a0.usmap", agpResult.Selected.FileName);
        Assert.Equal("agp/usmap", agpResult.Selected.SourceLabel);
        Assert.Equal(3, agpResult.Candidates.Count);

        var formattedUsmap = MappingDiscovery.FormatUsmapDisplay(
            ue4ssResult.Selected,
            ue4ssResult.Candidates.Count,
            TimeZoneInfo.Utc);
        Assert.Contains("dump-latest.usmap", formattedUsmap, StringComparison.Ordinal);
        Assert.Contains("2026-11-25 16:45", formattedUsmap, StringComparison.Ordinal);
        Assert.Contains("共找到 2 份", formattedUsmap, StringComparison.Ordinal);

        // 4. Optional .jmap discovery lists files from both ue4ss/ and Win64/.
        var jmapTitle = Path.Combine(ue4ssDir, "title.jmap");
        var jmapBattle = Path.Combine(ue4ssDir, "battle.jmap");
        var jmapWin64 = Path.Combine(win64Dir, "extra.jmap");
        File.WriteAllText(jmapTitle, "{}", Encoding.UTF8);
        File.WriteAllText(jmapBattle, "{}", Encoding.UTF8);
        File.WriteAllText(jmapWin64, "{}", Encoding.UTF8);
        File.SetLastWriteTimeUtc(jmapTitle, new DateTime(2026, 11, 25, 10, 0, 0, DateTimeKind.Utc));
        File.SetLastWriteTimeUtc(jmapBattle, new DateTime(2026, 11, 25, 11, 0, 0, DateTimeKind.Utc));
        File.SetLastWriteTimeUtc(jmapWin64, new DateTime(2026, 11, 25, 12, 0, 0, DateTimeKind.Utc));

        var jmaps = MappingDiscovery.DiscoverJmaps(gameDir);
        Assert.Equal(3, jmaps.Count);
        Assert.Equal(["title.jmap", "battle.jmap", "extra.jmap"], jmaps.Select(j => j.FileName).ToArray());
        Assert.Contains("title.jmap", MappingDiscovery.FormatJmapDisplay(jmaps[0], TimeZoneInfo.Utc), StringComparison.Ordinal);
    }

    [Fact]
    public void GuiPaths_ComputesDefaultOutputPathResolvesCliAndChecksDirectorySafety()
    {
        using var workspace = new TempWorkspace("ffr-gui-paths");

        var fakeAgpHome = workspace.AgpHomeDir;
        var defaultRoot = GuiPaths.ResolveDefaultOutputRoot(agpHomeOverride: fakeAgpHome);
        Assert.Equal(Path.Combine(fakeAgpHome, "ffr-export", "output"), defaultRoot);

        var defaultOut = GuiPaths.BuildDefaultOutputDirectory(
            outputRootOverride: null,
            gameCode: "ffr",
            steamBuildId: "24250053",
            agpHomeOverride: fakeAgpHome);
        Assert.Equal(Path.Combine(fakeAgpHome, "ffr-export", "output", "ffr-24250053"), defaultOut);

        var unknownBuildOut = GuiPaths.BuildDefaultOutputDirectory(
            outputRootOverride: defaultRoot,
            gameCode: "ffr",
            steamBuildId: "   ");
        Assert.Equal(Path.Combine(defaultRoot, "ffr-unknown"), unknownBuildOut);

        // ExtractOutputRootDirectory avoids double nesting ffr-<buildId>.
        Assert.Equal(defaultRoot, GuiPaths.ExtractOutputRootDirectory(defaultOut, "ffr"));
        var customRoot = Path.Combine(workspace.Root, "CustomExports");
        Assert.Equal(customRoot, GuiPaths.ExtractOutputRootDirectory(customRoot, "ffr"));

        // CLI lookup checks sibling executable and produces a Chinese error when missing.
        var binDir = Path.Combine(workspace.Root, "AppDir");
        Directory.CreateDirectory(binDir);
        var missingCli = GuiPaths.ResolveCliExecutable(binDir, isWindowsOverride: true);
        Assert.False(missingCli.Found);
        Assert.Equal("ffr-export.exe", missingCli.FileName);
        Assert.Contains("ffr-export.exe", missingCli.ErrorMessage, StringComparison.Ordinal);

        File.WriteAllBytes(Path.Combine(binDir, "ffr-export.exe"), [0x4D, 0x5A]);
        var foundCli = GuiPaths.ResolveCliExecutable(binDir, isWindowsOverride: true);
        Assert.True(foundCli.Found);
        Assert.Equal(Path.Combine(binDir, "ffr-export.exe"), foundCli.ExpectedPath);

        // Directory inspection classifies not-exists, empty, own previous build, foreign non-empty, overlap, and file.
        var gameDir = Path.Combine(workspace.Root, "GameRoot");
        Directory.CreateDirectory(gameDir);

        var notExists = GuiPaths.InspectOutputDirectory(Path.Combine(workspace.Root, "FreshOut"), gameDir);
        Assert.Equal(OutputDirectoryStatus.NotExists, notExists.Status);

        var emptyDir = Path.Combine(workspace.Root, "EmptyOut");
        Directory.CreateDirectory(emptyDir);
        Assert.Equal(OutputDirectoryStatus.ExistsEmpty, GuiPaths.InspectOutputDirectory(emptyDir, gameDir).Status);

        var ownDir = Path.Combine(workspace.Root, "OwnBuildOut");
        Directory.CreateDirectory(ownDir);
        File.WriteAllText(
            Path.Combine(ownDir, GuiPaths.BuildMetadataFileName),
            """{"command":"build-db","cue4ParseVersion":"1.2.2.202611","status":"completed"}""",
            Encoding.UTF8);
        Assert.Equal(OutputDirectoryStatus.ExistsPreviousBuild, GuiPaths.InspectOutputDirectory(ownDir, gameDir).Status);

        var foreignDir = Path.Combine(workspace.Root, "ForeignOut");
        Directory.CreateDirectory(foreignDir);
        File.WriteAllText(Path.Combine(foreignDir, "user-notes.txt"), "hello", Encoding.UTF8);
        var foreignCheck = GuiPaths.InspectOutputDirectory(foreignDir, gameDir);
        Assert.Equal(OutputDirectoryStatus.ExistsForeignNonEmpty, foreignCheck.Status);
        Assert.Equal(1, foreignCheck.EntryCount);
        Assert.False(foreignCheck.IsProtectedDirectory);

        var protectedForeign = GuiPaths.InspectOutputDirectory(foreignDir, gameDir, [foreignDir]);
        Assert.Equal(OutputDirectoryStatus.ExistsForeignNonEmpty, protectedForeign.Status);
        Assert.True(protectedForeign.IsProtectedDirectory);
        Assert.Throws<InvalidOperationException>(() => GuiPaths.ClearDirectoryContents(foreignDir, [foreignDir]));

        GuiPaths.ClearDirectoryContents(foreignDir);
        Assert.Empty(Directory.EnumerateFileSystemEntries(foreignDir));

        var overlapCheck = GuiPaths.InspectOutputDirectory(Path.Combine(gameDir, "sub"), gameDir);
        Assert.Equal(OutputDirectoryStatus.OverlapsGameDirectory, overlapCheck.Status);

        var filePath = Path.Combine(workspace.Root, "existing-file.txt");
        File.WriteAllText(filePath, "x", Encoding.UTF8);
        Assert.Equal(OutputDirectoryStatus.TargetIsFile, GuiPaths.InspectOutputDirectory(filePath, gameDir).Status);
    }

    [Fact]
    public void BuildCliCommand_AssemblesArgumentsParsesStepProgressAndUsesUtf8Pipes()
    {
        var argsNoJmap = BuildCliCommand.BuildArguments(new BuildCommandRequest(
            GameDirectory: "/games/FINAL FANTASY RESONANCE",
            UsmapPath: "/games/ue4ss/FFRS.usmap",
            JmapPaths: [],
            OutputDirectory: "/out/ffr-24250053",
            SteamBuildId: "24250053",
            Force: false));

        Assert.Equal(
            [
                "build-db",
                "--game", "/games/FINAL FANTASY RESONANCE",
                "--usmap", "/games/ue4ss/FFRS.usmap",
                "--out", "/out/ffr-24250053",
                "--steam-buildid", "24250053",
            ],
            argsNoJmap);

        // Verify the assembled args after "build-db" parse identically through FfrExport's real CLI parser.
        var parsedNoJmap = Program.ParseBuildDbArgs(argsNoJmap.Skip(1).ToArray());
        Assert.Equal("/games/FINAL FANTASY RESONANCE", parsedNoJmap.GameDirectory);
        Assert.Equal("/games/ue4ss/FFRS.usmap", parsedNoJmap.UsmapPath);
        Assert.Empty(parsedNoJmap.JmapPaths);
        Assert.Equal("/out/ffr-24250053", parsedNoJmap.OutputArgument);
        Assert.Equal("24250053", parsedNoJmap.SteamBuildId);
        Assert.False(parsedNoJmap.Force);

        var argsWithJmaps = BuildCliCommand.BuildArguments(new BuildCommandRequest(
            GameDirectory: "/games/FFRS",
            UsmapPath: "/dumps/Mappings.usmap",
            JmapPaths: ["/dumps/title.jmap", "/dumps/battle.jmap"],
            OutputDirectory: "/out/ffr-unknown",
            SteamBuildId: null,
            Force: true));
        var parsedWithJmaps = Program.ParseBuildDbArgs(argsWithJmaps.Skip(1).ToArray());
        Assert.Equal(["/dumps/title.jmap", "/dumps/battle.jmap"], parsedWithJmaps.JmapPaths);
        Assert.Null(parsedWithJmaps.SteamBuildId);
        Assert.True(parsedWithJmaps.Force);

        // Progress parser tracks "==> 步骤 n/m" monotonically.
        var previousPercent = 0;
        for (var step = 1; step <= 5; step++)
        {
            Assert.True(BuildCliCommand.TryParseStepProgress($"==> 步骤 {step}/5：测试阶段 {step}", out var parsed));
            Assert.Equal(step, parsed.CurrentStep);
            Assert.Equal(5, parsed.TotalSteps);
            Assert.Equal($"测试阶段 {step}", parsed.Title);
            Assert.True(parsed.Percent > previousPercent);
            Assert.InRange(parsed.Percent, 1, 95);
            previousPercent = parsed.Percent;
        }

        Assert.False(BuildCliCommand.TryParseStepProgress("普通日志行", out _));
        Assert.False(BuildCliCommand.TryParseStepProgress("==> 步骤 0/5：非法", out _));
        Assert.False(BuildCliCommand.TryParseStepProgress("==> 步骤 6/5：越界", out _));

        // ProcessStartInfo forces UTF-8 for stdout and stderr with no console window, and forwards AGP_HOME.
        var startInfo = BuildCliCommand.CreateStartInfo("/app/ffr-export", argsNoJmap, agpHomeOverride: "/data/agp-active");
        Assert.True(startInfo.RedirectStandardOutput);
        Assert.True(startInfo.RedirectStandardError);
        Assert.Equal(Encoding.UTF8.WebName, startInfo.StandardOutputEncoding?.WebName);
        Assert.Equal(Encoding.UTF8.WebName, startInfo.StandardErrorEncoding?.WebName);
        Assert.False(startInfo.UseShellExecute);
        Assert.True(startInfo.CreateNoWindow);
        Assert.Equal("/data/agp-active", startInfo.Environment[AgpPaths.AgpHomeEnvVar]);
    }

    [Fact]
    public void BuildReportReader_ParsesStructuredBuildJsonAndExplainsBothInputCombinations()
    {
        using var workspace = new TempWorkspace("ffr-gui-report");

        // 1. Matched (usmap only, 0 jmaps)
        var matchedDir = Path.Combine(workspace.Root, "matched-usmap-only");
        Directory.CreateDirectory(matchedDir);
        File.WriteAllText(
            Path.Combine(matchedDir, "ffr.sqlite.build.json"),
            JsonSerializer.Serialize(new
            {
                format = 1,
                command = "build-db",
                status = "completed",
                inputs = new { jmaps = Array.Empty<object>() },
                database = new
                {
                    file_name = "ffr.sqlite",
                    size = 123456L,
                    file_sha256 = "aaaa000000000000000000000000000000000000000000000000000000000001",
                    content_sha256 = "bbbb000000000000000000000000000000000000000000000000000000000002",
                },
                known_build_check = new
                {
                    status = "matched",
                    message = "与登记的版本一致",
                    source = "file",
                    matched_registered_at = "2026-11-26",
                    file_sha256_mismatch_only = false,
                    table_diffs = Array.Empty<object>(),
                },
                export_exit_codes = new { datatable = 0, assets = 0, maps = 0 },
                timings_ms = new { total = 4200L },
            }),
            Encoding.UTF8);

        var matched = BuildReportReader.ReadFromOutputDirectory(matchedDir);
        Assert.Equal(BuildComparisonVerdict.Matched, matched.Verdict);
        Assert.Equal("✓", matched.VerdictSymbol);
        Assert.Equal("与登记的版本一致", matched.VerdictTitle);
        Assert.Equal(Path.Combine(matchedDir, "ffr.sqlite"), matched.DatabasePath);
        Assert.Equal("aaaa000000000000000000000000000000000000000000000000000000000001", matched.FileSha256);
        Assert.Equal("bbbb000000000000000000000000000000000000000000000000000000000002", matched.ContentSha256);
        Assert.Contains("只有 .usmap", matched.VerdictExplanation, StringComparison.Ordinal);
        Assert.Contains(".usmap 加两份 .jmap", matched.VerdictExplanation, StringComparison.Ordinal);
        Assert.Contains("2026-11-26", matched.VerdictExplanation, StringComparison.Ordinal);
        Assert.False(matched.HasPartialExportFailure);

        var clipboard = BuildReportReader.FormatClipboardHashText(matched);
        Assert.Contains(matched.FileSha256, clipboard, StringComparison.Ordinal);
        Assert.Contains(matched.ContentSha256!, clipboard, StringComparison.Ordinal);
        Assert.Contains("✓ 与登记的版本一致", clipboard, StringComparison.Ordinal);

        // 2. New version (with fallback *.build.json discovery)
        var newVerDir = Path.Combine(workspace.Root, "new-version");
        Directory.CreateDirectory(newVerDir);
        File.WriteAllText(
            Path.Combine(newVerDir, "custom.build.json"),
            JsonSerializer.Serialize(new
            {
                status = "completed",
                inputs = new { jmaps = new object[] { new { file_name = "a.jmap" }, new { file_name = "b.jmap" } } },
                database = new
                {
                    file_name = "custom.sqlite",
                    file_sha256 = "cccc000000000000000000000000000000000000000000000000000000000003",
                    content_sha256 = "dddd000000000000000000000000000000000000000000000000000000000004",
                },
                known_build_check = new
                {
                    status = "new_game_version",
                    source = "embedded",
                },
            }),
            Encoding.UTF8);

        var newVersion = BuildReportReader.ReadFromOutputDirectory(newVerDir);
        Assert.Equal(BuildComparisonVerdict.NewVersion, newVersion.Verdict);
        Assert.Equal("✗", newVersion.VerdictSymbol);
        Assert.Equal("新版本，未登记", newVersion.VerdictTitle);
        Assert.Equal(2, newVersion.JmapCount);
        Assert.Equal(Path.Combine(newVerDir, "custom.sqlite"), newVersion.DatabasePath);
        Assert.Contains("2 份 .jmap", newVersion.VerdictExplanation, StringComparison.Ordinal);

        // 3. Hash mismatch with 1 jmap and table diffs + partial export failure
        var mismatchDir = Path.Combine(workspace.Root, "hash-mismatch");
        Directory.CreateDirectory(mismatchDir);
        File.WriteAllText(
            Path.Combine(mismatchDir, "ffr.sqlite.build.json"),
            JsonSerializer.Serialize(new
            {
                status = "completed",
                inputs = new { jmaps = new object[] { new { file_name = "only-one.jmap" } } },
                database = new
                {
                    file_name = "ffr.sqlite",
                    file_sha256 = "eeee000000000000000000000000000000000000000000000000000000000005",
                    content_sha256 = "ffff000000000000000000000000000000000000000000000000000000000006",
                },
                known_build_check = new
                {
                    status = "hash_mismatch",
                    file_sha256_mismatch_only = false,
                    table_diffs = new object[] { new { name = "ue_class", kind = "changed" } },
                },
                export_exit_codes = new { datatable = 0, assets = 1, maps = 0 },
            }),
            Encoding.UTF8);

        var mismatch = BuildReportReader.ReadFromOutputDirectory(mismatchDir);
        Assert.Equal(BuildComparisonVerdict.HashMismatch, mismatch.Verdict);
        Assert.Equal("✗", mismatch.VerdictSymbol);
        Assert.Equal("哈希不同", mismatch.VerdictTitle);
        Assert.Equal(1, mismatch.TableDiffCount);
        Assert.True(mismatch.HasPartialExportFailure);
        Assert.Contains("1 张表", mismatch.VerdictExplanation, StringComparison.Ordinal);
        Assert.Contains("当前提供了 1 份 .jmap", mismatch.VerdictExplanation, StringComparison.Ordinal);
    }

    [Fact]
    public void GuiSettingsStore_PersistsToUnifiedAgpPathMigratesLegacyAndHandlesCorruptFiles()
    {
        using var workspace = new TempWorkspace("ffr-gui-settings");

        var settingsPath = GuiSettingsStore.GetDefaultSettingsPath(agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(Path.Combine(workspace.AgpHomeDir, "ffr-export", "settings.json"), settingsPath);

        // Missing file returns defaults (IncludeJmap == false).
        var initial = GuiSettingsStore.Load(settingsPath);
        Assert.Null(initial.OutputRootDirectory);
        Assert.False(initial.IncludeJmap);

        // Legacy %APPDATA%\ffr-export\settings.json is migrated to the new path and renamed to .migrated (not deleted).
        var legacyAppData = Path.Combine(workspace.Root, "LegacyAppData");
        var legacyPath = GuiSettingsStore.GetLegacySettingsPath(legacyAppData)!;
        var legacyRoot = Path.Combine(workspace.Root, "Legacy Exports");
        GuiSettingsStore.Save(new GuiSettings(legacyRoot, true), legacyPath);
        Assert.True(File.Exists(legacyPath));

        var migratedResult = GuiSettingsStore.LoadWithDiagnostics(
            settingsPath,
            legacySettingsPathOverride: legacyPath);
        Assert.Equal(legacyPath, migratedResult.MigratedFromPath);
        Assert.Null(migratedResult.WarningMessage);
        Assert.Equal(legacyRoot, migratedResult.Settings.OutputRootDirectory);
        Assert.True(migratedResult.Settings.IncludeJmap);
        Assert.True(File.Exists(settingsPath));
        Assert.False(File.Exists(legacyPath));
        Assert.True(File.Exists(legacyPath + ".migrated"));

        // Save and reload roundtrip.
        var customRoot = Path.Combine(workspace.Root, "My Exports");
        GuiSettingsStore.Save(new GuiSettings(customRoot, true), settingsPath);
        Assert.True(File.Exists(settingsPath));

        var reloaded = GuiSettingsStore.Load(settingsPath);
        Assert.Equal(customRoot, reloaded.OutputRootDirectory);
        Assert.True(reloaded.IncludeJmap);

        // Existing new settings file is NOT overwritten by a newly placed legacy file.
        GuiSettingsStore.Save(new GuiSettings(Path.Combine(workspace.Root, "IgnoredLegacy"), false), legacyPath);
        var noOverwrite = GuiSettingsStore.LoadWithDiagnostics(
            settingsPath,
            legacySettingsPathOverride: legacyPath);
        Assert.Null(noOverwrite.MigratedFromPath);
        Assert.Equal(customRoot, noOverwrite.Settings.OutputRootDirectory);
        Assert.True(File.Exists(legacyPath));

        // Corrupt JSON degrades safely to defaults.
        File.WriteAllText(settingsPath, "{ invalid json", Encoding.UTF8);
        var fallback = GuiSettingsStore.Load(settingsPath);
        Assert.Null(fallback.OutputRootDirectory);
        Assert.False(fallback.IncludeJmap);
    }

    [Fact]
    public void GuiStartupLog_WritesStartupBannerToSelectedRootWithIntegrityAndNeverFallsBackToTemp()
    {
        using var workspace = new TempWorkspace("ffr-gui-startup-log");

        var localAppData = Path.Combine(workspace.Root, "LocalAppData");
        var localLow = Path.Combine(workspace.Root, "LocalLow");
        var expectedPrimaryDir = Path.Combine(localAppData, "agp", "ffr-export", "logs");
        Assert.Equal(expectedPrimaryDir, GuiStartupLog.GetPrimaryLogDirectory(localAppData));

        var lowIntegrity = ProcessIntegritySupport.ClassifyRid(ProcessIntegritySupport.SecurityMandatoryLowRid);
        var primaryAgp = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: workspace.Root,
            executableDirectoryOverride: workspace.Root,
            integrityOverride: ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Medium));
        Assert.True(primaryAgp.Success);

        var info = new GuiStartupInfo(
            Version: "0.2.0",
            GitCommit: "abc123def456",
            OsVersion: "TestOS 10.0",
            DotNetVersion: ".NET 10.0.0",
            ProcessBitness: "64-bit (X64)",
            CurrentDirectory: workspace.Root,
            ExecutablePath: Path.Combine(workspace.Root, "ffr-export-gui.exe"),
            DpiSetting: "PerMonitorV2 (SetPerMonitorV2=True)",
            Mode: "gui",
            IntegrityLevel: primaryAgp.Integrity.FormatForLog(),
            DataRoot: primaryAgp.ToolRoot);

        var fixedTime = new DateTimeOffset(2026, 11, 26, 10, 15, 30, TimeSpan.FromHours(9));
        var session = GuiStartupLog.StartSession(
            info,
            nowOverride: fixedTime,
            agpRootResolution: primaryAgp);

        Assert.True(session.IsWritable);
        Assert.False(session.UsedFallbackDirectory);
        Assert.Equal(expectedPrimaryDir, session.LogDirectory);
        Assert.Equal(Path.Combine(expectedPrimaryDir, "gui-20261126-101530.log"), session.LogFilePath);
        Assert.True(File.Exists(session.LogFilePath));

        var lines = File.ReadAllLines(session.LogFilePath!, Encoding.UTF8);
        Assert.True(lines.Length >= 2);
        var banner = lines[0];
        Assert.Contains("mode=gui", banner, StringComparison.Ordinal);
        Assert.Contains("version=0.2.0", banner, StringComparison.Ordinal);
        Assert.Contains("git=abc123def456", banner, StringComparison.Ordinal);
        Assert.Contains("integrity=\"Medium (0x2000)\"", banner, StringComparison.Ordinal);
        Assert.Contains("os=\"TestOS 10.0\"", banner, StringComparison.Ordinal);
        Assert.Contains("dotnet=\".NET 10.0.0\"", banner, StringComparison.Ordinal);
        Assert.Contains("bits=\"64-bit (X64)\"", banner, StringComparison.Ordinal);
        Assert.Contains("dpi=\"PerMonitorV2 (SetPerMonitorV2=True)\"", banner, StringComparison.Ordinal);
        Assert.Contains($"cwd=\"{workspace.Root}\"", banner, StringComparison.Ordinal);
        Assert.Contains("ffr-export-gui.exe", banner, StringComparison.Ordinal);
        Assert.Contains($"data_root=\"{primaryAgp.ToolRoot}\"", banner, StringComparison.Ordinal);
        Assert.Contains("进程完整性级别：中完整性（Medium IL）", lines[1], StringComparison.Ordinal);

        // Same-second launch avoids overwriting the existing log file.
        var secondSession = GuiStartupLog.StartSession(
            info,
            nowOverride: fixedTime,
            agpRootResolution: primaryAgp);
        Assert.True(secondSession.IsWritable);
        Assert.NotEqual(session.LogFilePath, secondSession.LogFilePath);
        Assert.Equal(Path.Combine(expectedPrimaryDir, "gui-20261126-101530-2.log"), secondSession.LogFilePath);

        // Low IL + blocked LocalAppData writes logs to LocalLow/agp/ffr-export/logs and records integrity + skipped candidate.
        var blockingFile = Path.Combine(workspace.Root, "blocked-local-appdata");
        File.WriteAllText(blockingFile, "not a directory", Encoding.UTF8);
        var lowFallbackAgp = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: blockingFile,
            localAppDataLowOverride: localLow,
            userProfileOverride: workspace.Root,
            executableDirectoryOverride: workspace.Root,
            integrityOverride: lowIntegrity);
        Assert.True(lowFallbackAgp.Success);
        Assert.Equal(AgpRootSource.LocalAppDataLow, lowFallbackAgp.Source);

        var lowInfo = info with
        {
            IntegrityLevel = lowIntegrity.FormatForLog(),
            DataRoot = lowFallbackAgp.ToolRoot,
        };
        var fallbackSession = GuiStartupLog.StartSession(
            lowInfo,
            nowOverride: fixedTime,
            agpRootResolution: lowFallbackAgp);

        Assert.True(fallbackSession.IsWritable);
        Assert.True(fallbackSession.UsedFallbackDirectory);
        Assert.Equal(Path.Combine(localLow, "agp", "ffr-export", "logs"), fallbackSession.LogDirectory);
        Assert.NotNull(fallbackSession.LogFilePath);
        var fallbackText = File.ReadAllText(fallbackSession.LogFilePath!, Encoding.UTF8);
        Assert.Contains("integrity=\"Low (0x1000)\"", fallbackText, StringComparison.Ordinal);
        Assert.Contains("低完整性（Low IL）", fallbackText, StringComparison.Ordinal);
        Assert.Contains("[WARN] 跳过不可写候选目录", fallbackText, StringComparison.Ordinal);

        // When log creation fails or all AGP roots failed, StartSession degrades safely WITHOUT writing to %TEMP%.
        var deniedSession = GuiStartupLog.StartSession(
            info,
            nowOverride: fixedTime.AddSeconds(5),
            logsDirectoryOverride: Path.Combine(workspace.Root, "DeniedLogs"),
            directoryCreatorOverride: _ => throw new UnauthorizedAccessException("模拟无权创建日志目录"));
        Assert.False(deniedSession.IsWritable);
        Assert.Null(deniedSession.LogFilePath);
        Assert.Contains("模拟无权创建日志目录", deniedSession.LastWriteError, StringComparison.Ordinal);

        var allFailedAgp = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: Path.Combine(workspace.Root, "C1"),
            localAppDataLowOverride: Path.Combine(workspace.Root, "C2"),
            userProfileOverride: Path.Combine(workspace.Root, "C3"),
            executableDirectoryOverride: Path.Combine(workspace.Root, "C4"),
            writeProbeOverride: _ => new AgpWriteProbeResult(false, "拒绝访问"));
        Assert.False(allFailedAgp.Success);

        var totalFailure = GuiStartupLog.StartSession(
            info,
            nowOverride: fixedTime,
            agpRootResolution: allFailedAgp);
        Assert.False(totalFailure.IsWritable);
        Assert.Null(totalFailure.LogFilePath);
        Assert.NotNull(totalFailure.LastWriteError);
    }

    [Fact]
    public void GuiStartupLog_RetainsLatestTenLogsAndPreservesUnrelatedFiles()
    {
        using var workspace = new TempWorkspace("ffr-gui-log-retention");

        var localAppData = Path.Combine(workspace.Root, "LocalAppData");
        var logsDir = Path.Combine(localAppData, "agp", "ffr-export", "logs");
        Directory.CreateDirectory(logsDir);

        var baseTime = new DateTimeOffset(2026, 11, 26, 9, 0, 0, TimeSpan.Zero);
        for (var i = 0; i < 13; i++)
        {
            var stamp = baseTime.AddMinutes(i);
            var filePath = Path.Combine(logsDir, GuiStartupLog.BuildLogFileName(stamp));
            File.WriteAllText(filePath, $"log {i}\n", Encoding.UTF8);
            File.SetLastWriteTimeUtc(filePath, stamp.UtcDateTime);
        }

        // Unrelated files in the directory must never be pruned.
        var unrelatedFile = Path.Combine(logsDir, "other-tool.log");
        File.WriteAllText(unrelatedFile, "keep me", Encoding.UTF8);

        var info = GuiStartupLog.CollectStartupInfo(typeof(GuiCoreTests).Assembly, "PerMonitorV2", "gui");
        var newestTime = baseTime.AddMinutes(20);
        var session = GuiStartupLog.StartSession(
            info,
            nowOverride: newestTime,
            logsDirectoryOverride: logsDir,
            maxRetainedLogs: GuiStartupLog.DefaultMaxRetainedLogs);

        Assert.True(session.IsWritable);
        Assert.True(File.Exists(unrelatedFile));

        var remainingGuiLogs = Directory.EnumerateFiles(logsDir, "gui-*.log")
            .Select(Path.GetFileName)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();
        Assert.Equal(10, remainingGuiLogs.Length);
        Assert.Contains(Path.GetFileName(session.LogFilePath), remainingGuiLogs);

        // Oldest 4 pre-existing logs (minutes 0..3) were pruned; minutes 4..12 plus newest (20) remain.
        for (var i = 0; i < 4; i++)
        {
            Assert.DoesNotContain(GuiStartupLog.BuildLogFileName(baseTime.AddMinutes(i)), remainingGuiLogs);
        }

        for (var i = 4; i < 13; i++)
        {
            Assert.Contains(GuiStartupLog.BuildLogFileName(baseTime.AddMinutes(i)), remainingGuiLogs);
        }

        // Even with backwards clock skew, the newly created active log file is always retained.
        var skewedSession = GuiStartupLog.StartSession(
            info,
            nowOverride: baseTime.AddHours(-1),
            logsDirectoryOverride: logsDir,
            maxRetainedLogs: GuiStartupLog.DefaultMaxRetainedLogs);
        Assert.NotNull(skewedSession.LogFilePath);
        Assert.True(File.Exists(skewedSession.LogFilePath));
        Assert.Equal(10, Directory.EnumerateFiles(logsDir, "gui-*.log").Count());
    }

    [Fact]
    public void GuiStartupLog_FormatsFullExceptionChainCrashDialogAndSelfTestError()
    {
        using var workspace = new TempWorkspace("ffr-gui-exception-format");

        Exception captured;
        try
        {
            try
            {
                throw new UnauthorizedAccessException("底层拒绝访问");
            }
            catch (Exception innerMost)
            {
                throw new IOException("中层读取失败", innerMost);
            }
        }
        catch (Exception outer)
        {
            captured = new InvalidOperationException("顶层窗口初始化失败", outer);
        }

        var formatted = GuiStartupLog.FormatExceptionDetails("MainForm 构造", captured);
        Assert.Contains("阶段：MainForm 构造", formatted, StringComparison.Ordinal);
        Assert.Contains("System.InvalidOperationException", formatted, StringComparison.Ordinal);
        Assert.Contains("顶层窗口初始化失败", formatted, StringComparison.Ordinal);
        Assert.Contains("HResult：0x", formatted, StringComparison.Ordinal);
        Assert.Contains("堆栈跟踪：", formatted, StringComparison.Ordinal);
        Assert.Contains("InnerException (depth 1)", formatted, StringComparison.Ordinal);
        Assert.Contains("System.IO.IOException", formatted, StringComparison.Ordinal);
        Assert.Contains("中层读取失败", formatted, StringComparison.Ordinal);
        Assert.Contains("InnerException (depth 2)", formatted, StringComparison.Ordinal);
        Assert.Contains("System.UnauthorizedAccessException", formatted, StringComparison.Ordinal);
        Assert.Contains("底层拒绝访问", formatted, StringComparison.Ordinal);

        // AggregateException includes every inner branch.
        var aggregate = new AggregateException(
            "并行初始化失败",
            new ArgumentException("参数 A 无效"),
            new InvalidDataException("数据 B 损坏"));
        var aggText = GuiStartupLog.FormatExceptionDetails("AggregateTest", aggregate);
        Assert.Contains("AggregateException.InnerExceptions[0]", aggText, StringComparison.Ordinal);
        Assert.Contains("AggregateException.InnerExceptions[1]", aggText, StringComparison.Ordinal);
        Assert.Contains("参数 A 无效", aggText, StringComparison.Ordinal);
        Assert.Contains("数据 B 损坏", aggText, StringComparison.Ordinal);

        // Crash dialog text states startup failure in Chinese, includes exception message, log path, and developer note.
        var fakeLogPath = Path.Combine(workspace.Root, "gui-20261126-101530.log");
        var dialog = GuiStartupLog.FormatCrashDialogMessage(captured, fakeLogPath);
        Assert.Contains("窗口程序启动失败", dialog, StringComparison.Ordinal);
        Assert.Contains("InvalidOperationException: 顶层窗口初始化失败", dialog, StringComparison.Ordinal);
        Assert.Contains("IOException: 中层读取失败", dialog, StringComparison.Ordinal);
        Assert.Contains(fakeLogPath, dialog, StringComparison.Ordinal);
        Assert.Contains("发送给开发者", dialog, StringComparison.Ordinal);

        // Crash dialog degrades cleanly when the log file could not be written.
        var noLogDialog = GuiStartupLog.FormatCrashDialogMessage(captured, null, "磁盘空间不足");
        Assert.Contains("窗口程序启动失败", noLogDialog, StringComparison.Ordinal);
        Assert.Contains("未能写入日志文件", noLogDialog, StringComparison.Ordinal);
        Assert.Contains("磁盘空间不足", noLogDialog, StringComparison.Ordinal);

        // Self-test stderr report includes exception details and log path.
        var selfTestErr = GuiStartupLog.FormatSelfTestFailureMessage(captured, fakeLogPath);
        Assert.Contains("窗口程序自检失败", selfTestErr, StringComparison.Ordinal);
        Assert.Contains(fakeLogPath, selfTestErr, StringComparison.Ordinal);
        Assert.Contains("System.UnauthorizedAccessException", selfTestErr, StringComparison.Ordinal);

        // Session WriteException persists the full formatted trace to disk.
        var session = GuiStartupLog.StartSession(
            GuiStartupLog.CollectStartupInfo(typeof(GuiCoreTests).Assembly, "PerMonitorV2", "self-test"),
            logsDirectoryOverride: Path.Combine(workspace.Root, "LocalAppData", "agp", "ffr-export", "logs"));
        session.WriteException("self-test", captured);
        var diskLog = File.ReadAllText(session.LogFilePath!, Encoding.UTF8);
        Assert.Contains("顶层窗口初始化失败", diskLog, StringComparison.Ordinal);
        Assert.Contains("底层拒绝访问", diskLog, StringComparison.Ordinal);
    }

    [Fact]
    public void GuiFallbacks_HandleCorruptOrDeniedSettingsFontOutputSteamAndMappings()
    {
        using var workspace = new TempWorkspace("ffr-gui-fallbacks");

        // 1. Settings load diagnostics: missing -> no warning; corrupt / permission denied / invalid path -> warning + defaults.
        var missingSettings = GuiSettingsStore.LoadWithDiagnostics(Path.Combine(workspace.Root, "missing.json"));
        Assert.Equal(GuiSettings.Default, missingSettings.Settings);
        Assert.Null(missingSettings.WarningMessage);

        var corruptPath = Path.Combine(workspace.Root, "corrupt.json");
        File.WriteAllText(corruptPath, "{ not valid json", Encoding.UTF8);
        var corruptSettings = GuiSettingsStore.LoadWithDiagnostics(corruptPath);
        Assert.Equal(GuiSettings.Default, corruptSettings.Settings);
        Assert.Contains("已损坏", corruptSettings.WarningMessage, StringComparison.Ordinal);

        var deniedSettings = GuiSettingsStore.LoadWithDiagnostics(
            corruptPath,
            fileReaderOverride: _ => throw new UnauthorizedAccessException("拒绝读取设置"));
        Assert.Equal(GuiSettings.Default, deniedSettings.Settings);
        Assert.Contains("无权读取配置文件", deniedSettings.WarningMessage, StringComparison.Ordinal);

        var invalidPathSettings = GuiSettingsStore.LoadWithDiagnostics("bad\0settings.json");
        Assert.Equal(GuiSettings.Default, invalidPathSettings.Settings);
        Assert.NotNull(invalidPathSettings.WarningMessage);

        // 2. Font fallback chain: preferred -> MessageBoxFont (same style) -> MessageBoxFont (regular) -> DefaultFont.
        var okFont = GuiFontFallback.ResolveUiFont(
            "Microsoft YaHei UI",
            9.5f,
            isBold: true,
            tryCreatePreferred: (name, _, _) => $"{name}-Bold",
            tryCreateMessageBoxFont: (_, _) => "MsgBox",
            getFallbackDefaultFont: () => "Default");
        Assert.Equal("Microsoft YaHei UI-Bold", okFont.Font);
        Assert.True(okFont.IsOwned);
        Assert.False(okFont.UsedFallback);
        Assert.Null(okFont.WarningMessage);

        var msgBoxFallback = GuiFontFallback.ResolveUiFont(
            "Microsoft YaHei UI",
            9.5f,
            isBold: false,
            tryCreatePreferred: (_, _, _) => throw new ArgumentException("字体不存在"),
            tryCreateMessageBoxFont: (_, bold) => bold ? "MsgBox-Bold" : "MsgBox-Regular",
            getFallbackDefaultFont: () => "Default");
        Assert.Equal("MsgBox-Regular", msgBoxFallback.Font);
        Assert.True(msgBoxFallback.IsOwned);
        Assert.True(msgBoxFallback.UsedFallback);
        Assert.Contains("MessageBoxFont", msgBoxFallback.WarningMessage, StringComparison.Ordinal);

        var regularMsgBoxFallback = GuiFontFallback.ResolveUiFont(
            "Microsoft YaHei UI",
            11f,
            isBold: true,
            tryCreatePreferred: (_, _, _) => null,
            tryCreateMessageBoxFont: (_, bold) => bold ? throw new ArgumentException("不支持粗体") : "MsgBox-Regular",
            getFallbackDefaultFont: () => "Default");
        Assert.Equal("MsgBox-Regular", regularMsgBoxFallback.Font);
        Assert.True(regularMsgBoxFallback.UsedFallback);

        var defaultFontFallback = GuiFontFallback.ResolveUiFont(
            "Microsoft YaHei UI",
            9.5f,
            isBold: true,
            tryCreatePreferred: (_, _, _) => throw new ArgumentException("缺失"),
            tryCreateMessageBoxFont: (_, _) => throw new InvalidOperationException("系统字体异常"),
            getFallbackDefaultFont: () => "ControlDefault");
        Assert.Equal("ControlDefault", defaultFontFallback.Font);
        Assert.False(defaultFontFallback.IsOwned);
        Assert.True(defaultFontFallback.UsedFallback);

        var monoFallback = GuiFontFallback.ResolveMonoFont(
            "Consolas",
            9f,
            tryCreatePreferred: (_, _) => throw new ArgumentException("缺少 Consolas"),
            tryCreateGenericMonospace: _ => null,
            tryCreateMessageBoxFont: (_, _) => "MsgBox-MonoFallback",
            getFallbackDefaultFont: () => "ControlDefault");
        Assert.Equal("MsgBox-MonoFallback", monoFallback.Font);
        Assert.True(monoFallback.UsedFallback);

        // 3. Default output directory fallback when LocalAppData is unwritable falls back to LocalLow (never TEMP).
        var blockedLocal = Path.Combine(workspace.Root, "BlockedLocal");
        var writableLocalLow = Path.Combine(workspace.Root, "WritableLocalLow");
        var exeDir = Path.Combine(workspace.Root, "PortableExe");

        var primaryFallback = GuiPaths.ResolveDefaultOutputRootSafe(
            agpHomeOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: blockedLocal,
            localAppDataLowOverride: writableLocalLow,
            userProfileOverride: workspace.Root,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path => path.StartsWith(blockedLocal, StringComparison.Ordinal)
                ? new AgpWriteProbeResult(false, "拒绝访问")
                : new AgpWriteProbeResult(true),
            integrityOverride: ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Low));
        Assert.True(primaryFallback.UsedFallback);
        Assert.Equal(Path.Combine(writableLocalLow, "agp", "ffr-export", "output"), primaryFallback.Path);
        Assert.Contains("LocalLow", primaryFallback.WarningMessage, StringComparison.Ordinal);
        Assert.Contains("解除锁定", primaryFallback.WarningMessage, StringComparison.Ordinal);

        var badSavedRoot = GuiPaths.BuildDefaultOutputDirectorySafe(
            outputRootOverride: "bad\0saved-root",
            gameCode: "ffr",
            steamBuildId: "24250053",
            agpHomeOverride: workspace.AgpHomeDir,
            writeProbeOverride: _ => new AgpWriteProbeResult(true));
        Assert.True(badSavedRoot.UsedFallback);
        Assert.Equal(Path.Combine(workspace.AgpHomeDir, "ffr-export", "output", "ffr-24250053"), badSavedRoot.Path);
        Assert.Contains("上次保存的输出目录无效", badSavedRoot.WarningMessage, StringComparison.Ordinal);

        // 4. Steam scan fallback on registry permission error and unreadable Steam library directory.
        var registryDenied = GameScanner.ScanSteamGamesSafe(
            isWindowsOverride: true,
            windowsRegistryReader: () => throw new UnauthorizedAccessException("注册表访问被拒绝"),
            defaultWindowsCandidatesOverride: [Path.Combine(workspace.Root, "NonExistentSteam")]);
        Assert.Empty(registryDenied.Games);
        Assert.Contains("注册表", registryDenied.ErrorMessage, StringComparison.Ordinal);
        Assert.Contains("手动选择游戏目录", registryDenied.ErrorMessage, StringComparison.Ordinal);

        var restrictedSteamRoot = Path.Combine(workspace.Root, "RestrictedSteam");
        Directory.CreateDirectory(Path.Combine(restrictedSteamRoot, "steamapps"));
        var dirDenied = GameScanner.ScanSteamGamesSafe(
            isWindowsOverride: true,
            windowsRegistryReader: () => restrictedSteamRoot,
            defaultWindowsCandidatesOverride: [],
            steamDirectoryAccessProbe: _ => throw new UnauthorizedAccessException("Steam 目录无读权限"));
        Assert.Empty(dirDenied.Games);
        Assert.Contains("无权访问", dirDenied.ErrorMessage, StringComparison.Ordinal);
        Assert.Contains("手动选择游戏目录", dirDenied.ErrorMessage, StringComparison.Ordinal);

        // 5. .usmap / .jmap discovery reports permission errors (e.g. Program Files) without throwing.
        var protectedGameDir = Path.Combine(workspace.Root, "ProtectedGame");
        var usmapDenied = MappingDiscovery.DiscoverUsmap(
            protectedGameDir,
            fileEnumeratorOverride: _ => throw new UnauthorizedAccessException("Program Files 拒绝读取"));
        Assert.Null(usmapDenied.Selected);
        Assert.Contains("无权读取目录", usmapDenied.ScanError, StringComparison.Ordinal);
        Assert.Contains("选择 .usmap", usmapDenied.NotFoundGuidance, StringComparison.Ordinal);

        var jmapDenied = MappingDiscovery.DiscoverJmapsWithDiagnostics(
            protectedGameDir,
            fileEnumeratorOverride: _ => throw new UnauthorizedAccessException("Program Files 拒绝读取"));
        Assert.Empty(jmapDenied.Files);
        Assert.Contains("无权读取目录", jmapDenied.ScanError, StringComparison.Ordinal);

        // Partial permission failure: ue4ss denied, Win64 readable -> still selects Win64 .usmap and preserves warning.
        var partialGameDir = Path.Combine(workspace.Root, "PartialGame");
        var win64Usmap = Path.Combine(partialGameDir, "FFRS", "Binaries", "Win64", "fallback.usmap");
        Directory.CreateDirectory(Path.GetDirectoryName(win64Usmap)!);
        File.WriteAllBytes(win64Usmap, [1, 2, 3]);
        var partialUsmap = MappingDiscovery.DiscoverUsmap(
            partialGameDir,
            fileEnumeratorOverride: dir =>
            {
                if (dir.EndsWith("ue4ss", StringComparison.OrdinalIgnoreCase))
                {
                    throw new UnauthorizedAccessException("ue4ss 无读权限");
                }

                if (dir.EndsWith("usmap", StringComparison.OrdinalIgnoreCase))
                {
                    return [];
                }

                return [win64Usmap];
            });
        Assert.NotNull(partialUsmap.Selected);
        Assert.Equal("fallback.usmap", partialUsmap.Selected.FileName);
        Assert.Contains("ue4ss 无读权限", partialUsmap.ScanError, StringComparison.Ordinal);

        // Edge timestamps near MinValue do not throw ArgumentOutOfRangeException on negative timezone offsets.
        var minusOffsetZone = TimeZoneInfo.CreateCustomTimeZone("TestMinus", TimeSpan.FromHours(-8), "TestMinus", "TestMinus");
        var formattedEdge = MappingDiscovery.FormatModifiedDate(DateTimeOffset.MinValue.AddHours(1), minusOffsetZone);
        Assert.Contains("0001-01-01", formattedEdge, StringComparison.Ordinal);
    }

    [Fact]
    public async Task BuildCliCommand_RunsChildProcessWithUtf8OutputAndSupportsCancellation()
    {
        var cliHostDll = Path.Combine(AppContext.BaseDirectory, "cli-host", "FfrExport.TestCliHost.dll");
        Assert.True(File.Exists(cliHostDll));

        var dotnetExe = ResolveDotnetForTest();
        var stdoutLines = new List<string>();
        var stderrLines = new List<string>();

        // Running an invalid subcommand verifies UTF-8 Chinese stderr streaming through BuildCliCommand.RunAsync.
        var exitCode = await BuildCliCommand.RunAsync(
            dotnetExe,
            ["exec", cliHostDll, "unknown-gui-probe"],
            line =>
            {
                lock (stdoutLines)
                {
                    stdoutLines.Add(line);
                }
            },
            line =>
            {
                lock (stderrLines)
                {
                    stderrLines.Add(line);
                }
            },
            CancellationToken.None);

        Assert.Equal(2, exitCode);
        Assert.Contains(stderrLines, line => line.Contains("未知子命令", StringComparison.Ordinal));

        // Verify pre-cancelled token cancels before spawning.
        using (var preCancelled = new CancellationTokenSource())
        {
            preCancelled.Cancel();
            await Assert.ThrowsAnyAsync<OperationCanceledException>(() =>
                BuildCliCommand.RunAsync(
                    dotnetExe,
                    ["exec", cliHostDll, "unknown-gui-probe"],
                    _ => { },
                    _ => { },
                    preCancelled.Token));
        }

        // Verify live cancellation terminates an active child process tree cleanly.
        var (sleepExe, sleepArgs) = OperatingSystem.IsWindows()
            ? ("cmd.exe", new[] { "/c", "ping -n 30 127.0.0.1 > nul" })
            : ("/bin/sh", new[] { "-c", "sleep 30" });
        using var liveCts = new CancellationTokenSource(TimeSpan.FromMilliseconds(250));
        await Assert.ThrowsAnyAsync<OperationCanceledException>(() =>
            BuildCliCommand.RunAsync(
                sleepExe,
                sleepArgs,
                _ => { },
                _ => { },
                liveCts.Token));
    }

    [Fact]
    public async Task DirectFfrExportProcess_RedirectedPipesEmitBomlessUtf8ChineseAndStepProgress()
    {
        using var workspace = new TempWorkspace("ffr-direct-utf8");
        var (exePath, prefixArgs) = ResolveDirectFfrExportInvocation();

        // 1. Direct invocation with an invalid subcommand must emit valid UTF-8 Chinese on stderr (no TestCliHost).
        var errStdout = new List<string>();
        var errStderr = new List<string>();
        var invalidExit = await BuildCliCommand.RunAsync(
            exePath,
            [.. prefixArgs, "unknown-utf8-probe"],
            line =>
            {
                lock (errStdout)
                {
                    errStdout.Add(line);
                }
            },
            line =>
            {
                lock (errStderr)
                {
                    errStderr.Add(line);
                }
            },
            CancellationToken.None);

        Assert.Equal(2, invalidExit);
        Assert.NotEmpty(errStderr);
        Assert.False(errStderr[0].StartsWith('\uFEFF'), "重定向 stderr 不得包含 UTF-8 BOM");
        var stderrText = string.Join(Environment.NewLine, errStderr);
        Assert.Contains("错误：未知子命令：unknown-utf8-probe", stderrText, StringComparison.Ordinal);
        Assert.DoesNotContain('\uFFFD', stderrText);

        // 2. Direct build-db invocation on synthetic fixtures must emit UTF-8 Chinese stdout matching "==> 步骤 n/m".
        var gameDir = Path.Combine(workspace.Root, "Synthetic Game");
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

        var usmapPath = Path.Combine(workspace.Root, "Mappings.usmap");
        await File.WriteAllBytesAsync(usmapPath, SyntheticFixtureBuilder.BuildUsmap());

        var fakeDbExe = CopyFakeFfrDbTo(Path.Combine(workspace.Root, "fake-db"));
        var outDir = Path.Combine(workspace.Root, "out-utf8");

        var buildStdout = new List<string>();
        var buildStderr = new List<string>();
        var parsedSteps = new List<StepProgress>();

        var buildExit = await BuildCliCommand.RunAsync(
            exePath,
            [
                .. prefixArgs,
                "build-db",
                "--game", gameDir,
                "--usmap", usmapPath,
                "--out", outDir,
                "--ffr-db", fakeDbExe,
            ],
            line =>
            {
                lock (buildStdout)
                {
                    buildStdout.Add(line);
                    if (BuildCliCommand.TryParseStepProgress(line, out var step))
                    {
                        parsedSteps.Add(step);
                    }
                }
            },
            line =>
            {
                lock (buildStderr)
                {
                    buildStderr.Add(line);
                }
            },
            CancellationToken.None);

        Assert.Equal(0, buildExit);
        Assert.NotEmpty(buildStdout);
        Assert.False(buildStdout[0].StartsWith('\uFEFF'), "重定向 stdout 不得包含 UTF-8 BOM");
        var stdoutText = string.Join(Environment.NewLine, buildStdout);
        Assert.DoesNotContain('\uFFFD', stdoutText);
        Assert.Contains("==> 步骤 1/5：识别游戏版本与输入哈希", stdoutText, StringComparison.Ordinal);
        Assert.Contains("建库完成：", stdoutText, StringComparison.Ordinal);
        Assert.Equal([1, 2, 3, 4, 5], parsedSteps.Select(s => s.CurrentStep).ToArray());
        Assert.All(parsedSteps, s => Assert.Equal(5, s.TotalSteps));

        var report = BuildReportReader.ReadFromOutputDirectory(outDir);
        Assert.Equal(BuildComparisonVerdict.NewVersion, report.Verdict);
    }

    [Fact]
    public void GuiProject_EnforcesStrictWinFormsSettingsSharesSteamLocatorAndContainsNoForbiddenTokens()
    {
        var root = TestHarness.FindRepoRoot();
        var guiDir = Path.Combine(root, "src", "FfrExport.Gui");
        var csproj = File.ReadAllText(Path.Combine(guiDir, "FfrExport.Gui.csproj"), Encoding.UTF8);

        Assert.Contains("<OutputType>WinExe</OutputType>", csproj, StringComparison.Ordinal);
        Assert.Contains("<TargetFramework>net10.0-windows</TargetFramework>", csproj, StringComparison.Ordinal);
        Assert.Contains("<UseWindowsForms>true</UseWindowsForms>", csproj, StringComparison.Ordinal);
        Assert.Contains("<EnableWindowsTargeting>true</EnableWindowsTargeting>", csproj, StringComparison.Ordinal);
        Assert.Contains("<AssemblyName>ffr-export-gui</AssemblyName>", csproj, StringComparison.Ordinal);
        Assert.Contains("<Nullable>enable</Nullable>", csproj, StringComparison.Ordinal);
        Assert.Contains("<TreatWarningsAsErrors>true</TreatWarningsAsErrors>", csproj, StringComparison.Ordinal);
        Assert.Contains(@"..\FfrExport\SteamLocator.cs", csproj, StringComparison.Ordinal);
        Assert.False(File.Exists(Path.Combine(guiDir, "SteamLocator.cs")), "SteamLocator 应共享链接而不是复制一份");

        var programCs = File.ReadAllText(Path.Combine(guiDir, "Program.cs"), Encoding.UTF8);
        Assert.Contains("Application.ThreadException", programCs, StringComparison.Ordinal);
        Assert.Contains("AppDomain.CurrentDomain.UnhandledException", programCs, StringComparison.Ordinal);
        Assert.Contains("Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException)", programCs, StringComparison.Ordinal);
        Assert.Contains("new MainForm(", programCs, StringComparison.Ordinal);
        Assert.Contains("form.CreateControl()", programCs, StringComparison.Ordinal);
        Assert.Contains("ProcessIntegritySupport.DetectCurrent()", programCs, StringComparison.Ordinal);

        var mainFormCs = File.ReadAllText(Path.Combine(guiDir, "MainForm.cs"), Encoding.UTF8);
        Assert.Contains("清理 UE4SS…", mainFormCs, StringComparison.Ordinal);
        Assert.Contains("agpHomeOverride: _activeAgpRoot", mainFormCs, StringComparison.Ordinal);
        Assert.Contains("StatusStrip", mainFormCs, StringComparison.Ordinal);

        string[] forbiddenTokens = ["OodleHelper", "DownloadOodle", "HttpClient", "WebClient"];
        var linuxRootRegex = new Regex(@"/root/", RegexOptions.CultureInvariant);
        var windowsDriveRegex = new Regex(@"(?<![A-Za-z0-9+.-])[A-Za-z]:[\\/]", RegexOptions.CultureInvariant);

        foreach (var file in Directory.EnumerateFiles(guiDir, "*.cs", SearchOption.AllDirectories))
        {
            if (file.Contains($"{Path.DirectorySeparatorChar}obj{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                || file.Contains($"{Path.DirectorySeparatorChar}bin{Path.DirectorySeparatorChar}", StringComparison.Ordinal))
            {
                continue;
            }

            var content = File.ReadAllText(file, Encoding.UTF8);
            foreach (var token in forbiddenTokens)
            {
                Assert.False(content.Contains(token, StringComparison.Ordinal), $"{file} 不得包含 {token}");
            }

            Assert.DoesNotMatch(linuxRootRegex, content);
            Assert.DoesNotMatch(windowsDriveRegex, content);
        }
    }

    [Fact]
    public void AgpPaths_CandidateOrderLowIntegrityExplicitAgpHomeAndAllUnwritableHandling()
    {
        using var workspace = new TempWorkspace("ffr-agp-candidates");

        var envAgp = Path.Combine(workspace.Root, "EnvAgp");
        var localAppData = Path.Combine(workspace.Root, "LocalAppData");
        var localLow = Path.Combine(workspace.Root, "LocalLow");
        var exeDir = Path.Combine(workspace.Root, "AppExe");
        var userProfile = Path.Combine(workspace.Root, "UserHome");
        var savedCustom = Path.Combine(workspace.Root, "SavedCustom");
        Directory.CreateDirectory(exeDir);

        var expectedLocal = Path.Combine(localAppData, "agp");
        var expectedLocalLow = Path.Combine(localLow, "agp");
        var expectedPortable = Path.Combine(exeDir, "agp-data");
        var expectedUserProfile = Path.Combine(userProfile, "agp");

        // 1. Integrity level RID classification & injection
        Assert.Equal(ProcessIntegrityLevel.Untrusted, ProcessIntegritySupport.ClassifyRid(0x0000).Level);
        Assert.Equal(ProcessIntegrityLevel.Low, ProcessIntegritySupport.ClassifyRid(0x1000).Level);
        Assert.True(ProcessIntegritySupport.ClassifyRid(0x1000).IsLowOrUntrusted);
        Assert.Equal(ProcessIntegrityLevel.Medium, ProcessIntegritySupport.ClassifyRid(0x2000).Level);
        Assert.Equal(ProcessIntegrityLevel.Medium, ProcessIntegritySupport.ClassifyRid(0x2100).Level);
        Assert.Equal(ProcessIntegrityLevel.High, ProcessIntegritySupport.ClassifyRid(0x3000).Level);
        Assert.Equal(ProcessIntegrityLevel.System, ProcessIntegritySupport.ClassifyRid(0x4000).Level);
        var injectedLow = ProcessIntegritySupport.DetectCurrent(
            () => ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Low));
        Assert.Equal(ProcessIntegrityLevel.Low, injectedLow.Level);
        Assert.Equal("低完整性（Low IL）", injectedLow.DisplayTextZh);

        // 2. Explicit AGP_HOME fails immediately without falling back when unwritable.
        var envProbed = new List<string>();
        var envFail = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: envAgp,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: userProfile,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path =>
            {
                envProbed.Add(path);
                return new AgpWriteProbeResult(false, "拒绝写入 AGP_HOME");
            });
        Assert.False(envFail.Success);
        Assert.Equal(AgpRootSource.AgpHomeEnv, envFail.Source);
        Assert.Equal([envAgp], envProbed);
        Assert.Contains("AGP_HOME", envFail.ErrorMessage, StringComparison.Ordinal);
        Assert.Contains("不会自动回退", envFail.ErrorMessage, StringComparison.Ordinal);

        // 3. Exact candidate probe order under Low IL when all fail:
        //    %LOCALAPPDATA%\agp -> %USERPROFILE%\AppData\LocalLow\agp -> <exeDir>\agp-data -> %USERPROFILE%\agp -> agp-location.txt
        AgpPaths.TrySaveLocationFile(exeDir, savedCustom, out _);
        var probedOrder = new List<string>();
        var allFailed = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: userProfile,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path =>
            {
                probedOrder.Add(path);
                return new AgpWriteProbeResult(false, "权限不足");
            },
            integrityOverride: injectedLow);
        Assert.False(allFailed.Success);
        Assert.Equal(
            [expectedLocal, expectedLocalLow, expectedPortable, expectedUserProfile, savedCustom],
            probedOrder);
        Assert.Contains("AGP_HOME", allFailed.ErrorMessage, StringComparison.Ordinal);
        Assert.Contains("解除锁定", allFailed.ErrorMessage, StringComparison.Ordinal);

        // GUI all-failed message mentions security software / Low IL, choosing folder, and admin only as last resort.
        var guiAllFailedMedium = AgpPaths.FormatAllCandidatesFailedGuiMessage(
            allFailed with { Integrity = ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Medium) });
        Assert.Contains("安全软件", guiAllFailedMedium, StringComparison.Ordinal);
        Assert.Contains("白名单", guiAllFailedMedium, StringComparison.Ordinal);
        Assert.Contains("选择文件夹", guiAllFailedMedium, StringComparison.Ordinal);
        Assert.Contains("最后手段", guiAllFailedMedium, StringComparison.Ordinal);

        // 4. Low IL with LocalAppData blocked picks LocalLow and formats Low IL unblock guidance.
        var lowLocalLowWin = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: userProfile,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path => path == expectedLocalLow
                ? new AgpWriteProbeResult(true)
                : new AgpWriteProbeResult(false, "拒绝访问"),
            integrityOverride: injectedLow);
        Assert.True(lowLocalLowWin.Success);
        Assert.Equal(expectedLocalLow, lowLocalLowWin.AgpRoot);
        Assert.Equal(AgpRootSource.LocalAppDataLow, lowLocalLowWin.Source);
        Assert.True(lowLocalLowWin.UsedFallback);

        var lowBanner = AgpPaths.FormatGuiBannerMessage(lowLocalLowWin);
        Assert.NotNull(lowBanner);
        Assert.Contains("低完整性级别", lowBanner, StringComparison.Ordinal);
        Assert.Contains("解除锁定", lowBanner, StringComparison.Ordinal);
        Assert.Contains("资源管理器重新解压", lowBanner, StringComparison.Ordinal);
        Assert.Contains("LocalLow", lowBanner, StringComparison.Ordinal);

        // 5. Fallback to portable <exeDir>\agp-data or %USERPROFILE%\agp displays "数据目录：...（原因：%LOCALAPPDATA% 不可写）".
        var mediumIntegrity = ProcessIntegritySupport.FromLevel(ProcessIntegrityLevel.Medium);
        var portableWin = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: userProfile,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path => path == expectedPortable
                ? new AgpWriteProbeResult(true)
                : new AgpWriteProbeResult(false, "拒绝访问"),
            integrityOverride: mediumIntegrity);
        Assert.True(portableWin.Success);
        Assert.Equal(expectedPortable, portableWin.AgpRoot);
        Assert.Equal(AgpRootSource.PortableExeDir, portableWin.Source);
        Assert.Contains("数据目录：", AgpPaths.FormatGuiBannerMessage(portableWin), StringComparison.Ordinal);
        Assert.Contains("%LOCALAPPDATA% 不可写", AgpPaths.FormatGuiBannerMessage(portableWin), StringComparison.Ordinal);
        Assert.Contains("数据目录：", AgpPaths.FormatCliFallbackNotice(portableWin), StringComparison.Ordinal);
        Assert.Contains("%LOCALAPPDATA% 不可写", AgpPaths.FormatCliFallbackNotice(portableWin), StringComparison.Ordinal);

        var userProfileWin = AgpPaths.ResolveWritableAgpRoot(
            agpHomeEnvOverride: string.Empty,
            isWindowsOverride: true,
            localAppDataOverride: localAppData,
            localAppDataLowOverride: localLow,
            userProfileOverride: userProfile,
            executableDirectoryOverride: exeDir,
            writeProbeOverride: path => path == expectedUserProfile
                ? new AgpWriteProbeResult(true)
                : new AgpWriteProbeResult(false, "拒绝访问"),
            integrityOverride: mediumIntegrity);
        Assert.True(userProfileWin.Success);
        Assert.Equal(expectedUserProfile, userProfileWin.AgpRoot);
        Assert.Equal(AgpRootSource.UserProfile, userProfileWin.Source);

        // 6. Real write probe creates <agpRoot>/ffr-export/tmp and cleans up the probe file afterwards.
        var realProbeRoot = Path.Combine(workspace.Root, "RealProbe");
        var probeRes = AgpPaths.ProbeDirectoryWritable(realProbeRoot);
        Assert.True(probeRes.IsWritable);
        var realTmpDir = AgpPaths.GetTempDirectoryForAgpRoot(realProbeRoot);
        Assert.True(Directory.Exists(realTmpDir));
        Assert.Empty(Directory.EnumerateFiles(realTmpDir));

        // 7. User-selected directory saves agp-location.txt when exeDir is writable, and still succeeds if exeDir is blocked.
        var manualDir = Path.Combine(workspace.Root, "ManualChosen");
        var accepted = AgpPaths.TryAcceptUserSelectedAgpRoot(manualDir, mediumIntegrity, executableDirectoryOverride: exeDir);
        Assert.True(accepted.Success);
        Assert.True(accepted.LocationFileSaved);
        Assert.Equal(manualDir, AgpPaths.TryReadLocationFile(exeDir));

        var blockingExePath = Path.Combine(workspace.Root, "blocked-exe-dir-file");
        File.WriteAllText(blockingExePath, "occupy path", Encoding.UTF8);
        var acceptedReadOnlyExe = AgpPaths.TryAcceptUserSelectedAgpRoot(
            manualDir,
            mediumIntegrity,
            executableDirectoryOverride: blockingExePath);
        Assert.True(acceptedReadOnlyExe.Success);
        Assert.False(acceptedReadOnlyExe.LocationFileSaved);
        Assert.NotNull(acceptedReadOnlyExe.LocationFileSaveError);
    }

    [Fact]
    public void AgpPaths_TryConfigureProcessTempDirectory_RedirectsTempAndTmpToDataRootTmpOnWindowsOnly()
    {
        using var workspace = new TempWorkspace("ffr-agp-process-temp");
        var previousTemp = Environment.GetEnvironmentVariable("TEMP");
        var previousTmp = Environment.GetEnvironmentVariable("TMP");

        try
        {
            var resolution = AgpPaths.ResolveWritableAgpRoot(
                agpHomeEnvOverride: workspace.AgpHomeDir,
                isWindowsOverride: true);
            Assert.True(resolution.Success);

            var expectedTmpDir = AgpPaths.GetTempDirectoryForAgpRoot(workspace.AgpHomeDir);

            // Windows: creates <dataRoot>\tmp and redirects both TEMP and TMP to it.
            var winConfigured = AgpPaths.TryConfigureProcessTempDirectory(
                resolution,
                out var configuredDir,
                out var winError,
                isWindowsOverride: true);
            Assert.True(winConfigured);
            Assert.Null(winError);
            Assert.Equal(expectedTmpDir, configuredDir);
            Assert.True(Directory.Exists(expectedTmpDir));
            Assert.Equal(expectedTmpDir, Environment.GetEnvironmentVariable("TEMP"));
            Assert.Equal(expectedTmpDir, Environment.GetEnvironmentVariable("TMP"));

            // Non-Windows: creates tmp directory but leaves TEMP/TMP untouched.
            var sentinelTemp = Path.Combine(workspace.Root, "non-windows-sentinel");
            Environment.SetEnvironmentVariable("TEMP", sentinelTemp);
            Environment.SetEnvironmentVariable("TMP", sentinelTemp);
            var unixConfigured = AgpPaths.TryConfigureProcessTempDirectory(
                resolution,
                out var unixDir,
                out var unixError,
                isWindowsOverride: false);
            Assert.True(unixConfigured);
            Assert.Null(unixError);
            Assert.Equal(expectedTmpDir, unixDir);
            Assert.Equal(sentinelTemp, Environment.GetEnvironmentVariable("TEMP"));
            Assert.Equal(sentinelTemp, Environment.GetEnvironmentVariable("TMP"));

            // Failed resolution returns false without touching TEMP/TMP.
            var failedResolution = AgpPaths.ResolveWritableAgpRoot(
                agpHomeEnvOverride: Path.Combine(workspace.Root, "UnwritableEnv"),
                isWindowsOverride: true,
                writeProbeOverride: _ => new AgpWriteProbeResult(false, "拒绝访问"));
            Assert.False(failedResolution.Success);

            var failedConfigured = AgpPaths.TryConfigureProcessTempDirectory(
                failedResolution,
                out var failedDir,
                out var failedError,
                isWindowsOverride: true);
            Assert.False(failedConfigured);
            Assert.Null(failedDir);
            Assert.False(string.IsNullOrWhiteSpace(failedError));
            Assert.Equal(sentinelTemp, Environment.GetEnvironmentVariable("TEMP"));
            Assert.Equal(sentinelTemp, Environment.GetEnvironmentVariable("TMP"));
        }
        finally
        {
            Environment.SetEnvironmentVariable("TEMP", previousTemp);
            Environment.SetEnvironmentVariable("TMP", previousTmp);
        }
    }

    private static (string ExecutablePath, string[] PrefixArgs) ResolveDirectFfrExportInvocation()
    {
        var apphostName = OperatingSystem.IsWindows() ? "FfrExport.exe" : "FfrExport";
        var apphostPath = Path.Combine(AppContext.BaseDirectory, apphostName);
        if (File.Exists(apphostPath)
            && (OperatingSystem.IsWindows() || !string.IsNullOrWhiteSpace(Environment.GetEnvironmentVariable("DOTNET_ROOT"))))
        {
            return (apphostPath, []);
        }

        var ffrExportDll = Path.Combine(AppContext.BaseDirectory, "FfrExport.dll");
        Assert.True(File.Exists(ffrExportDll), $"找不到 FfrExport.dll：{ffrExportDll}");
        return (ResolveDotnetForTest(), ["exec", ffrExportDll]);
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

    private static string ResolveDotnetForTest()
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

        var repoDotnet = Path.Combine(
            TestHarness.FindRepoRoot(),
            ".deps",
            "dotnet",
            OperatingSystem.IsWindows() ? "dotnet.exe" : "dotnet");
        return File.Exists(repoDotnet) ? repoDotnet : "dotnet";
    }
}
