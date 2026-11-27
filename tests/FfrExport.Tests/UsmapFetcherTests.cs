using System.Diagnostics;
using System.IO.Compression;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using FfrExport.Gui.Core;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class UsmapFetcherTests
{
    [Fact]
    public void AgpPaths_ResolvesPriorityOrderAndUnifiedSubdirectories()
    {
        using var workspace = new TempWorkspace("ffr-agp-paths");

        var envHome = Path.Combine(workspace.Root, "env-agp");
        var winLocal = Path.Combine(workspace.Root, "WinLocalAppData");
        var xdgData = Path.Combine(workspace.Root, "XdgData");
        var unixHome = Path.Combine(workspace.Root, "UnixHome");

        // 1. AGP_HOME takes highest priority and is used directly as the agp layer.
        Assert.Equal(
            envHome,
            AgpPaths.ResolveAgpHome(
                agpHomeEnvOverride: envHome,
                isWindowsOverride: true,
                localAppDataOverride: winLocal,
                xdgDataHomeOverride: xdgData,
                userProfileOverride: unixHome));

        // 2. Windows falls back to %LOCALAPPDATA%\agp when AGP_HOME is unset.
        Assert.Equal(
            Path.Combine(winLocal, "agp"),
            AgpPaths.ResolveAgpHome(
                agpHomeEnvOverride: string.Empty,
                isWindowsOverride: true,
                localAppDataOverride: winLocal));

        // 3. Linux/macOS uses $XDG_DATA_HOME/agp, then ~/.local/share/agp.
        Assert.Equal(
            Path.Combine(xdgData, "agp"),
            AgpPaths.ResolveAgpHome(
                agpHomeEnvOverride: string.Empty,
                isWindowsOverride: false,
                xdgDataHomeOverride: xdgData,
                userProfileOverride: unixHome));

        Assert.Equal(
            Path.Combine(unixHome, ".local", "share", "agp"),
            AgpPaths.ResolveAgpHome(
                agpHomeEnvOverride: string.Empty,
                isWindowsOverride: false,
                xdgDataHomeOverride: string.Empty,
                userProfileOverride: unixHome));

        // 4. Unified subdirectories under <agp>/ffr-export/.
        Assert.Equal(
            Path.Combine(envHome, "ffr-export", "output", "ffr-24250053"),
            AgpPaths.GetDefaultOutputDirectory("ffr", "24250053", agpHomeEnvOverride: envHome));
        Assert.Equal(
            Path.Combine(envHome, "ffr-export", "usmap"),
            AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: envHome));
        Assert.Equal(
            Path.Combine(envHome, "ffr-export", "downloads"),
            AgpPaths.GetDownloadsDirectory(agpHomeEnvOverride: envHome));
        Assert.Equal(
            Path.Combine(envHome, "ffr-export", "logs"),
            AgpPaths.GetLogsDirectory(agpHomeEnvOverride: envHome));
        Assert.Equal(
            Path.Combine(envHome, "ffr-export", "settings.json"),
            AgpPaths.GetSettingsFilePath(agpHomeEnvOverride: envHome));
    }

    [Fact]
    public async Task Ue4ssArchiveVerification_RejectsWrongHashOrSizeAndReusesValidCache()
    {
        using var langScope = Localization.UseLanguageScope("zh-Hans");
        using var workspace = new TempWorkspace("ffr-ue4ss-verify");

        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "synthetic.zip"));
        var zipBytes = await File.ReadAllBytesAsync(zipPath);
        var validSha = Convert.ToHexStringLower(SHA256.HashData(zipBytes));

        // Valid size and SHA-256 pass.
        Assert.True(HttpsUe4ssPackageDownloader.TryVerifyArchiveFile(zipPath, zipBytes.Length, validSha, out var okReason));
        Assert.Null(okReason);

        // Wrong SHA-256 is rejected.
        var badHashEx = Assert.Throws<InvalidDataException>(() =>
            HttpsUe4ssPackageDownloader.VerifyArchiveFileOrThrow(
                zipPath,
                zipBytes.Length,
                new string('0', 64),
                "synthetic-source"));
        Assert.Contains("SHA-256 校验不一致", badHashEx.Message, StringComparison.Ordinal);

        // Wrong size is rejected.
        var badSizeEx = Assert.Throws<InvalidDataException>(() =>
            HttpsUe4ssPackageDownloader.VerifyArchiveFileOrThrow(
                zipPath,
                zipBytes.Length + 1,
                validSha,
                "synthetic-source"));
        Assert.Contains("文件大小不匹配", badSizeEx.Message, StringComparison.Ordinal);

        // When a valid archive is already cached in downloads/, HttpsUe4ssPackageDownloader reuses it without network access.
        var downloadsDir = AgpPaths.GetDownloadsDirectory(agpHomeEnvOverride: workspace.AgpHomeDir);
        Directory.CreateDirectory(downloadsDir);
        var cachedFile = Path.Combine(downloadsDir, "UE4SS_v4.0.0.zip");
        File.Copy(zipPath, cachedFile, overwrite: true);

        var downloader = new HttpsUe4ssPackageDownloader(
            downloadUrl: "https://invalid.example.test/must-not-connect.zip",
            expectedSize: zipBytes.Length,
            expectedSha256: validSha);
        var logOut = new StringWriter();
        var resolved = await downloader.EnsureVerifiedArchiveAsync(downloadsDir, logOut, CancellationToken.None);
        Assert.Equal(cachedFile, resolved);
        Assert.Contains("已命中本地下载缓存", logOut.ToString(), StringComparison.Ordinal);
    }

    [Fact]
    public void Ue4ssArchiveInstaller_ExtractsPatchesSettingsRollsBackAndUninstallsOnlyMarkedFiles()
    {
        using var langScope = Localization.UseLanguageScope("zh-Hans");
        using var workspace = new TempWorkspace("ffr-ue4ss-install");

        var gameDir = CreateSyntheticGameDirectory(workspace.Root, "24250053");
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDir);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir);
        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "UE4SS_v4.0.0.zip"));
        var zipSha = GameVersionDetector.ComputeFileSha256Lower(zipPath);

        // 1. Zip-slip entry is rejected before touching Win64.
        var evilZip = Path.Combine(workspace.Root, "evil.zip");
        using (var archive = ZipFile.Open(evilZip, ZipArchiveMode.Create))
        {
            WriteZipEntry(archive, "dwmapi.dll", "dll");
            WriteZipEntry(archive, "ue4ss/UE4SS.dll", "dll");
            WriteZipEntry(archive, "../escape.dll", "evil");
        }

        Assert.Throws<InvalidDataException>(() =>
            Ue4ssArchiveInstaller.InstallFromVerifiedZip(evilZip, gameDir, zipSha));
        Assert.False(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));

        // 2. Halfway extraction failure rolls back every written file and created directory.
        var failEx = Assert.Throws<IOException>(() =>
            Ue4ssArchiveInstaller.InstallFromVerifiedZip(
                zipPath,
                gameDir,
                zipSha,
                beforeFileWriteHook: rel =>
                {
                    if (rel.EndsWith("UE4SS-settings.ini", StringComparison.OrdinalIgnoreCase))
                    {
                        throw new IOException("模拟磁盘写入中断");
                    }
                }));
        Assert.Contains("模拟磁盘写入中断", failEx.Message, StringComparison.Ordinal);
        Assert.False(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.False(Directory.Exists(ue4ssDir));

        // 3. Clean install extracts all files, preserves LICENSE, patches UE4SS-settings.ini, writes marker.
        var marker = Ue4ssArchiveInstaller.InstallFromVerifiedZip(zipPath, gameDir, zipSha);
        Assert.Equal("v4.0.0", marker.Ue4ssVersion);
        Assert.True(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.True(File.Exists(Path.Combine(ue4ssDir, "UE4SS.dll")));
        Assert.True(File.Exists(Path.Combine(ue4ssDir, "LICENSE")));
        Assert.True(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir));

        var patchedIni = File.ReadAllText(Path.Combine(ue4ssDir, "UE4SS-settings.ini"), Encoding.UTF8);
        Assert.Contains("ConsoleEnabled = 0", patchedIni, StringComparison.Ordinal);
        Assert.Contains("GuiConsoleEnabled = 0", patchedIni, StringComparison.Ordinal);
        Assert.Contains("GuiConsoleVisible = 0", patchedIni, StringComparison.Ordinal);
        Assert.Contains("EnableHotReloadSystem = 0", patchedIni, StringComparison.Ordinal);
        Assert.Contains("bUseUObjectArrayCache = false", patchedIni, StringComparison.Ordinal);
        Assert.Contains("MajorVersion = 5", patchedIni, StringComparison.Ordinal);
        Assert.Contains("MinorVersion = 6", patchedIni, StringComparison.Ordinal);

        var inspection = Ue4ssRuntimeSupport.InspectUe4ssState(gameDir);
        Assert.Equal(Ue4ssInstallState.InstalledByFfrExport, inspection.State);

        // 4. Uninstall is refused while the game is running.
        var runningLauncher = new FakeGameLauncher { Running = true };
        var runningEx = Assert.Throws<InvalidOperationException>(() =>
            Ue4ssArchiveInstaller.Uninstall(gameDir, runningLauncher, new StringWriter()));
        Assert.Contains("正在运行", runningEx.Message, StringComparison.Ordinal);

        // 5. Uninstall deletes files recorded in the marker AND cleans our remnants (Mods/FfrUsmapDump, .done, .tmp)
        //    while preserving user/runtime files (UE4SS.log, UserCustomMod) and archived agp/ffr-export/usmap/.
        var runtimeLogPath = Path.Combine(ue4ssDir, "UE4SS.log");
        var userModFile = Path.Combine(ue4ssDir, "Mods", "UserCustomMod", "Scripts", "main.lua");
        var dumpModScript = Path.Combine(ue4ssDir, "Mods", "FfrUsmapDump", "Scripts", "main.lua");
        var doneFile = Path.Combine(ue4ssDir, "ffr-export-usmap.done");
        var tmpFile = Path.Combine(ue4ssDir, "ffr-export-usmap.done.tmp");
        Directory.CreateDirectory(Path.GetDirectoryName(userModFile)!);
        Directory.CreateDirectory(Path.GetDirectoryName(dumpModScript)!);
        File.WriteAllText(runtimeLogPath, "runtime log", Encoding.UTF8);
        File.WriteAllText(userModFile, "-- user mod\n", Encoding.UTF8);
        File.WriteAllText(dumpModScript, "-- dump mod\n", Encoding.UTF8);
        File.WriteAllText(doneFile, "{}", Encoding.UTF8);
        File.WriteAllText(tmpFile, "partial", Encoding.UTF8);

        var archivedUsmapDir = AgpPaths.GetUsmapDirectoryForAgpRoot(workspace.AgpHomeDir);
        Directory.CreateDirectory(archivedUsmapDir);
        var archivedUsmap = Path.Combine(archivedUsmapDir, "FFRS-24250053-deadbeef.usmap");
        File.WriteAllBytes(archivedUsmap, SyntheticFixtureBuilder.BuildUsmap());

        var ownPlan = Ue4ssRuntimeSupport.InspectCleanupPlan(gameDir);
        Assert.True(ownPlan.HasOurManagedInstall);
        Assert.True(ownPlan.HasRemnants);
        var ownPreview = Ue4ssRuntimeSupport.FormatCleanupPreviewText(ownPlan, includeForeign: false, "<backup>");
        Assert.Contains("dwmapi.dll", ownPreview, StringComparison.Ordinal);
        Assert.Contains("FfrUsmapDump", ownPreview, StringComparison.Ordinal);

        var stoppedLauncher = new FakeGameLauncher { Running = false };
        var uninstallOut = new StringWriter();
        var ownRes = Ue4ssArchiveInstaller.Uninstall(
            gameDir,
            stoppedLauncher,
            uninstallOut,
            agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(Ue4ssUninstallOutcome.RemovedOwnInstall, ownRes.Outcome);
        Assert.True(ownRes.RemovedFiles >= 8);
        Assert.Equal(2, ownRes.RemainingFilesInUe4ss);
        Assert.Null(ownRes.BackupDirectory);
        Assert.False(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.False(File.Exists(Path.Combine(ue4ssDir, "UE4SS.dll")));
        Assert.False(File.Exists(Path.Combine(ue4ssDir, "UE4SS-settings.ini")));
        Assert.False(File.Exists(Ue4ssRuntimeSupport.GetInstallMarkerPath(gameDir)));
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDir)));
        Assert.False(File.Exists(doneFile));
        Assert.False(File.Exists(tmpFile));
        Assert.False(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir));
        Assert.True(File.Exists(runtimeLogPath));
        Assert.True(File.Exists(userModFile));
        Assert.True(File.Exists(archivedUsmap));

        // 6. Foreign UE4SS (no marker):
        //    6a) Without --include-foreign: only cleans our remnants; leaves dwmapi.dll & ue4ss/ untouched.
        File.WriteAllText(Path.Combine(win64Dir, "dwmapi.dll"), "user-dwmapi", Encoding.UTF8);
        File.WriteAllText(Path.Combine(ue4ssDir, "UE4SS.dll"), "user-ue4ss", Encoding.UTF8);
        File.WriteAllText(Path.Combine(ue4ssDir, "UE4SS-settings.ini"), "[Debug]\nConsoleEnabled = 1\n", Encoding.UTF8);
        Directory.CreateDirectory(Path.GetDirectoryName(dumpModScript)!);
        File.WriteAllText(dumpModScript, "-- dump mod\n", Encoding.UTF8);
        File.WriteAllText(doneFile, "{}", Encoding.UTF8);
        File.WriteAllText(tmpFile, "partial", Encoding.UTF8);
        Assert.Equal(Ue4ssInstallState.InstalledByUser, Ue4ssRuntimeSupport.InspectUe4ssState(gameDir).State);
        Assert.False(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir));

        var foreignPlan = Ue4ssRuntimeSupport.InspectCleanupPlan(gameDir);
        Assert.True(foreignPlan.IsForeignUe4ss);
        Assert.True(foreignPlan.HasRemnants);
        Assert.Contains("dwmapi.dll", foreignPlan.ForeignPathsToMove);
        Assert.Contains("ue4ss/", foreignPlan.ForeignPathsToMove);

        var foreignDefaultOut = new StringWriter();
        var foreignDefaultRes = Ue4ssArchiveInstaller.Uninstall(
            gameDir,
            stoppedLauncher,
            foreignDefaultOut,
            includeForeign: false,
            agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(Ue4ssUninstallOutcome.RemovedRemnantsOnly, foreignDefaultRes.Outcome);
        Assert.Null(foreignDefaultRes.BackupDirectory);
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDir)));
        Assert.False(File.Exists(doneFile));
        Assert.False(File.Exists(tmpFile));
        Assert.True(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.True(File.Exists(Path.Combine(ue4ssDir, "UE4SS.dll")));
        Assert.True(File.Exists(userModFile));
        Assert.Contains("--include-foreign", foreignDefaultOut.ToString(), StringComparison.Ordinal);

        //    6b) With --include-foreign and declining interactive prompt -> cancelled, nothing moved.
        var declineOut = new StringWriter();
        var declineRes = Ue4ssArchiveInstaller.Uninstall(
            gameDir,
            stoppedLauncher,
            declineOut,
            includeForeign: true,
            yes: false,
            input: new StringReader("n\n"),
            agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(Ue4ssUninstallOutcome.Cancelled, declineRes.Outcome);
        Assert.True(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.True(File.Exists(Path.Combine(ue4ssDir, "UE4SS.dll")));

        //    6c) With --include-foreign --yes -> removes our remnants and MOVES dwmapi.dll + ue4ss/ into backup dir.
        Directory.CreateDirectory(Path.GetDirectoryName(dumpModScript)!);
        File.WriteAllText(dumpModScript, "-- temp remnant\n", Encoding.UTF8);
        var fixedBackupTime = new DateTimeOffset(2026, 11, 26, 14, 30, 45, TimeSpan.FromHours(9));
        var foreignBackupOut = new StringWriter();
        var backupRes = Ue4ssArchiveInstaller.Uninstall(
            gameDir,
            stoppedLauncher,
            foreignBackupOut,
            includeForeign: true,
            yes: true,
            agpHomeOverride: workspace.AgpHomeDir,
            nowOverride: () => fixedBackupTime);

        Assert.Equal(Ue4ssUninstallOutcome.BackedUpForeignInstall, backupRes.Outcome);
        var expectedBackupDir = Path.Combine(
            workspace.AgpHomeDir,
            "ffr-export",
            "ue4ss-backup",
            "20261126-143045");
        Assert.Equal(expectedBackupDir, backupRes.BackupDirectory);

        // Game directory is now clean of UE4SS.
        Assert.False(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.False(Directory.Exists(ue4ssDir));

        // Backup directory preserves dwmapi.dll, ue4ss/UE4SS.dll, settings, user mods, and excludes cleaned dump mod.
        Assert.Equal("user-dwmapi", File.ReadAllText(Path.Combine(expectedBackupDir, "dwmapi.dll"), Encoding.UTF8));
        Assert.Equal("user-ue4ss", File.ReadAllText(Path.Combine(expectedBackupDir, "ue4ss", "UE4SS.dll"), Encoding.UTF8));
        Assert.Equal(
            "-- user mod\n",
            File.ReadAllText(Path.Combine(expectedBackupDir, "ue4ss", "Mods", "UserCustomMod", "Scripts", "main.lua"), Encoding.UTF8));
        Assert.False(Directory.Exists(Path.Combine(expectedBackupDir, "ue4ss", "Mods", "FfrUsmapDump")));
        Assert.Contains("恢复方法", foreignBackupOut.ToString(), StringComparison.Ordinal);
        Assert.Contains(expectedBackupDir, foreignBackupOut.ToString(), StringComparison.Ordinal);

        // Archived .usmap in agp/ffr-export/usmap/ remains completely intact.
        Assert.True(File.Exists(archivedUsmap));

        //    6d) When nothing is installed -> reports "未发现 UE4SS".
        var notFoundOut = new StringWriter();
        var notFoundRes = Ue4ssArchiveInstaller.Uninstall(
            gameDir,
            stoppedLauncher,
            notFoundOut,
            agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(Ue4ssUninstallOutcome.NotFound, notFoundRes.Outcome);
        Assert.Contains("未发现 UE4SS", notFoundOut.ToString(), StringComparison.Ordinal);

        // 7. Tampered marker with path traversal is rejected before deleting anything.
        Directory.CreateDirectory(ue4ssDir);
        var tamperedMarkerPath = Ue4ssRuntimeSupport.GetInstallMarkerPath(gameDir);
        File.WriteAllText(
            tamperedMarkerPath,
            JsonConvert.SerializeObject(new
            {
                format = 1,
                installed_by = "ffr-export",
                ue4ss_version = "v4.0.0",
                source_url = "https://example.invalid",
                archive_sha256 = zipSha,
                installed_at = "2026-11-26T00:00:00Z",
                files = new[] { "../outside.txt" },
                directories = Array.Empty<string>(),
            }),
            Encoding.UTF8);
        Assert.Throws<InvalidDataException>(() =>
            Ue4ssArchiveInstaller.Uninstall(gameDir, stoppedLauncher, new StringWriter()));
    }

    [Fact]
    public void Ue4ssModManager_EmbeddedLuaEnabledTxtAndModsTxtPreservation()
    {
        using var workspace = new TempWorkspace("ffr-ue4ss-mod");

        // 1. Embedded Lua resource content checks.
        var lua = Ue4ssModManager.LoadEmbeddedMainLua();
        Assert.Contains("DumpUSMAP", lua, StringComparison.Ordinal);
        Assert.Contains("pcall", lua, StringComparison.Ordinal);
        Assert.Contains("ffr-export-usmap.done", lua, StringComparison.Ordinal);
        Assert.Contains("usmapFile", lua, StringComparison.Ordinal);
        Assert.Contains("ExecuteInGameThread", lua, StringComparison.Ordinal);
        Assert.Contains("LoopAsync", lua, StringComparison.Ordinal);
        Assert.DoesNotContain("os.execute", lua, StringComparison.Ordinal);

        // 2. Deployment uses enabled.txt and leaves mods.txt untouched byte-for-byte.
        var gameDir = CreateSyntheticGameDirectory(workspace.Root, "24250053");
        var modsDir = Path.Combine(Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir), "Mods");
        Directory.CreateDirectory(modsDir);
        var modsTxtPath = Path.Combine(modsDir, "mods.txt");
        const string originalModsTxt = "; User comments kept verbatim\r\nMyCustomMod : 1\r\n\r\n; Built-in keybinds\r\nKeybinds : 0\r\n";
        File.WriteAllText(modsTxtPath, originalModsTxt, new UTF8Encoding(false));

        var deployedDir = Ue4ssModManager.DeployTemporaryDumpMod(gameDir);
        Assert.True(File.Exists(Path.Combine(deployedDir, "Scripts", "main.lua")));
        Assert.True(File.Exists(Path.Combine(deployedDir, "enabled.txt")));
        Assert.Equal(originalModsTxt, File.ReadAllText(modsTxtPath, new UTF8Encoding(false)));

        Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(gameDir);
        Assert.False(Directory.Exists(deployedDir));
        Assert.Equal(originalModsTxt, File.ReadAllText(modsTxtPath, new UTF8Encoding(false)));

        // 3. Fallback mods.txt helper appends one line while preserving all existing lines, comments, and CRLF.
        var appended = Ue4ssModManager.AppendModEntryPreservingModsTxt(originalModsTxt, "FfrUsmapDump", enabled: true);
        Assert.Equal(originalModsTxt + "FfrUsmapDump : 1\r\n", appended);

        var disabled = Ue4ssModManager.AppendModEntryPreservingModsTxt(appended, "FfrUsmapDump", enabled: false);
        Assert.Equal(originalModsTxt + "FfrUsmapDump : 0\r\n", disabled);
    }

    [Fact]
    public async Task UsmapFetcher_EndToEndFlow_FreshInstallUserUe4ssTimeoutModErrorAndGuards()
    {
        using var langScope = Localization.UseLanguageScope("zh-Hans");
        using var workspace = new TempWorkspace("ffr-fetch-e2e");

        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "UE4SS_v4.0.0.zip"));
        var syntheticUsmapBytes = SyntheticFixtureBuilder.BuildUsmapV0(
            ["EDemo", "Zero", "One", "FDemoRow", "Id"],
            [new("EDemo", ["Zero", "One"])],
            [new("FDemoRow", null, 1, [new(0, 1, "Id", 2)])]);
        var tempUsmapForHash = Path.Combine(workspace.Root, "ref.usmap");
        await File.WriteAllBytesAsync(tempUsmapForHash, syntheticUsmapBytes);
        var expectedHashes = GameVersionDetector.ComputeUsmapHashes(tempUsmapForHash);

        // Prepare a temporary known-builds.json matching our synthetic .usmap.
        var knownBuildsDir = Path.Combine(workspace.Root, "known-builds-bin");
        Directory.CreateDirectory(knownBuildsDir);
        await File.WriteAllTextAsync(
            Path.Combine(knownBuildsDir, "known-builds.json"),
            JsonConvert.SerializeObject(new[]
            {
                new
                {
                    registered_at = "2026-11-26",
                    note = "synthetic usmap test",
                    game_version = new
                    {
                        steam = new { app_id = 3259780, build_id = "24250053" },
                        exe = new { relative_path = GameVersionDetector.RelativeExeSubpath, size = 100L },
                        pak_fingerprint = "0000",
                        pak_files = Array.Empty<object>(),
                    },
                    inputs = new
                    {
                        usmap = new
                        {
                            file_name = "FFRS-5.6.1-0+UE5-30afe5ff.usmap",
                            size = expectedHashes.Size,
                            file_sha256 = expectedHashes.FileSha256,
                            content_sha256 = expectedHashes.ContentSha256,
                            type_count = expectedHashes.TypeCount,
                            enum_count = expectedHashes.EnumCount,
                        },
                    },
                    database = new
                    {
                        file_name = "ffr.sqlite",
                        size = 1024L,
                        file_sha256 = new string('a', 64),
                        content_sha256 = new string('b', 64),
                        tables = Array.Empty<object>(),
                    },
                },
            }),
            Encoding.UTF8);

        // 1. Fresh install happy path (--yes): installs UE4SS, launches game, ignores stale .usmap,
        //    archives fresh .usmap to <AGP_HOME>/ffr-export/usmap/, compares with known-builds, removes mod, closes game.
        var gameDir1 = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameFresh");
        var downloader1 = new FakeUe4ssDownloader(zipPath);
        var launcher1 = new FakeGameLauncher
        {
            OnLaunch = uri =>
            {
                Assert.Equal(Ue4ssRuntimeSupport.SteamRunGameUri, uri);
                var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir1);
                // Ensure the temporary mod was deployed before launching.
                Assert.True(File.Exists(Path.Combine(ue4ssDir, "Mods", "FfrUsmapDump", "enabled.txt")));

                var dumpedPath = Path.Combine(ue4ssDir, "FFRS-5.6.1-0+UE5-30afe5ff.usmap");
                File.WriteAllBytes(dumpedPath, syntheticUsmapBytes);
                File.SetLastWriteTimeUtc(dumpedPath, DateTime.UtcNow.AddSeconds(5));

                var donePath = Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDir1);
                File.WriteAllText(
                    donePath,
                    """{"ok":true,"usmapFile":"FFRS-5.6.1-0+UE5-30afe5ff.usmap","timestamp":"2026-11-26T12:00:00Z"}""",
                    Encoding.UTF8);
            },
        };

        var out1 = new StringWriter();
        var err1 = new StringWriter();
        var (exit1, res1) = await UsmapFetcher.FetchCoreAsync(
            new FetchUsmapOptions(gameDir1, TimeoutSeconds: 10, Yes: true, ProgressJson: true),
            new StringReader(string.Empty),
            out1,
            err1,
            downloader1,
            launcher1,
            agpHomeOverride: workspace.AgpHomeDir,
            isWindowsOverride: true,
            pollIntervalOverride: TimeSpan.FromMilliseconds(10),
            progressIntervalOverride: TimeSpan.FromMilliseconds(50),
            gracefulCloseWaitOverride: TimeSpan.FromMilliseconds(50),
            knownBuildsBaseDirOverride: knownBuildsDir);

        Assert.Equal(0, exit1);
        Assert.NotNull(res1);
        Assert.True(res1.InstalledUe4ss);
        Assert.Equal(1, downloader1.CallCount);
        Assert.Equal(1, launcher1.LaunchCount);
        Assert.Equal(1, launcher1.CloseCount);
        Assert.Equal(UsmapComparisonStatus.ExactMatch, res1.KnownBuildComparison.Status);
        Assert.Equal(expectedHashes.FileSha256, res1.UsmapHashes.FileSha256);
        Assert.Equal(expectedHashes.ContentSha256, res1.UsmapHashes.ContentSha256);
        Assert.True(File.Exists(res1.ArchivedUsmapPath));
        Assert.StartsWith(
            AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: workspace.AgpHomeDir),
            res1.ArchivedUsmapPath,
            StringComparison.Ordinal);
        Assert.Contains("24250053", Path.GetFileName(res1.ArchivedUsmapPath), StringComparison.Ordinal);
        Assert.Contains(expectedHashes.FileSha256[..8], Path.GetFileName(res1.ArchivedUsmapPath), StringComparison.Ordinal);

        // Temporary mod & done marker removed; UE4SS and marker retained.
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDir1)));
        Assert.False(File.Exists(Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDir1)));
        Assert.True(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir1));
        Assert.True(File.Exists(Path.Combine(Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir1), "LICENSE")));

        // Verify progress lines and BuildCliCommand parser compatibility.
        var out1Lines = out1.ToString().Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries);
        var parsedSteps = new List<StepProgress>();
        string? parsedSavedPath = null;
        foreach (var line in out1Lines)
        {
            if (BuildCliCommand.TryParseStepProgress(line, out var step))
            {
                parsedSteps.Add(step);
            }

            if (BuildCliCommand.TryParseFetchedUsmapPath(line, out var saved))
            {
                parsedSavedPath = saved;
            }
        }

        Assert.Equal([1, 2, 3, 4, 5, 6], parsedSteps.Select(s => s.CurrentStep).ToArray());
        Assert.All(parsedSteps, s => Assert.Equal(6, s.TotalSteps));
        Assert.Equal(res1.ArchivedUsmapPath, parsedSavedPath);

        // 2. Pre-existing user UE4SS (no marker): never calls downloader, never edits UE4SS-settings.ini or mods.txt, never writes marker.
        var gameDirUser = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameUserUe4ss");
        var userWin64 = Ue4ssRuntimeSupport.GetWin64Directory(gameDirUser);
        var userUe4ss = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDirUser);
        Directory.CreateDirectory(Path.Combine(userUe4ss, "Mods"));
        File.WriteAllText(Path.Combine(userWin64, "dwmapi.dll"), "user-proxy");
        File.WriteAllText(Path.Combine(userUe4ss, "UE4SS.dll"), "user-core");
        const string userSettingsIni = "[Debug]\nGuiConsoleEnabled = 1\nConsoleEnabled = 1\n";
        const string userModsTxt = "UserCoolMod : 1\n";
        File.WriteAllText(Path.Combine(userUe4ss, "UE4SS-settings.ini"), userSettingsIni);
        File.WriteAllText(Path.Combine(userUe4ss, "Mods", "mods.txt"), userModsTxt);

        var downloaderUser = new FakeUe4ssDownloader(zipPath);
        var launcherUser = new FakeGameLauncher
        {
            OnLaunch = _ =>
            {
                var dumped = Path.Combine(userUe4ss, "UserDump.usmap");
                File.WriteAllBytes(dumped, syntheticUsmapBytes);
                File.SetLastWriteTimeUtc(dumped, DateTime.UtcNow.AddSeconds(5));
                File.WriteAllText(
                    Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDirUser),
                    """{"ok":true,"usmapFile":"UserDump.usmap","timestamp":"2026-11-26T12:00:00Z"}""");
            },
        };

        var (exitUser, resUser) = await UsmapFetcher.FetchCoreAsync(
            new FetchUsmapOptions(gameDirUser, TimeoutSeconds: 10, Yes: true),
            new StringReader(string.Empty),
            new StringWriter(),
            new StringWriter(),
            downloaderUser,
            launcherUser,
            agpHomeOverride: workspace.AgpHomeDir,
            isWindowsOverride: true,
            pollIntervalOverride: TimeSpan.FromMilliseconds(10),
            gracefulCloseWaitOverride: TimeSpan.FromMilliseconds(50),
            knownBuildsBaseDirOverride: knownBuildsDir);

        Assert.Equal(0, exitUser);
        Assert.NotNull(resUser);
        Assert.False(resUser.InstalledUe4ss);
        Assert.Equal(0, downloaderUser.CallCount);
        Assert.Equal(userSettingsIni, File.ReadAllText(Path.Combine(userUe4ss, "UE4SS-settings.ini")));
        Assert.Equal(userModsTxt, File.ReadAllText(Path.Combine(userUe4ss, "Mods", "mods.txt")));
        Assert.False(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDirUser));
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDirUser)));

        // 3. Timeout: throws TimeoutException with UE4SS.log advice, STILL cleans up mod and closes game.
        var launcherTimeout = new FakeGameLauncher
        {
            OnLaunch = _ => { /* never writes .done */ },
            RunningAfterLaunch = true,
        };
        var timeoutOut = new StringWriter();
        var timeoutEx = await Assert.ThrowsAsync<TimeoutException>(() =>
            UsmapFetcher.FetchCoreAsync(
                new FetchUsmapOptions(gameDir1, TimeoutSeconds: 1, Yes: true),
                new StringReader(string.Empty),
                timeoutOut,
                new StringWriter(),
                downloader1,
                launcherTimeout,
                agpHomeOverride: workspace.AgpHomeDir,
                isWindowsOverride: true,
                pollIntervalOverride: TimeSpan.FromMilliseconds(20),
                progressIntervalOverride: TimeSpan.FromMilliseconds(200),
                gracefulCloseWaitOverride: TimeSpan.FromMilliseconds(20)));
        Assert.Contains("UE4SS.log", timeoutEx.Message, StringComparison.Ordinal);
        Assert.Contains("等待 .usmap 转储", timeoutOut.ToString(), StringComparison.Ordinal);
        Assert.Equal(1, launcherTimeout.CloseCount);
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDir1)));

        // 4. Mod reports ok:false -> fails promptly, cleans up mod and closes game.
        var launcherModError = new FakeGameLauncher
        {
            OnLaunch = _ =>
            {
                File.WriteAllText(
                    Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDir1),
                    """{"ok":false,"usmapFile":null,"error":"DumpUSMAP failed"}""");
            },
        };
        var modErrEx = await Assert.ThrowsAsync<InvalidOperationException>(() =>
            UsmapFetcher.FetchCoreAsync(
                new FetchUsmapOptions(gameDir1, TimeoutSeconds: 5, Yes: true),
                new StringReader(string.Empty),
                new StringWriter(),
                new StringWriter(),
                downloader1,
                launcherModError,
                agpHomeOverride: workspace.AgpHomeDir,
                isWindowsOverride: true,
                pollIntervalOverride: TimeSpan.FromMilliseconds(10),
                gracefulCloseWaitOverride: TimeSpan.FromMilliseconds(20)));
        Assert.Contains("DumpUSMAP failed", modErrEx.Message, StringComparison.Ordinal);
        Assert.Contains("UE4SS.log", modErrEx.Message, StringComparison.Ordinal);
        Assert.Equal(1, launcherModError.CloseCount);
        Assert.False(Directory.Exists(Ue4ssRuntimeSupport.GetDumpModDirectory(gameDir1)));

        // 5. Without --yes, declining confirmation aborts cleanly without downloading or launching.
        var gameDirCancel = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameDecline");
        var cancelDownloader = new FakeUe4ssDownloader(zipPath);
        var cancelLauncher = new FakeGameLauncher();
        var cancelOut = new StringWriter();
        var (exitCancel, _) = await UsmapFetcher.FetchCoreAsync(
            new FetchUsmapOptions(gameDirCancel, TimeoutSeconds: 10, Yes: false),
            new StringReader("n\n"),
            cancelOut,
            new StringWriter(),
            cancelDownloader,
            cancelLauncher,
            agpHomeOverride: workspace.AgpHomeDir,
            isWindowsOverride: true);
        Assert.Equal(0, exitCancel);
        Assert.Equal(0, cancelDownloader.CallCount);
        Assert.Equal(0, cancelLauncher.LaunchCount);
        Assert.Contains("即将执行以下操作", cancelOut.ToString(), StringComparison.Ordinal);

        // 6. Non-Windows platform is explicitly rejected.
        var nonWinEx = await Assert.ThrowsAsync<PlatformNotSupportedException>(() =>
            UsmapFetcher.FetchCoreAsync(
                new FetchUsmapOptions(gameDir1, TimeoutSeconds: 10, Yes: true),
                new StringReader(string.Empty),
                new StringWriter(),
                new StringWriter(),
                downloader1,
                new FakeGameLauncher(),
                agpHomeOverride: workspace.AgpHomeDir,
                isWindowsOverride: false));
        Assert.Contains("仅支持 Windows", nonWinEx.Message, StringComparison.Ordinal);

        // 7. Game already running is rejected before modifying anything.
        var runningEx = await Assert.ThrowsAsync<InvalidOperationException>(() =>
            UsmapFetcher.FetchCoreAsync(
                new FetchUsmapOptions(gameDir1, TimeoutSeconds: 10, Yes: true),
                new StringReader(string.Empty),
                new StringWriter(),
                new StringWriter(),
                downloader1,
                new FakeGameLauncher { Running = true },
                agpHomeOverride: workspace.AgpHomeDir,
                isWindowsOverride: true));
        Assert.Contains("正在运行", runningEx.Message, StringComparison.Ordinal);
    }

    [Fact]
    public async Task CliArgsWizardDiscoveryBuildDbDefaultOutAndLogsRetention()
    {
        using var workspace = new TempWorkspace("ffr-unified-cli");

        // 1. BuildCliCommand fetch-usmap & uninstall-ue4ss args roundtrip through CLI parsers.
        var fetchArgs = BuildCliCommand.BuildFetchUsmapArguments("/games/FFRS", timeoutSeconds: 180, yes: true, language: "ja", progressJson: true);
        Assert.Contains("--lang", fetchArgs);
        Assert.Contains("ja", fetchArgs);
        Assert.Contains("--progress-json", fetchArgs);
        var parsedFetch = UsmapFetcher.ParseFetchUsmapArgs(fetchArgs.Skip(1).ToArray());
        Assert.Equal("/games/FFRS", parsedFetch.GameDirectory);
        Assert.Equal(180, parsedFetch.TimeoutSeconds);
        Assert.True(parsedFetch.Yes);
        Assert.True(parsedFetch.ProgressJson);

        var uninstallArgs = BuildCliCommand.BuildUninstallUe4ssArguments("/games/FFRS", includeForeign: true, yes: true, language: "zh-Hans");
        Assert.Contains("--lang", uninstallArgs);
        Assert.Contains("zh-Hans", uninstallArgs);
        var parsedUninstall = UsmapFetcher.ParseUninstallUe4ssArgs(uninstallArgs.Skip(1).ToArray());
        Assert.Equal("/games/FFRS", parsedUninstall.GameDirectory);
        Assert.True(parsedUninstall.IncludeForeign);
        Assert.True(parsedUninstall.Yes);

        // 2. InteractiveWizard auto-discovers .usmap in <AGP_HOME>/ffr-export/usmap/ and pressing Enter accepts it.
        var gameDir = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameWizard");
        var agpUsmapDir = AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: workspace.AgpHomeDir);
        Directory.CreateDirectory(agpUsmapDir);
        var archivedUsmap = Path.Combine(agpUsmapDir, "FFRS-24250053-11223344.usmap");
        await File.WriteAllBytesAsync(archivedUsmap, SyntheticFixtureBuilder.BuildUsmap());

        var discovered = InteractiveWizard.TryDiscoverLatestUsmap(gameDir, workspace.AgpHomeDir);
        Assert.NotNull(discovered);
        Assert.Equal(archivedUsmap, discovered.Value.FullPath);
        Assert.Equal("agp/usmap", discovered.Value.SourceLabel);

        string[]? wizardBuildArgs = null;
        var wizardStdin = string.Join(Environment.NewLine, ["1", "", "", "", ""]) + Environment.NewLine;
        var wizardExit = await InteractiveWizard.RunAsync(
            new StringReader(wizardStdin),
            new StringWriter(),
            new StringWriter(),
            args =>
            {
                wizardBuildArgs = args;
                return Task.FromResult(0);
            },
            () => [new DiscoveredSteamGame(SteamLocator.ResonanceGame, "FINAL FANTASY RESONANCE", gameDir, "24250053", workspace.Root, workspace.Root)],
            () => Path.Combine(workspace.Root, "fake-ffr-db"),
            workspace.Root,
            isWindowsOverride: false,
            agpHomeOverride: workspace.AgpHomeDir);
        Assert.Equal(0, wizardExit);
        Assert.NotNull(wizardBuildArgs);
        var parsedWizardBuild = Program.ParseBuildDbArgs(wizardBuildArgs);
        Assert.Equal(archivedUsmap, parsedWizardBuild.UsmapPath);
        Assert.Equal(
            Path.Combine(workspace.AgpHomeDir, "ffr-export", "output", "ffr-24250053"),
            parsedWizardBuild.OutputArgument);

        // 3. CliLogWriter writes to <AGP_HOME>/ffr-export/logs and prunes to latest 10 files.
        var logsDir = AgpPaths.GetLogsDirectory(agpHomeEnvOverride: workspace.AgpHomeDir);
        Directory.CreateDirectory(logsDir);
        var baseStamp = new DateTime(2026, 11, 26, 8, 0, 0, DateTimeKind.Utc);
        for (var i = 0; i < 12; i++)
        {
            var stamp = baseStamp.AddMinutes(i);
            var path = Path.Combine(logsDir, $"cli-{stamp:yyyyMMdd-HHmmss}.log");
            await File.WriteAllTextAsync(path, $"old {i}\n");
            File.SetLastWriteTimeUtc(path, stamp);
        }

        using (var logWriter = CliLogWriter.Create(new StringWriter(), logsDir))
        {
            logWriter.WriteLine("测试命令行日志");
            Assert.NotNull(logWriter.LogFilePath);
        }

        Assert.Equal(10, Directory.EnumerateFiles(logsDir, "cli-*.log").Count());

        // 4. End-to-end CLI smoke test for uninstall-ue4ss (and fetch-usmap platform check on non-Windows).
        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "UE4SS_v4.0.0.zip"));
        Ue4ssArchiveInstaller.InstallFromVerifiedZip(
            zipPath,
            gameDir,
            GameVersionDetector.ComputeFileSha256Lower(zipPath));
        Assert.True(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir));

        var cliUninstall = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "uninstall-ue4ss", "--game", gameDir],
            agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, cliUninstall.ExitCode);
        Assert.Contains("已按安装标记卸载 UE4SS", cliUninstall.Stdout, StringComparison.Ordinal);
        Assert.False(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameDir));

        // Second uninstall (nothing installed) reports "未发现 UE4SS" cleanly.
        var cliUninstallAgain = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "uninstall-ue4ss", "--game", gameDir],
            agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, cliUninstallAgain.ExitCode);
        Assert.Contains("未发现 UE4SS", cliUninstallAgain.Stdout, StringComparison.Ordinal);

        // End-to-end CLI test for foreign UE4SS: default leaves foreign UE4SS intact; --include-foreign --yes backs it up.
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDir);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir);
        Directory.CreateDirectory(Path.Combine(ue4ssDir, "Mods", "CustomForeignMod"));
        await File.WriteAllTextAsync(Path.Combine(win64Dir, "dwmapi.dll"), "foreign-dwmapi");
        await File.WriteAllTextAsync(Path.Combine(ue4ssDir, "UE4SS.dll"), "foreign-ue4ss");
        await File.WriteAllTextAsync(Path.Combine(ue4ssDir, "Mods", "CustomForeignMod", "mod.txt"), "keep");

        var cliForeignNoFlag = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "uninstall-ue4ss", "--game", gameDir],
            agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, cliForeignNoFlag.ExitCode);
        Assert.True(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.True(File.Exists(Path.Combine(ue4ssDir, "UE4SS.dll")));

        var cliForeignBackup = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "uninstall-ue4ss", "--game", gameDir, "--include-foreign", "--yes"],
            agpHome: workspace.AgpHomeDir);
        Assert.Equal(0, cliForeignBackup.ExitCode);
        Assert.Contains("恢复方法", cliForeignBackup.Stdout, StringComparison.Ordinal);
        Assert.False(File.Exists(Path.Combine(win64Dir, "dwmapi.dll")));
        Assert.False(Directory.Exists(ue4ssDir));
        var backupRoot = AgpPaths.GetUe4ssBackupDirectoryForAgpRoot(workspace.AgpHomeDir);
        var backupFolder = Assert.Single(Directory.EnumerateDirectories(backupRoot));
        Assert.Equal("foreign-dwmapi", await File.ReadAllTextAsync(Path.Combine(backupFolder, "dwmapi.dll")));
        Assert.Equal("keep", await File.ReadAllTextAsync(Path.Combine(backupFolder, "ue4ss", "Mods", "CustomForeignMod", "mod.txt")));

        // --help succeeds even when TEMP / TMP / TMPDIR point to an uncreatable path under a regular file.
        var blockedTempFile = Path.Combine(workspace.Root, "blocked-temp-sentinel");
        await File.WriteAllTextAsync(blockedTempFile, "blocked");
        var unwritableTemp = Path.Combine(blockedTempFile, "sub");
        var helpResult = await TestHarness.RunCliAsync(
            ["--help"],
            agpHome: workspace.AgpHomeDir,
            extraEnv: new Dictionary<string, string?>
            {
                ["TEMP"] = unwritableTemp,
                ["TMP"] = unwritableTemp,
                ["TMPDIR"] = unwritableTemp,
            });
        Assert.Equal(0, helpResult.ExitCode);
        Assert.Contains("uninstall-ue4ss", helpResult.Stdout, StringComparison.Ordinal);
        Assert.Contains("AGP_HOME", helpResult.Stdout, StringComparison.Ordinal);

        // Unwritable explicit AGP_HOME fails CLI immediately with clear Chinese error when --lang zh-Hans is specified.
        var unwritableAgp = Path.Combine(blockedTempFile, "agp");
        var badAgpResult = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "uninstall-ue4ss", "--game", gameDir],
            agpHome: workspace.AgpHomeDir,
            extraEnv: new Dictionary<string, string?>
            {
                [AgpPaths.AgpHomeEnvVar] = unwritableAgp,
            });
        Assert.Equal(2, badAgpResult.ExitCode);
        Assert.Contains("AGP_HOME", badAgpResult.Stderr, StringComparison.Ordinal);
        Assert.Contains("不可写", badAgpResult.Stderr, StringComparison.Ordinal);

        if (!OperatingSystem.IsWindows())
        {
            var cliFetchLinux = await TestHarness.RunCliAsync(
                ["--lang", "zh-Hans", "fetch-usmap", "--game", gameDir, "--yes"],
                agpHome: workspace.AgpHomeDir);
            Assert.Equal(2, cliFetchLinux.ExitCode);
            Assert.Contains("仅支持 Windows", cliFetchLinux.Stderr, StringComparison.Ordinal);
        }

        // 5. build-db without --out defaults to <AGP_HOME>/ffr-export/output/ffr-24250053.
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");
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

        var fakeDbExe = CopyFakeFfrDbTo(Path.Combine(workspace.Root, "fake-db"));
        var defaultBuildResult = await TestHarness.RunCliAsync(
            ["build-db", "--game", gameDir, "--usmap", archivedUsmap, "--ffr-db", fakeDbExe],
            agpHome: workspace.AgpHomeDir);
        Assert.True(defaultBuildResult.ExitCode == 0, $"stdout:\n{defaultBuildResult.Stdout}\nstderr:\n{defaultBuildResult.Stderr}");
        var expectedDefaultSqlite = Path.Combine(
            workspace.AgpHomeDir,
            "ffr-export",
            "output",
            "ffr-24250053",
            "ffr.sqlite");
        Assert.True(File.Exists(expectedDefaultSqlite));
    }

    [Fact]
    public async Task UninstallUe4ss_StripsReadOnlyOnlyOnTargetFilesAndRespectsAgpHomeAndResultFile()
    {
        using var workspace = new TempWorkspace("ffr-ue4ss-readonly");
        var gameDir = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameReadOnly");
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDir);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir);
        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "UE4SS_v4.0.0.zip"));

        Ue4ssArchiveInstaller.InstallFromVerifiedZip(
            zipPath,
            gameDir,
            GameVersionDetector.ComputeFileSha256Lower(zipPath));

        var dumpModScript = Path.Combine(ue4ssDir, "Mods", "FfrUsmapDump", "Scripts", "main.lua");
        var doneFile = Ue4ssRuntimeSupport.GetDoneMarkerPath(gameDir);
        var untouchedUserLog = Path.Combine(ue4ssDir, "UE4SS.log");
        Directory.CreateDirectory(Path.GetDirectoryName(dumpModScript)!);
        await File.WriteAllTextAsync(dumpModScript, "-- dump\n");
        await File.WriteAllTextAsync(doneFile, "{}");
        await File.WriteAllTextAsync(untouchedUserLog, "keep me read-only");

        var dwmapiPath = Path.Combine(win64Dir, "dwmapi.dll");
        var ue4ssDllPath = Path.Combine(ue4ssDir, "UE4SS.dll");
        var markerPath = Ue4ssRuntimeSupport.GetInstallMarkerPath(gameDir);

        string[] readOnlyPaths = [dwmapiPath, ue4ssDllPath, markerPath, dumpModScript, doneFile, untouchedUserLog];
        try
        {
            foreach (var path in readOnlyPaths)
            {
                File.SetAttributes(path, File.GetAttributes(path) | FileAttributes.ReadOnly);
            }

            var explicitAgpHome = Path.Combine(workspace.Root, "explicit-agp");
            var resultFile = Path.Combine(workspace.Root, "results", "readonly-result.json");
            var cliResult = await TestHarness.RunCliAsync(
                [
                    "--lang", "en",
                    "uninstall-ue4ss",
                    "--game", gameDir,
                    "--yes",
                    "--agp-home", explicitAgpHome,
                    "--result-file", resultFile,
                ],
                agpHome: workspace.AgpHomeDir);

            Assert.True(cliResult.ExitCode == 0, $"stdout:\n{cliResult.Stdout}\nstderr:\n{cliResult.Stderr}");
            Assert.False(File.Exists(dwmapiPath));
            Assert.False(File.Exists(ue4ssDllPath));
            Assert.False(File.Exists(markerPath));
            Assert.False(File.Exists(dumpModScript));
            Assert.False(File.Exists(doneFile));

            // Untouched non-manifest user file must still exist AND retain its ReadOnly attribute.
            Assert.True(File.Exists(untouchedUserLog));
            Assert.True((File.GetAttributes(untouchedUserLog) & FileAttributes.ReadOnly) != 0);

            Assert.True(File.Exists(resultFile));
            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(resultFile, out var doc, out var err), err);
            Assert.NotNull(doc);
            Assert.Equal(ProgressProtocol.CleanupStatusCompleted, doc.Status);
            Assert.Equal("removed_own_install", doc.Outcome);
            Assert.Equal(0, doc.ExitCode);
            Assert.True(doc.RemovedFiles >= 7);
            Assert.Null(doc.BlockedPath);

            // --agp-home must override AGP_HOME env var for foreign backup storage.
            Directory.CreateDirectory(ue4ssDir);
            await File.WriteAllTextAsync(dwmapiPath, "foreign-dwmapi");
            await File.WriteAllTextAsync(ue4ssDllPath, "foreign-ue4ss");
            var foreignResultFile = Path.Combine(workspace.Root, "results", "foreign-result.json");
            var foreignRes = await TestHarness.RunCliAsync(
                [
                    "--lang", "en",
                    "uninstall-ue4ss",
                    "--game", gameDir,
                    "--include-foreign",
                    "--yes",
                    "--agp-home", explicitAgpHome,
                    "--result-file", foreignResultFile,
                ],
                agpHome: workspace.AgpHomeDir);

            Assert.True(foreignRes.ExitCode == 0, $"stdout:\n{foreignRes.Stdout}\nstderr:\n{foreignRes.Stderr}");
            var explicitBackupRoot = AgpPaths.GetUe4ssBackupDirectoryForAgpRoot(explicitAgpHome);
            var envBackupRoot = AgpPaths.GetUe4ssBackupDirectoryForAgpRoot(workspace.AgpHomeDir);
            Assert.True(Directory.Exists(explicitBackupRoot));
            var backupDir = Assert.Single(Directory.EnumerateDirectories(explicitBackupRoot));
            Assert.False(Directory.Exists(envBackupRoot));

            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(foreignResultFile, out var foreignDoc, out var foreignErr), foreignErr);
            Assert.NotNull(foreignDoc);
            Assert.Equal(ProgressProtocol.CleanupStatusCompleted, foreignDoc.Status);
            Assert.Equal("backed_up_foreign_install", foreignDoc.Outcome);
            Assert.Equal(backupDir, foreignDoc.BackupDirectory);
        }
        finally
        {
            foreach (var path in readOnlyPaths)
            {
                if (File.Exists(path))
                {
                    try
                    {
                        File.SetAttributes(path, File.GetAttributes(path) & ~FileAttributes.ReadOnly);
                    }
                    catch
                    {
                        // Best-effort cleanup.
                    }
                }
            }
        }
    }

    [Fact]
    public async Task UninstallUe4ss_AccessDeniedInUseMarkerLastSafeRetryAndFetchClearError()
    {
        using var workspace = new TempWorkspace("ffr-ue4ss-elev");
        var zipPath = CreateSyntheticUe4ssZip(Path.Combine(workspace.Root, "UE4SS_v4.0.0.zip"));
        var zipSha = GameVersionDetector.ComputeFileSha256Lower(zipPath);
        var stoppedLauncher = new FakeGameLauncher { Running = false };

        // 1. Own install: marker (.installed-by-ffr-export.json) sorts before UE4SS.dll, yet must be deleted LAST.
        //    When deleting UE4SS.dll fails with UnauthorizedAccessException:
        //    - exit code is ExitCodeNeedsElevation (5)
        //    - stdout emits @@ue4ss-cleanup with status=needs_elevation, reason=access_denied, path=...UE4SS.dll
        //    - result-file JSON contains matching status, exit_code, blocked_path, reason
        //    - marker file STILL exists so retry works safely.
        var gameOwn = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameOwnDeny");
        Ue4ssArchiveInstaller.InstallFromVerifiedZip(zipPath, gameOwn, zipSha);
        var ownUe4ssDll = Path.Combine(Ue4ssRuntimeSupport.GetUe4ssDirectory(gameOwn), "UE4SS.dll");
        var ownMarker = Ue4ssRuntimeSupport.GetInstallMarkerPath(gameOwn);

        using (Localization.UseLanguageScope("zh-Hans"))
        {
            var denyResultFile = Path.Combine(workspace.Root, "own-deny.json");
            var denyOut = new StringWriter();
            var denyErr = new StringWriter();
            var denyExit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameOwn, IncludeForeign: false, Yes: true, AgpHome: workspace.AgpHomeDir, ResultFile: denyResultFile),
                stoppedLauncher,
                new StringReader(string.Empty),
                denyOut,
                denyErr,
                beforeDeleteHook: path =>
                {
                    if (string.Equals(Path.GetFileName(path), "UE4SS.dll", StringComparison.OrdinalIgnoreCase))
                    {
                        throw new UnauthorizedAccessException("Access to the path is denied.");
                    }
                });

            Assert.Equal(ProgressProtocol.ExitCodeNeedsElevation, denyExit);
            Assert.True(File.Exists(ownMarker), "Install marker must remain intact when deleting another file fails.");
            Assert.True(Ue4ssRuntimeSupport.HasOwnInstallMarker(gameOwn));

            var protocolLine = denyOut.ToString()
                .Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries)
                .Single(line => line.StartsWith(ProgressProtocol.Ue4ssCleanupPrefix, StringComparison.Ordinal));
            Assert.True(ProgressProtocol.TryParseUe4ssCleanupLine(protocolLine, out var msg));
            Assert.Equal(ProgressProtocol.CleanupStatusNeedsElevation, msg.Status);
            Assert.Equal(ProgressProtocol.CleanupReasonAccessDenied, msg.Reason);
            Assert.Equal(ownUe4ssDll, msg.Path);

            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(denyResultFile, out var doc, out var err), err);
            Assert.NotNull(doc);
            Assert.Equal(ProgressProtocol.CleanupStatusNeedsElevation, doc.Status);
            Assert.Equal(ProgressProtocol.ExitCodeNeedsElevation, doc.ExitCode);
            Assert.Equal(ProgressProtocol.CleanupReasonAccessDenied, doc.Reason);
            Assert.Equal(ownUe4ssDll, doc.BlockedPath);

            // Retry after permissions are granted completes cleanly and deletes UE4SS.dll and marker.
            var retryResultFile = Path.Combine(workspace.Root, "own-retry.json");
            var retryOut = new StringWriter();
            var retryExit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameOwn, IncludeForeign: false, Yes: true, AgpHome: workspace.AgpHomeDir, ResultFile: retryResultFile),
                stoppedLauncher,
                new StringReader(string.Empty),
                retryOut,
                new StringWriter());
            Assert.Equal(0, retryExit);
            Assert.False(File.Exists(ownUe4ssDll));
            Assert.False(File.Exists(ownMarker));
            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(retryResultFile, out var retryDoc, out var retryErr), retryErr);
            Assert.NotNull(retryDoc);
            Assert.Equal(ProgressProtocol.CleanupStatusCompleted, retryDoc.Status);
            Assert.Equal("removed_own_install", retryDoc.Outcome);
        }

        // 2. Sharing violation IOException (file in use) -> ExitCodeFileInUse (6), reason=in_use, prompts user to exit game, no elevation.
        var gameInUse = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameInUse");
        Ue4ssArchiveInstaller.InstallFromVerifiedZip(zipPath, gameInUse, zipSha);
        var inUseDwmapi = Path.Combine(Ue4ssRuntimeSupport.GetWin64Directory(gameInUse), "dwmapi.dll");
        var inUseMarker = Ue4ssRuntimeSupport.GetInstallMarkerPath(gameInUse);

        using (Localization.UseLanguageScope("zh-Hans"))
        {
            var inUseResultFile = Path.Combine(workspace.Root, "in-use.json");
            var inUseOut = new StringWriter();
            var inUseExit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameInUse, IncludeForeign: false, Yes: true, AgpHome: workspace.AgpHomeDir, ResultFile: inUseResultFile),
                stoppedLauncher,
                new StringReader(string.Empty),
                inUseOut,
                new StringWriter(),
                beforeDeleteHook: path =>
                {
                    if (string.Equals(Path.GetFileName(path), "dwmapi.dll", StringComparison.OrdinalIgnoreCase))
                    {
                        throw new IOException("The process cannot access the file because it is being used by another process.", unchecked((int)0x80070020));
                    }
                });

            Assert.Equal(ProgressProtocol.ExitCodeFileInUse, inUseExit);
            Assert.True(File.Exists(inUseMarker));
            Assert.Contains("请确认游戏已完全退出后重试", inUseOut.ToString(), StringComparison.Ordinal);

            var protocolLine = inUseOut.ToString()
                .Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries)
                .Single(line => line.StartsWith(ProgressProtocol.Ue4ssCleanupPrefix, StringComparison.Ordinal));
            Assert.True(ProgressProtocol.TryParseUe4ssCleanupLine(protocolLine, out var inUseMsg));
            Assert.Equal(ProgressProtocol.CleanupStatusInUse, inUseMsg.Status);
            Assert.Equal(ProgressProtocol.CleanupReasonInUse, inUseMsg.Reason);
            Assert.Equal(inUseDwmapi, inUseMsg.Path);

            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(inUseResultFile, out var inUseDoc, out var inUseErr), inUseErr);
            Assert.NotNull(inUseDoc);
            Assert.Equal(ProgressProtocol.CleanupStatusInUse, inUseDoc.Status);
            Assert.Equal(ProgressProtocol.CleanupReasonInUse, inUseDoc.Reason);
            Assert.Equal(ProgressProtocol.ExitCodeFileInUse, inUseDoc.ExitCode);
            Assert.Equal(inUseDwmapi, inUseDoc.BlockedPath);
        }

        // Verify HResult classification for ERROR_LOCK_VIOLATION (33) and ERROR_USER_MAPPED_FILE (1224).
        Assert.True(Ue4ssRuntimeSupport.TryClassifyAccessFailure(
            new IOException("lock violation", unchecked((int)0x80070021)),
            out var lockReason));
        Assert.Equal(ProgressProtocol.CleanupReasonInUse, lockReason);
        Assert.True(Ue4ssRuntimeSupport.TryClassifyAccessFailure(
            new IOException("user mapped file", unchecked((int)0x800704C8)),
            out var mappedReason));
        Assert.Equal(ProgressProtocol.CleanupReasonInUse, mappedReason);
        Assert.False(Ue4ssRuntimeSupport.TryClassifyAccessFailure(
            new IOException("generic disk error"),
            out _));

        // 3. Foreign UE4SS partial delete failure -> retry verifies content matches and reuses the SAME backup (no second backup created!).
        var gameForeign = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameForeignRetry");
        var foreignWin64 = Ue4ssRuntimeSupport.GetWin64Directory(gameForeign);
        var foreignUe4ss = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameForeign);
        var customModFile = Path.Combine(foreignUe4ss, "Mods", "CustomMod", "main.lua");
        Directory.CreateDirectory(Path.GetDirectoryName(customModFile)!);
        await File.WriteAllTextAsync(Path.Combine(foreignWin64, "dwmapi.dll"), "foreign-dwmapi-v1");
        await File.WriteAllTextAsync(Path.Combine(foreignUe4ss, "UE4SS.dll"), "foreign-ue4ss-v1");
        await File.WriteAllTextAsync(Path.Combine(foreignUe4ss, "UE4SS-settings.ini"), "[Debug]\nConsoleEnabled = 1\n");
        await File.WriteAllTextAsync(customModFile, "-- user custom mod\n");

        var foreignAgp = Path.Combine(workspace.Root, "foreign-retry-agp");
        var firstTime = new DateTimeOffset(2026, 11, 27, 14, 0, 0, TimeSpan.FromHours(9));
        var secondTime = firstTime.AddMinutes(5);

        using (Localization.UseLanguageScope("zh-Hans"))
        {
            var attempt1ResultFile = Path.Combine(workspace.Root, "foreign-attempt1.json");
            var attempt1Exit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameForeign, IncludeForeign: true, Yes: true, AgpHome: foreignAgp, ResultFile: attempt1ResultFile),
                stoppedLauncher,
                new StringReader(string.Empty),
                new StringWriter(),
                new StringWriter(),
                nowOverride: () => firstTime,
                beforeDeleteHook: path =>
                {
                    // Allow CustomMod/main.lua to be deleted first (partial failure), then fail on UE4SS.dll.
                    if (string.Equals(Path.GetFileName(path), "UE4SS.dll", StringComparison.OrdinalIgnoreCase))
                    {
                        throw new UnauthorizedAccessException("Access denied on UE4SS.dll");
                    }
                });

            Assert.Equal(ProgressProtocol.ExitCodeNeedsElevation, attempt1Exit);
            Assert.False(File.Exists(customModFile), "Non-sentinel file was partially deleted on attempt 1.");
            Assert.True(File.Exists(Path.Combine(foreignUe4ss, "UE4SS.dll")), "Sentinel UE4SS.dll remains for retry detection.");
            Assert.True(File.Exists(Path.Combine(foreignWin64, "dwmapi.dll")), "Sentinel dwmapi.dll remains for retry detection.");

            var backupRoot = AgpPaths.GetUe4ssBackupDirectoryForAgpRoot(foreignAgp);
            var firstBackupDir = Assert.Single(Directory.EnumerateDirectories(backupRoot));
            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(attempt1ResultFile, out var doc1, out var err1), err1);
            Assert.NotNull(doc1);
            Assert.Equal(firstBackupDir, doc1.BackupDirectory);

            // Attempt 2 at a later timestamp reuses the exact same backup folder without creating a second one.
            var attempt2ResultFile = Path.Combine(workspace.Root, "foreign-attempt2.json");
            var attempt2Out = new StringWriter();
            var attempt2Exit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameForeign, IncludeForeign: true, Yes: true, AgpHome: foreignAgp, ResultFile: attempt2ResultFile),
                stoppedLauncher,
                new StringReader(string.Empty),
                attempt2Out,
                new StringWriter(),
                nowOverride: () => secondTime);

            Assert.Equal(0, attempt2Exit);
            var onlyBackupDir = Assert.Single(Directory.EnumerateDirectories(backupRoot));
            Assert.Equal(firstBackupDir, onlyBackupDir);
            Assert.Contains("跳过重复复制", attempt2Out.ToString(), StringComparison.Ordinal);
            Assert.False(File.Exists(Path.Combine(foreignWin64, "dwmapi.dll")));
            Assert.False(Directory.Exists(foreignUe4ss));

            // Full backup integrity verified (including CustomMod/main.lua that had been deleted in attempt 1).
            Assert.Equal("foreign-dwmapi-v1", await File.ReadAllTextAsync(Path.Combine(onlyBackupDir, "dwmapi.dll")));
            Assert.Equal("foreign-ue4ss-v1", await File.ReadAllTextAsync(Path.Combine(onlyBackupDir, "ue4ss", "UE4SS.dll")));
            Assert.Equal("-- user custom mod\n", await File.ReadAllTextAsync(Path.Combine(onlyBackupDir, "ue4ss", "Mods", "CustomMod", "main.lua")));

            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(attempt2ResultFile, out var doc2, out var err2), err2);
            Assert.NotNull(doc2);
            Assert.Equal(ProgressProtocol.CleanupStatusCompleted, doc2.Status);
            Assert.Equal(onlyBackupDir, doc2.BackupDirectory);

            // If new modified foreign files appear later, content hash differs so a new backup IS created.
            Directory.CreateDirectory(foreignUe4ss);
            await File.WriteAllTextAsync(Path.Combine(foreignWin64, "dwmapi.dll"), "foreign-dwmapi-v2-changed");
            await File.WriteAllTextAsync(Path.Combine(foreignUe4ss, "UE4SS.dll"), "foreign-ue4ss-v2-changed");
            var attempt3Exit = UsmapFetcher.RunUninstall(
                new UninstallUe4ssOptions(gameForeign, IncludeForeign: true, Yes: true, AgpHome: foreignAgp),
                stoppedLauncher,
                new StringReader(string.Empty),
                new StringWriter(),
                new StringWriter(),
                nowOverride: () => secondTime.AddMinutes(5));
            Assert.Equal(0, attempt3Exit);
            Assert.Equal(2, Directory.EnumerateDirectories(backupRoot).Count());
        }

        // 4. Real OS-level delete denial via CLI subprocess (chmod 0555 on non-root Linux, icacls DE/DC on Windows).
        await VerifyOsLevelDeleteDeniedViaCliAsync(workspace.Root, zipPath, zipSha);

        // 5. fetch-usmap install & temporary mod deployment report clear actionable error on UnauthorizedAccessException.
        var gameFetchDeny = CreateSyntheticGameDirectory(workspace.Root, "24250053", "GameFetchDeny");
        using (Localization.UseLanguageScope("zh-Hans"))
        {
            var installEx = Assert.Throws<UnauthorizedAccessException>(() =>
                Ue4ssArchiveInstaller.InstallFromVerifiedZip(
                    zipPath,
                    gameFetchDeny,
                    zipSha,
                    beforeFileWriteHook: _ => throw new UnauthorizedAccessException("Access denied.")));
            Assert.Contains("对当前用户不可写", installEx.Message, StringComparison.Ordinal);
            Assert.Contains("清理 UE4SS", installEx.Message, StringComparison.Ordinal);
            Assert.Contains("管理员身份", installEx.Message, StringComparison.Ordinal);
        }

        using (Localization.UseLanguageScope("en"))
        {
            var installExEn = Assert.Throws<UnauthorizedAccessException>(() =>
                Ue4ssArchiveInstaller.InstallFromVerifiedZip(
                    zipPath,
                    gameFetchDeny,
                    zipSha,
                    beforeFileWriteHook: _ => throw new UnauthorizedAccessException("Access denied.")));
            Assert.Contains("not writable by the current user", installExEn.Message, StringComparison.Ordinal);
            Assert.Contains("Clean up UE4SS", installExEn.Message, StringComparison.Ordinal);
            Assert.Contains("Administrator", installExEn.Message, StringComparison.Ordinal);
        }
    }

    private static async Task VerifyOsLevelDeleteDeniedViaCliAsync(string root, string zipPath, string zipSha)
    {
        var gameDir = CreateSyntheticGameDirectory(root, "24250053", "GameOsDeny");
        Ue4ssArchiveInstaller.InstallFromVerifiedZip(zipPath, gameDir, zipSha);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDir);
        var markerPath = Ue4ssRuntimeSupport.GetInstallMarkerPath(gameDir);
        var probeDir = Path.Combine(root, "os-acl-probe");
        var probeFile = Path.Combine(probeDir, "probe.txt");
        Directory.CreateDirectory(probeDir);
        await File.WriteAllTextAsync(probeFile, "probe");

        var denyAppliedToProbe = false;
        var denyAppliedToUe4ss = false;
        try
        {
            denyAppliedToProbe = TryApplyDeleteDeny(probeDir);
            var osEnforcesDeleteDeny = false;
            if (denyAppliedToProbe)
            {
                try
                {
                    File.Delete(probeFile);
                }
                catch (UnauthorizedAccessException)
                {
                    osEnforcesDeleteDeny = true;
                }
            }

            if (!osEnforcesDeleteDeny)
            {
                // Root on Linux (uid 0) bypasses chmod 0555; on Windows and non-root Linux this must always take effect.
                Assert.False(OperatingSystem.IsWindows(), "icacls deny DE/DC must take effect on Windows.");
                return;
            }

            Assert.True(TryApplyDeleteDeny(ue4ssDir));
            denyAppliedToUe4ss = true;

            var agpHome = Path.Combine(root, "os-deny-agp");
            var resultFile = Path.Combine(root, "os-deny-result.json");
            var cliDeny = await TestHarness.RunCliAsync(
                [
                    "--lang", "en",
                    "uninstall-ue4ss",
                    "--game", gameDir,
                    "--yes",
                    "--agp-home", agpHome,
                    "--result-file", resultFile,
                ],
                agpHome: agpHome);

            Assert.Equal(ProgressProtocol.ExitCodeNeedsElevation, cliDeny.ExitCode);
            var protocolLine = cliDeny.Stdout
                .Split(['\r', '\n'], StringSplitOptions.RemoveEmptyEntries)
                .Single(line => line.StartsWith(ProgressProtocol.Ue4ssCleanupPrefix, StringComparison.Ordinal));
            Assert.True(ProgressProtocol.TryParseUe4ssCleanupLine(protocolLine, out var msg));
            Assert.Equal(ProgressProtocol.CleanupStatusNeedsElevation, msg.Status);
            Assert.Equal(ProgressProtocol.CleanupReasonAccessDenied, msg.Reason);
            Assert.False(string.IsNullOrWhiteSpace(msg.Path));

            Assert.True(Ue4ssCleanupElevation.TryReadResultFile(resultFile, out var denyDoc, out var denyErr), denyErr);
            Assert.NotNull(denyDoc);
            Assert.Equal(ProgressProtocol.CleanupStatusNeedsElevation, denyDoc.Status);
            Assert.Equal(ProgressProtocol.ExitCodeNeedsElevation, denyDoc.ExitCode);
            Assert.Equal(ProgressProtocol.CleanupReasonAccessDenied, denyDoc.Reason);
            Assert.Equal(msg.Path, denyDoc.BlockedPath);

            // Lift deny and verify marker was preserved and retry removes everything cleanly.
            RemoveDeleteDeny(ue4ssDir);
            denyAppliedToUe4ss = false;
            Assert.True(File.Exists(markerPath));

            var retryResultFile = Path.Combine(root, "os-retry-result.json");
            var cliRetry = await TestHarness.RunCliAsync(
                [
                    "--lang", "en",
                    "uninstall-ue4ss",
                    "--game", gameDir,
                    "--yes",
                    "--agp-home", agpHome,
                    "--result-file", retryResultFile,
                ],
                agpHome: agpHome);

            Assert.True(cliRetry.ExitCode == 0, $"stdout:\n{cliRetry.Stdout}\nstderr:\n{cliRetry.Stderr}");
            Assert.False(File.Exists(markerPath));
            Assert.False(Directory.Exists(ue4ssDir));
        }
        finally
        {
            if (denyAppliedToUe4ss)
            {
                RemoveDeleteDeny(ue4ssDir);
            }

            if (denyAppliedToProbe)
            {
                RemoveDeleteDeny(probeDir);
            }
        }
    }

    private static bool TryApplyDeleteDeny(string directory)
    {
        if (OperatingSystem.IsWindows())
        {
            using var proc = Process.Start(new ProcessStartInfo
            {
                FileName = "icacls",
                Arguments = $"\"{directory}\" /deny *S-1-1-0:(OI)(CI)(DE,DC)",
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                CreateNoWindow = true,
            });
            if (proc is null)
            {
                return false;
            }

            proc.WaitForExit(10_000);
            return proc.ExitCode == 0;
        }

        File.SetUnixFileMode(
            directory,
            UnixFileMode.UserRead | UnixFileMode.UserExecute
            | UnixFileMode.GroupRead | UnixFileMode.GroupExecute
            | UnixFileMode.OtherRead | UnixFileMode.OtherExecute);
        return true;
    }

    private static void RemoveDeleteDeny(string directory)
    {
        try
        {
            if (!Directory.Exists(directory))
            {
                return;
            }

            if (OperatingSystem.IsWindows())
            {
                using var proc = Process.Start(new ProcessStartInfo
                {
                    FileName = "icacls",
                    Arguments = $"\"{directory}\" /remove:d *S-1-1-0",
                    UseShellExecute = false,
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    CreateNoWindow = true,
                });
                proc?.WaitForExit(10_000);
            }
            else
            {
                File.SetUnixFileMode(
                    directory,
                    UnixFileMode.UserRead | UnixFileMode.UserWrite | UnixFileMode.UserExecute
                    | UnixFileMode.GroupRead | UnixFileMode.GroupExecute
                    | UnixFileMode.OtherRead | UnixFileMode.OtherExecute);
            }
        }
        catch
        {
            // Best-effort ACL restoration during test cleanup.
        }
    }

    private sealed class FakeUe4ssDownloader : IUe4ssPackageDownloader
    {
        private readonly string _zipPath;

        internal FakeUe4ssDownloader(string zipPath)
        {
            _zipPath = zipPath;
        }

        internal int CallCount { get; private set; }

        public Task<string> EnsureVerifiedArchiveAsync(
            string downloadsDirectory,
            TextWriter output,
            CancellationToken cancellationToken)
        {
            CallCount++;
            Directory.CreateDirectory(downloadsDirectory);
            var target = Path.Combine(downloadsDirectory, HttpsUe4ssPackageDownloader.OfficialArchiveFileName);
            File.Copy(_zipPath, target, overwrite: true);
            return Task.FromResult(target);
        }
    }

    private sealed class FakeGameLauncher : IGameLauncher
    {
        internal bool Running { get; set; }

        internal bool RunningAfterLaunch { get; set; }

        internal int LaunchCount { get; private set; }

        internal int CloseCount { get; private set; }

        internal Action<string>? OnLaunch { get; set; }

        public bool IsGameRunning() => Running;

        public void LaunchGameViaSteam(string steamUri)
        {
            LaunchCount++;
            if (RunningAfterLaunch)
            {
                Running = true;
            }

            OnLaunch?.Invoke(steamUri);
        }

        public Task CloseGameAsync(TimeSpan gracefulWait, Action<string> log, CancellationToken cancellationToken)
        {
            CloseCount++;
            Running = false;
            log("模拟关闭游戏进程完成。");
            return Task.CompletedTask;
        }
    }

    private static string CreateSyntheticGameDirectory(string root, string buildId, string folderName = "FINAL FANTASY RESONANCE")
    {
        var steamappsDir = Path.Combine(root, "Steam", "steamapps");
        var gameDir = Path.Combine(steamappsDir, "common", folderName);
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");
        var win64Dir = Path.Combine(gameDir, "FFRS", "Binaries", "Win64");
        Directory.CreateDirectory(paksDir);
        Directory.CreateDirectory(win64Dir);

        File.WriteAllBytes(
            Path.Combine(win64Dir, "FFRS-Win64-Shipping.exe"),
            SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource((1, 0, 0, 0), (1, 0, 0, 0)));

        File.WriteAllText(
            Path.Combine(steamappsDir, "appmanifest_3259780.acf"),
            $$"""
            "AppState"
            {
            	"appid"		"3259780"
            	"name"		"FINAL FANTASY RESONANCE"
            	"installdir"		"{{folderName}}"
            	"buildid"		"{{buildId}}"
            }
            """,
            Encoding.UTF8);

        return gameDir;
    }

    private static string CreateSyntheticUe4ssZip(string zipPath)
    {
        if (File.Exists(zipPath))
        {
            File.Delete(zipPath);
        }

        using var archive = ZipFile.Open(zipPath, ZipArchiveMode.Create);
        WriteZipEntry(archive, "dwmapi.dll", "synthetic-dwmapi-dll");
        WriteZipEntry(archive, "ue4ss/UE4SS.dll", "synthetic-ue4ss-dll");
        WriteZipEntry(archive, "ue4ss/LICENSE", "MIT License - synthetic test copy\n");
        WriteZipEntry(
            archive,
            "ue4ss/UE4SS-settings.ini",
            """
            [General]
            EnableHotReloadSystem = 1
            bUseUObjectArrayCache = true

            [EngineVersionOverride]
            MajorVersion =
            MinorVersion =

            [Debug]
            ; Whether to enable the external UE4SS debug console.
            ConsoleEnabled = 1
            GuiConsoleEnabled = 1
            GuiConsoleVisible = 1
            """);
        WriteZipEntry(archive, "ue4ss/Mods/mods.txt", "CheatManagerEnablerMod : 0\nKeybinds : 1\n");
        return zipPath;
    }

    private static void WriteZipEntry(ZipArchive archive, string entryName, string content)
    {
        var entry = archive.CreateEntry(entryName);
        using var stream = entry.Open();
        using var writer = new StreamWriter(stream, new UTF8Encoding(false));
        writer.Write(content);
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
