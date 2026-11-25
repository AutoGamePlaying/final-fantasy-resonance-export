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

        // 1. None found -> Chinese UE4SS guidance
        var emptyResult = MappingDiscovery.DiscoverUsmap(gameDir);
        Assert.Null(emptyResult.Selected);
        Assert.Empty(emptyResult.Candidates);
        Assert.Contains("UE4SS", emptyResult.NotFoundGuidance, StringComparison.Ordinal);

        // 2. Only Win64 has .usmap -> fallback finds newest in Win64
        var win64Old = Path.Combine(win64Dir, "old.usmap");
        var win64New = Path.Combine(win64Dir, "new.usmap");
        File.WriteAllBytes(win64Old, [1]);
        File.WriteAllBytes(win64New, [2]);
        File.SetLastWriteTimeUtc(win64Old, new DateTime(2026, 11, 20, 8, 0, 0, DateTimeKind.Utc));
        File.SetLastWriteTimeUtc(win64New, new DateTime(2026, 11, 24, 12, 30, 0, DateTimeKind.Utc));

        var win64Result = MappingDiscovery.DiscoverUsmap(gameDir);
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

        var ue4ssResult = MappingDiscovery.DiscoverUsmap(gameDir);
        Assert.NotNull(ue4ssResult.Selected);
        Assert.Equal("dump-latest.usmap", ue4ssResult.Selected.FileName);
        Assert.Equal("ue4ss", ue4ssResult.Selected.SourceLabel);
        Assert.Equal(2, ue4ssResult.Candidates.Count);
        Assert.Null(ue4ssResult.NotFoundGuidance);

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

        var fakeDocs = Path.Combine(workspace.Root, "Documents");
        var defaultRoot = GuiPaths.ResolveDefaultOutputRoot(documentsDirectoryOverride: fakeDocs);
        Assert.Equal(Path.Combine(fakeDocs, "FF-Export"), defaultRoot);

        var defaultOut = GuiPaths.BuildDefaultOutputDirectory(
            outputRootOverride: null,
            gameCode: "ffr",
            steamBuildId: "24250053",
            documentsDirectoryOverride: fakeDocs);
        Assert.Equal(Path.Combine(fakeDocs, "FF-Export", "ffr-24250053"), defaultOut);

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

        // ProcessStartInfo forces UTF-8 for stdout and stderr with no console window.
        var startInfo = BuildCliCommand.CreateStartInfo("/app/ffr-export", argsNoJmap);
        Assert.True(startInfo.RedirectStandardOutput);
        Assert.True(startInfo.RedirectStandardError);
        Assert.Equal(Encoding.UTF8.WebName, startInfo.StandardOutputEncoding?.WebName);
        Assert.Equal(Encoding.UTF8.WebName, startInfo.StandardErrorEncoding?.WebName);
        Assert.False(startInfo.UseShellExecute);
        Assert.True(startInfo.CreateNoWindow);
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
    public void GuiSettingsStore_PersistsToJsonAndHandlesMissingOrCorruptFiles()
    {
        using var workspace = new TempWorkspace("ffr-gui-settings");

        var appDataDir = Path.Combine(workspace.Root, "AppData");
        var settingsPath = GuiSettingsStore.GetDefaultSettingsPath(appDataDir);
        Assert.Equal(Path.Combine(appDataDir, "ffr-export", "settings.json"), settingsPath);

        // Missing file returns defaults (IncludeJmap == false).
        var initial = GuiSettingsStore.Load(settingsPath);
        Assert.Null(initial.OutputRootDirectory);
        Assert.False(initial.IncludeJmap);

        // Save and reload roundtrip.
        var customRoot = Path.Combine(workspace.Root, "My Exports");
        GuiSettingsStore.Save(new GuiSettings(customRoot, true), settingsPath);
        Assert.True(File.Exists(settingsPath));

        var reloaded = GuiSettingsStore.Load(settingsPath);
        Assert.Equal(customRoot, reloaded.OutputRootDirectory);
        Assert.True(reloaded.IncludeJmap);

        // Corrupt JSON degrades safely to defaults.
        File.WriteAllText(settingsPath, "{ invalid json", Encoding.UTF8);
        var fallback = GuiSettingsStore.Load(settingsPath);
        Assert.Null(fallback.OutputRootDirectory);
        Assert.False(fallback.IncludeJmap);
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

        string[] forbiddenTokens = ["OodleHelper", "DownloadOodle", "HttpClient", "WebClient"];
        var linuxRootRegex = new Regex(@"/root/", RegexOptions.CultureInvariant);
        var windowsDriveRegex = new Regex(@"[A-Za-z]:[\\/]", RegexOptions.CultureInvariant);

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
