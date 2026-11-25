using System.Security.Cryptography;
using System.Text;
using System.Text.RegularExpressions;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class BuildDbTests
{
    [Fact]
    public void PeVersionReader_ParsesSyntheticPe32PlusAndPe32AndFallback()
    {
        // 1. PE32+ with resource directory placed in ".link" section (matching real UE5 shipping layout)
        //    and StringFileInfo overriding fixed version strings.
        var pe64 = SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource(
            fileVersion: (1, 0, 0, 0),
            productVersion: (1, 0, 0, 0),
            stringFileVersion: "1.2.3.4",
            stringProductVersion: "1.2.3-rc1",
            sectionName: ".link",
            pe32Plus: true);

        using (var ms = new MemoryStream(pe64))
        {
            var parsed = PeVersionReader.ReadFromStream(ms, "synthetic64.exe");
            Assert.Equal("1.2.3.4", parsed.FileVersion);
            Assert.Equal("1.2.3-rc1", parsed.ProductVersion);
            Assert.Equal("1.0.0.0", parsed.FixedFileVersion);
            Assert.Equal("1.0.0.0", parsed.FixedProductVersion);
        }

        // 2. PE32 with VS_FIXEDFILEINFO only (no StringFileInfo) falls back to fixed version numbers.
        var pe32 = SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource(
            fileVersion: (5, 6, 1, 99),
            productVersion: (2, 4, 0, 1),
            stringFileVersion: null,
            stringProductVersion: null,
            sectionName: ".rsrc",
            pe32Plus: false);

        using (var ms = new MemoryStream(pe32))
        {
            var parsed = PeVersionReader.ReadFromStream(ms, "synthetic32.exe");
            Assert.Equal("5.6.1.99", parsed.FileVersion);
            Assert.Equal("2.4.0.1", parsed.ProductVersion);
        }

        // 3. Invalid non-PE bytes throw InvalidDataException with clear Chinese message.
        using (var ms = new MemoryStream(Encoding.UTF8.GetBytes("not a pe binary at all - padding bytes to exceed 64 bytes........................")))
        {
            var ex = Assert.Throws<InvalidDataException>(() => PeVersionReader.ReadFromStream(ms, "bad.exe"));
            Assert.Contains("解析 PE 版本资源失败", ex.Message, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void PakFingerprint_ChangesWithContentAndIsInvariantToOrder()
    {
        using var workspace = new TempWorkspace("ffr-pak-fp");
        var paksDir = Path.Combine(workspace.Root, "Paks");
        Directory.CreateDirectory(paksDir);

        var pakPath = Path.Combine(paksDir, "Game-Windows.pak");
        var utocPath = Path.Combine(paksDir, "Game-Windows.utoc");
        var ucasPath = Path.Combine(paksDir, "Game-Windows.ucas");
        var ignoredPath = Path.Combine(paksDir, "readme.txt");

        File.WriteAllBytes(pakPath, Encoding.UTF8.GetBytes("pak-content-v1"));
        File.WriteAllBytes(utocPath, Encoding.UTF8.GetBytes("utoc-content-v1"));
        File.WriteAllBytes(ucasPath, new byte[128]);
        File.WriteAllText(ignoredPath, "ignored");

        var fp1 = GameVersionDetector.ComputePakFingerprint(paksDir);
        Assert.Equal(3, fp1.Files.Length);
        Assert.NotNull(fp1.Files.Single(f => f.RelativePath == "Game-Windows.pak").Sha256);
        Assert.NotNull(fp1.Files.Single(f => f.RelativePath == "Game-Windows.utoc").Sha256);
        Assert.Null(fp1.Files.Single(f => f.RelativePath == "Game-Windows.ucas").Sha256);

        // Order invariance in CombinePakFingerprint
        var reversed = fp1.Files.Reverse().ToArray();
        Assert.Equal(fp1.Fingerprint, GameVersionDetector.CombinePakFingerprint(reversed));

        // Modifying .ucas bytes WITHOUT changing size keeps fingerprint unchanged (.ucas only tracks size)
        var sameSizeUcas = new byte[128];
        Array.Fill(sameSizeUcas, (byte)0xAB);
        File.WriteAllBytes(ucasPath, sameSizeUcas);
        var fpSameUcasSize = GameVersionDetector.ComputePakFingerprint(paksDir);
        Assert.Equal(fp1.Fingerprint, fpSameUcasSize.Fingerprint);

        // Changing .ucas size changes fingerprint
        File.WriteAllBytes(ucasPath, new byte[129]);
        var fpDiffUcasSize = GameVersionDetector.ComputePakFingerprint(paksDir);
        Assert.NotEqual(fp1.Fingerprint, fpDiffUcasSize.Fingerprint);

        // Restore .ucas size, then mutate .pak content (same length) -> fingerprint changes
        File.WriteAllBytes(ucasPath, new byte[128]);
        File.WriteAllBytes(pakPath, Encoding.UTF8.GetBytes("pak-content-v2"));
        var fpDiffPak = GameVersionDetector.ComputePakFingerprint(paksDir);
        Assert.NotEqual(fp1.Fingerprint, fpDiffPak.Fingerprint);
    }

    [Fact]
    public void UsmapContentHash_IsInvariantToEntryOrderAndSensitiveToContentChanges()
    {
        using var workspace = new TempWorkspace("ffr-usmap-hash");

        var enumA = new SyntheticFixtureBuilder.UsmapEnumSpec("EAlpha", ["None", "First", "Second"]);
        var enumB = new SyntheticFixtureBuilder.UsmapEnumSpec("EBeta", ["Off", "On"]);

        var structA = new SyntheticFixtureBuilder.UsmapStructSpec(
            "FAlphaRow",
            "FTableRowBase",
            3,
            [
                new(0, 1, "Id", 2), // IntProperty
                new(1, 1, "Kind", 26, EnumName: "EAlpha", EnumInnerType: 0), // EnumProperty
                new(2, 1, "Tags", 8, InnerType: 10), // ArrayProperty<StrProperty>
            ]);
        var structB = new SyntheticFixtureBuilder.UsmapStructSpec(
            "FBetaRow",
            null,
            2,
            [
                new(0, 1, "Ref", 9, StructName: "FAlphaRow"), // StructProperty
                new(1, 1, "Weights", 24, InnerType: 5, ValueType: 3), // MapProperty<NameProperty, FloatProperty>
            ]);

        var structAShuffledProps = structA with
        {
            Properties = [structA.Properties[2], structA.Properties[0], structA.Properties[1]],
        };
        var structBShuffledProps = structB with
        {
            Properties = [structB.Properties[1], structB.Properties[0]],
        };

        var bytesOrder1 = SyntheticFixtureBuilder.BuildUsmapV0(
            ["EAlpha", "EBeta", "FAlphaRow", "FBetaRow", "Id", "Kind", "Tags"],
            [enumA, enumB],
            [structA, structB]);

        var bytesOrder2 = SyntheticFixtureBuilder.BuildUsmapV0(
            ["Weights", "Ref", "FBetaRow", "FTableRowBase", "EBeta", "On", "Off", "EAlpha"],
            [enumB, enumA],
            [structBShuffledProps, structAShuffledProps]);

        var structBModified = structB with
        {
            Properties =
            [
                structB.Properties[0],
                new(1, 1, "Weights", 24, InnerType: 5, ValueType: 7), // DoubleProperty instead of FloatProperty
            ],
        };
        var bytesModified = SyntheticFixtureBuilder.BuildUsmapV0(
            ["EAlpha", "EBeta", "FAlphaRow", "FBetaRow"],
            [enumA, enumB],
            [structA, structBModified]);

        var path1 = Path.Combine(workspace.Root, "order1.usmap");
        var path2 = Path.Combine(workspace.Root, "order2.usmap");
        var path3 = Path.Combine(workspace.Root, "modified.usmap");
        File.WriteAllBytes(path1, bytesOrder1);
        File.WriteAllBytes(path2, bytesOrder2);
        File.WriteAllBytes(path3, bytesModified);

        var hash1 = GameVersionDetector.ComputeUsmapHashes(path1);
        var hash2 = GameVersionDetector.ComputeUsmapHashes(path2);
        var hash3 = GameVersionDetector.ComputeUsmapHashes(path3);

        // Raw bytes differ, so file_sha256 differs, while canonical content_sha256 is identical.
        Assert.NotEqual(hash1.FileSha256, hash2.FileSha256);
        Assert.Equal(hash1.ContentSha256, hash2.ContentSha256);
        Assert.Equal(2, hash1.TypeCount);
        Assert.Equal(2, hash1.EnumCount);

        // Semantic change alters content_sha256.
        Assert.NotEqual(hash1.ContentSha256, hash3.ContentSha256);
    }

    [Fact]
    public void KnownBuildsComparison_CoversAllThreeOutcomesAndFileLoadingPriority()
    {
        using var workspace = new TempWorkspace("ffr-known-builds");

        var registeredEntry = new KnownBuildEntry(
            RegisteredAt: "2026-11-25",
            Note: "synthetic baseline",
            GameVersion: new GameVersionInfo(
                new SteamBuildInfo(3259780, "24250053", "FINAL FANTASY RESONANCE", "1761652000", "appmanifest"),
                new ExeVersionInfo("FFRS/Binaries/Win64/FFRS-Win64-Shipping.exe", 1024, "1.0.0.0", "1.0.0.0"),
                "aaaa1111bbbb2222",
                []),
            Inputs: null,
            Tools: null,
            Database: new BuildDatabaseInfo(
                "ffr.sqlite",
                4096,
                1,
                "3.49.1",
                "filehash_expected",
                "contenthash_expected",
                1,
                [
                    new("item", 100, "sha_item_1"),
                    new("skill", 200, "sha_skill_1"),
                    new("unit", 643, "sha_unit_1"),
                ]));

        // 1. Exact match -> "与登记的版本一致"
        var matched = KnownBuildsRegistry.Compare(
            [registeredEntry],
            "file",
            "aaaa1111bbbb2222",
            registeredEntry.Database);
        Assert.Equal(KnownBuildComparisonStatus.Matched, matched.StatusKind);
        Assert.Equal("与登记的版本一致", matched.Message);
        Assert.Empty(matched.TableDiffs);

        // 2. Unregistered game version -> "新的游戏版本，未登记"
        var newGame = KnownBuildsRegistry.Compare(
            [registeredEntry],
            "file",
            "cccc3333dddd4444",
            registeredEntry.Database);
        Assert.Equal(KnownBuildComparisonStatus.NewGameVersion, newGame.StatusKind);
        Assert.Equal("新的游戏版本，未登记", newGame.Message);
        Assert.Empty(newGame.TableDiffs);

        // 3. Same game version, different DB hash -> "游戏版本已登记，但数据库哈希不同" + differing tables
        var modifiedDb = registeredEntry.Database with
        {
            FileSha256 = "filehash_actual",
            ContentSha256 = "contenthash_actual",
            Tables =
            [
                new("item", 100, "sha_item_1"), // unchanged
                new("quest", 50, "sha_quest_1"), // added
                new("unit", 644, "sha_unit_2"), // changed (rows + sha256)
                // "skill" removed
            ],
        };
        var mismatch = KnownBuildsRegistry.Compare(
            [registeredEntry],
            "file",
            "aaaa1111bbbb2222",
            modifiedDb);
        Assert.Equal(KnownBuildComparisonStatus.HashMismatch, mismatch.StatusKind);
        Assert.Equal("游戏版本已登记，但数据库哈希不同", mismatch.Message);
        Assert.Equal(3, mismatch.TableDiffs.Length);

        var added = Assert.Single(mismatch.TableDiffs, d => d.Name == "quest");
        Assert.Equal("added", added.Kind);
        var removed = Assert.Single(mismatch.TableDiffs, d => d.Name == "skill");
        Assert.Equal("removed", removed.Kind);
        var changed = Assert.Single(mismatch.TableDiffs, d => d.Name == "unit");
        Assert.Equal("changed", changed.Kind);
        Assert.Equal(643, changed.ExpectedRows);
        Assert.Equal(644, changed.ActualRows);

        var printed = new StringWriter();
        KnownBuildsRegistry.PrintComparison(mismatch, printed);
        var output = printed.ToString();
        Assert.Contains("游戏版本已登记，但数据库哈希不同", output, StringComparison.Ordinal);
        Assert.Contains("quest", output, StringComparison.Ordinal);
        Assert.Contains("skill", output, StringComparison.Ordinal);
        Assert.Contains("unit", output, StringComparison.Ordinal);

        // 4. Load prefers adjacent known-builds.json, falls back to the embedded registry.
        var emptyDir = Path.Combine(workspace.Root, "empty-bin");
        Directory.CreateDirectory(emptyDir);
        var (embeddedEntries, embeddedSource) = KnownBuildsRegistry.Load(emptyDir);
        Assert.Equal("embedded", embeddedSource);
        Assert.NotNull(embeddedEntries);

        var lowerHex64Regex = new Regex("^[0-9a-f]{64}$", RegexOptions.CultureInvariant);
        foreach (var entry in embeddedEntries)
        {
            Assert.False(string.IsNullOrWhiteSpace(entry.RegisteredAt));
            Assert.NotNull(entry.GameVersion);
            Assert.NotNull(entry.GameVersion.Steam);
            Assert.False(string.IsNullOrWhiteSpace(entry.GameVersion.Steam.BuildId));
            Assert.NotNull(entry.Database);
            Assert.False(string.IsNullOrWhiteSpace(entry.Database.FileSha256));
            Assert.False(string.IsNullOrWhiteSpace(entry.Database.ContentSha256));
            Assert.Matches(lowerHex64Regex, entry.Database.FileSha256);
            Assert.Matches(lowerHex64Regex, entry.Database.ContentSha256);
        }

        var adjacentDir = Path.Combine(workspace.Root, "adjacent-bin");
        Directory.CreateDirectory(adjacentDir);
        File.WriteAllText(
            Path.Combine(adjacentDir, "known-builds.json"),
            JsonConvert.SerializeObject(new[] { registeredEntry }, Formatting.Indented));
        var (fileEntries, fileSource) = KnownBuildsRegistry.Load(adjacentDir);
        Assert.Equal("file", fileSource);
        Assert.Single(fileEntries);
        Assert.Equal("aaaa1111bbbb2222", fileEntries[0].GameVersion.PakFingerprint);
    }

    [Fact]
    public void FfrDbLocator_ResolvesPriorityAndReportsClearErrorWhenMissing()
    {
        using var workspace = new TempWorkspace("ffr-locator");
        var explicitBin = Path.Combine(workspace.Root, "explicit", "custom-ffr-db");
        var siblingDir = Path.Combine(workspace.Root, "sibling");
        var siblingBin = Path.Combine(siblingDir, "ffr-db");
        var pathDir = Path.Combine(workspace.Root, "path-dir");
        var pathBin = Path.Combine(pathDir, "ffr-db");

        Directory.CreateDirectory(Path.GetDirectoryName(explicitBin)!);
        Directory.CreateDirectory(siblingDir);
        Directory.CreateDirectory(pathDir);
        File.WriteAllText(explicitBin, "#!/bin/sh\n");
        File.WriteAllText(siblingBin, "#!/bin/sh\n");
        File.WriteAllText(pathBin, "#!/bin/sh\n");

        // 1. Explicit wins over sibling and PATH
        Assert.Equal(
            explicitBin,
            FfrDbLocator.Resolve(explicitBin, siblingDir, pathDir, isWindowsOverride: false));

        // 2. Sibling wins over PATH
        Assert.Equal(
            siblingBin,
            FfrDbLocator.Resolve(null, siblingDir, pathDir, isWindowsOverride: false));

        // 3. PATH used when sibling absent
        var emptySibling = Path.Combine(workspace.Root, "empty-sibling");
        Directory.CreateDirectory(emptySibling);
        Assert.Equal(
            pathBin,
            FfrDbLocator.Resolve(null, emptySibling, pathDir, isWindowsOverride: false));

        // 4. Windows lookup looks for ffr-db.exe
        var winSibling = Path.Combine(workspace.Root, "win-sibling");
        Directory.CreateDirectory(winSibling);
        var winExe = Path.Combine(winSibling, "ffr-db.exe");
        File.WriteAllText(winExe, "mz");
        Assert.Equal(
            winExe,
            FfrDbLocator.Resolve(null, winSibling, string.Empty, isWindowsOverride: true));

        // 5. Missing explicit path throws FileNotFoundException
        var missingExplicit = Assert.Throws<FileNotFoundException>(() =>
            FfrDbLocator.Resolve(Path.Combine(workspace.Root, "nope"), siblingDir, pathDir, isWindowsOverride: false));
        Assert.Contains("--ffr-db", missingExplicit.Message, StringComparison.Ordinal);

        // 6. Missing everywhere throws FileNotFoundException listing search order
        var missingAll = Assert.Throws<FileNotFoundException>(() =>
            FfrDbLocator.Resolve(null, emptySibling, string.Empty, isWindowsOverride: false));
        Assert.Contains("找不到 ffr-db 可执行文件", missingAll.Message, StringComparison.Ordinal);
        Assert.Contains("PATH", missingAll.Message, StringComparison.Ordinal);
    }

    [Fact]
    public void WindowsPathsAndManifest_HandleLongPathsUncAndCrossDrive()
    {
        // 1. Short Windows path untouched
        Assert.Equal(@"C:\Games\FFR", PathSupport.ApplyWindowsLongPathPrefix(@"C:\Games\FFR"));

        // 2. Long >260 char Windows path gets \\?\ prefix
        var longWinPath = @"D:\Output With Spaces\" + string.Join(@"\", Enumerable.Repeat("DeepSubDirectoryName", 15)) + @"\file.json";
        Assert.True(longWinPath.Length > 260);
        var prefixed = PathSupport.ApplyWindowsLongPathPrefix(longWinPath);
        Assert.Equal(@"\\?\" + longWinPath, prefixed);

        // Already prefixed is idempotent
        Assert.Equal(prefixed, PathSupport.ApplyWindowsLongPathPrefix(prefixed));

        // 3. Long UNC path gets \\?\UNC\ prefix
        var longUnc = @"\\fileserver\share\" + string.Join(@"\", Enumerable.Repeat("DeepSegmentName", 18));
        Assert.True(longUnc.Length > 260);
        Assert.Equal(@"\\?\UNC\" + longUnc[2..], PathSupport.ApplyWindowsLongPathPrefix(longUnc));

        // 4. Application manifest declares longPathAware=true and csproj wires it
        var repoRoot = TestHarness.FindRepoRoot();
        var manifest = File.ReadAllText(Path.Combine(repoRoot, "src", "FfrExport", "app.manifest"));
        Assert.Contains("<longPathAware", manifest, StringComparison.Ordinal);
        Assert.Contains(">true</longPathAware>", manifest, StringComparison.Ordinal);

        var csproj = File.ReadAllText(Path.Combine(repoRoot, "src", "FfrExport", "FfrExport.csproj"));
        Assert.Contains("<ApplicationManifest>app.manifest</ApplicationManifest>", csproj, StringComparison.Ordinal);
    }

    [Fact]
    public async Task BuildDb_EndToEndWithFakeFfrDb_SpacesLongPathsGuardrailsAndSteamSources()
    {
        using var workspace = new TempWorkspace("ffr-build-db-e2e");

        // Build a synthetic Steam layout with spaces in directory names.
        var steamappsDir = Path.Combine(workspace.Root, "Steam Library", "steamapps");
        var gameDir = Path.Combine(steamappsDir, "common", "FINAL FANTASY RESONANCE");
        var paksDir = Path.Combine(gameDir, "FFRS", "Content", "Paks");
        var binDir = Path.Combine(gameDir, "FFRS", "Binaries", "Win64");
        Directory.CreateDirectory(paksDir);
        Directory.CreateDirectory(binDir);

        // Synthetic appmanifest_3259780.acf
        var acfContent = """
            "AppState"
            {
            	"appid"		"3259780"
            	"Universe"		"1"
            	"name"		"FINAL FANTASY RESONANCE"
            	"LastUpdated"		"1761652000"
            	"buildid"		"24250053"
            	"UserConfig"
            	{
            		"buildid"		"99999999"
            	}
            }
            """;
        await File.WriteAllTextAsync(Path.Combine(steamappsDir, "appmanifest_3259780.acf"), acfContent);

        // Synthetic PE exe with version resource
        var peBytes = SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource(
            fileVersion: (1, 0, 0, 0),
            productVersion: (1, 0, 0, 0),
            stringFileVersion: "1.0.0.0",
            stringProductVersion: "1.0.0.0",
            sectionName: ".link",
            pe32Plus: true);
        await File.WriteAllBytesAsync(Path.Combine(binDir, "FFRS-Win64-Shipping.exe"), peBytes);

        // Synthetic Pak with valid AssetRegistry, 3 locres languages, and raw Config files (0 failing .uasset files)
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
        var iniBytes = Encoding.UTF8.GetBytes("[/Script/Test]\nKey=Value\n");

        var pakBytes = SyntheticFixtureBuilder.BuildPak([
            new("FFRS/AssetRegistry.bin", arBytes),
            new("FFRS/Content/Localization/Game/zh-Hans/Game.locres", zhLocres),
            new("FFRS/Content/Localization/Game/en/Game.locres", enLocres),
            new("FFRS/Content/Localization/Game/ja/Game.locres", jaLocres),
            new("FFRS/Config/DefaultGame.ini", iniBytes),
            new("FFRS/Content/Textures/T_Sample.uasset", [1, 2, 3, 4]),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pakBytes);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.ucas"), new byte[64]);

        var usmapPath = Path.Combine(workspace.Root, "Mappings Folder", "Fixture.usmap");
        Directory.CreateDirectory(Path.GetDirectoryName(usmapPath)!);
        var usmapBytes = SyntheticFixtureBuilder.BuildUsmapV0(
            ["EDemo", "Val0", "FDemoRow", "Id"],
            [new("EDemo", ["Val0"])],
            [new("FDemoRow", null, 1, [new(0, 1, "Id", 2)])]);
        await File.WriteAllBytesAsync(usmapPath, usmapBytes);

        var jmapPath = Path.Combine(workspace.Root, "Mappings Folder", "Fixture.jmap");
        await File.WriteAllTextAsync(jmapPath, "{\"classes\":[]}\n");

        // Create fake ffr-db script in a path with spaces
        var fakeDbScript = CreateFakeFfrDbScript(Path.Combine(workspace.Root, "Tools With Spaces"));

        // 1. Run build-db without --keep-json
        var outDir1 = Path.Combine(workspace.Root, "Output With Spaces", "run1");
        var res1 = await TestHarness.RunCliAsync([
            "build-db",
            "--game", gameDir,
            "--usmap", usmapPath,
            "--jmap", jmapPath,
            "--out", outDir1,
            "--ffr-db", fakeDbScript,
            "--jobs", "2",
        ]);
        Assert.True(res1.ExitCode == 0, $"stdout:\n{res1.Stdout}\nstderr:\n{res1.Stderr}");
        Assert.Contains("新的游戏版本，未登记", res1.Stdout, StringComparison.Ordinal);

        Assert.True(File.Exists(Path.Combine(outDir1, "ffr.sqlite")));
        Assert.False(Directory.Exists(Path.Combine(outDir1, "json")), "不加 --keep-json 时应删除 json 目录");

        var buildMeta1 = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir1, "ffr.sqlite.build.json")));
        Assert.Equal("build-db", buildMeta1["command"]?.Value<string>());
        Assert.Equal("completed", buildMeta1["status"]?.Value<string>());
        Assert.Equal("24250053", buildMeta1["game_version"]?["steam"]?["build_id"]?.Value<string>());
        Assert.Equal("appmanifest", buildMeta1["game_version"]?["steam"]?["source"]?.Value<string>());
        Assert.Equal("FINAL FANTASY RESONANCE", buildMeta1["game_version"]?["steam"]?["name"]?.Value<string>());
        Assert.Equal("1761652000", buildMeta1["game_version"]?["steam"]?["last_updated"]?.Value<string>());
        Assert.Equal("1.0.0.0", buildMeta1["game_version"]?["exe"]?["file_version"]?.Value<string>());
        Assert.Equal("1.0.0.0", buildMeta1["game_version"]?["exe"]?["product_version"]?.Value<string>());
        Assert.False(string.IsNullOrWhiteSpace(buildMeta1["game_version"]?["pak_fingerprint"]?.Value<string>()));
        Assert.Equal(2, Assert.IsType<JArray>(buildMeta1["game_version"]?["pak_files"]).Count);
        Assert.Equal(Convert.ToHexStringLower(SHA256.HashData(usmapBytes)), buildMeta1["inputs"]?["usmap"]?["file_sha256"]?.Value<string>());
        Assert.False(string.IsNullOrWhiteSpace(buildMeta1["inputs"]?["usmap"]?["content_sha256"]?.Value<string>()));
        Assert.Single(Assert.IsType<JArray>(buildMeta1["inputs"]?["jmaps"]));
        Assert.Equal("ffr-db 0.1.0-fake", buildMeta1["tools"]?["ffr_db"]?["version"]?.Value<string>());
        Assert.Equal("deadbeef00000000000000000000000000000000000000000000000000000001", buildMeta1["database"]?["content_sha256"]?.Value<string>());
        Assert.Equal(643, buildMeta1["database"]?["tables"]?[0]?["rows"]?.Value<long>());
        Assert.Equal("new_game_version", buildMeta1["known_build_check"]?["status"]?.Value<string>());

        // 2. Non-empty own build-db output dir refused without --force, allowed with --force
        var refuseOwn = await TestHarness.RunCliAsync([
            "build-db", "--game", gameDir, "--usmap", usmapPath, "--out", outDir1, "--ffr-db", fakeDbScript,
        ]);
        Assert.Equal(2, refuseOwn.ExitCode);
        Assert.Contains("--force", refuseOwn.Stderr, StringComparison.Ordinal);

        var forceOwn = await TestHarness.RunCliAsync([
            "build-db", "--game", gameDir, "--usmap", usmapPath, "--out", outDir1, "--ffr-db", fakeDbScript, "--force",
        ]);
        Assert.Equal(0, forceOwn.ExitCode);

        // 3. Foreign non-empty output dir refused even WITH --force
        var foreignDir = Path.Combine(workspace.Root, "foreign-dir");
        Directory.CreateDirectory(foreignDir);
        await File.WriteAllTextAsync(Path.Combine(foreignDir, "user-notes.txt"), "keep-me");
        var refuseForeign = await TestHarness.RunCliAsync([
            "build-db", "--game", gameDir, "--usmap", usmapPath, "--out", foreignDir, "--ffr-db", fakeDbScript, "--force",
        ]);
        Assert.Equal(2, refuseForeign.ExitCode);
        Assert.Contains("不是 ffr-export build-db 的输出目录", refuseForeign.Stderr, StringComparison.Ordinal);
        Assert.Equal("keep-me", await File.ReadAllTextAsync(Path.Combine(foreignDir, "user-notes.txt")));

        // 4. --out inside --game is refused
        var insideGameOut = Path.Combine(gameDir, "nested-out");
        var refuseOverlap = await TestHarness.RunCliAsync([
            "build-db", "--game", gameDir, "--usmap", usmapPath, "--out", insideGameOut, "--ffr-db", fakeDbScript,
        ]);
        Assert.Equal(2, refuseOverlap.ExitCode);
        Assert.Contains("不能与游戏目录重叠", refuseOverlap.Stderr, StringComparison.Ordinal);

        // 5. --keep-json + deep >260 char output path + manual --steam-buildid (when appmanifest absent)
        File.Delete(Path.Combine(steamappsDir, "appmanifest_3259780.acf"));
        var deepOutDir = Path.Combine(
            workspace.Root,
            "deep_output_root",
            string.Join(Path.DirectorySeparatorChar, Enumerable.Repeat("segment_over_twenty_chars_long", 9)));
        Assert.True(deepOutDir.Length > 260, $"测试路径应超过 260 字符，当前长度：{deepOutDir.Length}");

        var resManual = await TestHarness.RunCliAsync([
            "build-db",
            "--game", gameDir,
            "--usmap", usmapPath,
            "--out", deepOutDir,
            "--ffr-db", fakeDbScript,
            "--steam-buildid", "98765432",
            "--keep-json",
        ]);
        Assert.True(resManual.ExitCode == 0, $"stdout:\n{resManual.Stdout}\nstderr:\n{resManual.Stderr}");
        Assert.True(File.Exists(PathSupport.ToIoPath(Path.Combine(deepOutDir, "json", "export", "manifest.json"))));
        Assert.True(File.Exists(PathSupport.ToIoPath(Path.Combine(deepOutDir, "json", "export", "Localization", "zh-Hans.json"))));
        Assert.True(File.Exists(PathSupport.ToIoPath(Path.Combine(deepOutDir, "json", "inventory", "registry.jsonl"))));
        Assert.True(File.Exists(PathSupport.ToIoPath(Path.Combine(deepOutDir, "json", "export-full", "FFRS", "Config", "DefaultGame.ini"))));
        Assert.True(File.Exists(PathSupport.ToIoPath(Path.Combine(deepOutDir, "json", "export-full", "maps", "_graph.json"))));

        var buildMetaManual = JObject.Parse(await File.ReadAllTextAsync(PathSupport.ToIoPath(Path.Combine(deepOutDir, "ffr.sqlite.build.json"))));
        Assert.Equal("98765432", buildMetaManual["game_version"]?["steam"]?["build_id"]?.Value<string>());
        Assert.Equal("manual", buildMetaManual["game_version"]?["steam"]?["source"]?.Value<string>());

        // 6. Missing both appmanifest and --steam-buildid -> warning, build_id=null, does not abort
        var outDirNullSteam = Path.Combine(workspace.Root, "run-null-steam");
        var resNullSteam = await TestHarness.RunCliAsync([
            "build-db",
            "--game", gameDir,
            "--usmap", usmapPath,
            "--out", outDirNullSteam,
            "--ffr-db", fakeDbScript,
        ]);
        Assert.Equal(0, resNullSteam.ExitCode);
        Assert.Contains("警告", resNullSteam.Stderr, StringComparison.Ordinal);
        var buildMetaNull = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDirNullSteam, "ffr.sqlite.build.json")));
        Assert.Equal(JTokenType.Null, buildMetaNull["game_version"]?["steam"]?["build_id"]?.Type);
        Assert.Equal(JTokenType.Null, buildMetaNull["game_version"]?["steam"]?["source"]?.Type);
    }

    [Fact]
    public async Task BuildDb_ErrorsClearlyOnInvalidGameDirectoryAndMissingFfrDb()
    {
        using var workspace = new TempWorkspace("ffr-build-db-errors");
        var usmapPath = Path.Combine(workspace.Root, "Empty.usmap");
        await File.WriteAllBytesAsync(usmapPath, SyntheticFixtureBuilder.BuildUsmap());

        // 1. Missing Paks directory
        var badGameNoPaks = Path.Combine(workspace.Root, "bad-game-1");
        Directory.CreateDirectory(badGameNoPaks);
        var resNoPaks = await TestHarness.RunCliAsync([
            "build-db", "--game", badGameNoPaks, "--usmap", usmapPath,
        ]);
        Assert.Equal(2, resNoPaks.ExitCode);
        Assert.Contains("FFRS/Content/Paks", resNoPaks.Stderr, StringComparison.Ordinal);

        // 2. Missing FFRS-Win64-Shipping.exe
        var badGameNoExe = Path.Combine(workspace.Root, "bad-game-2");
        Directory.CreateDirectory(Path.Combine(badGameNoExe, "FFRS", "Content", "Paks"));
        var resNoExe = await TestHarness.RunCliAsync([
            "build-db", "--game", badGameNoExe, "--usmap", usmapPath,
        ]);
        Assert.Equal(2, resNoExe.ExitCode);
        Assert.Contains("FFRS-Win64-Shipping.exe", resNoExe.Stderr, StringComparison.Ordinal);

        // 3. Valid game layout but missing --ffr-db executable
        Directory.CreateDirectory(Path.Combine(badGameNoExe, "FFRS", "Binaries", "Win64"));
        await File.WriteAllBytesAsync(
            Path.Combine(badGameNoExe, "FFRS", "Binaries", "Win64", "FFRS-Win64-Shipping.exe"),
            SyntheticFixtureBuilder.BuildMinimalPeWithVersionResource((1, 0, 0, 0), (1, 0, 0, 0)));

        var resMissingFfrDb = await TestHarness.RunCliAsync([
            "build-db",
            "--game", badGameNoExe,
            "--usmap", usmapPath,
            "--ffr-db", Path.Combine(workspace.Root, "nonexistent-ffr-db"),
        ]);
        Assert.Equal(2, resMissingFfrDb.ExitCode);
        Assert.Contains("--ffr-db", resMissingFfrDb.Stderr, StringComparison.Ordinal);

        // 4. Missing ffr-db everywhere (no --ffr-db and empty PATH) reports Chinese error
        var resNoPathFfrDb = await TestHarness.RunCliAsync(
            ["build-db", "--game", badGameNoExe, "--usmap", usmapPath],
            pathEnvOverride: string.Empty);
        Assert.Equal(2, resNoPathFfrDb.ExitCode);
        Assert.Contains("找不到 ffr-db 可执行文件", resNoPathFfrDb.Stderr, StringComparison.Ordinal);

        // 5. Trailing separator in --game still locates steamapps/appmanifest_3259780.acf
        var trailingSteamapps = Path.Combine(workspace.Root, "trailing-steam", "steamapps");
        var trailingGame = Path.Combine(trailingSteamapps, "common", "FFR");
        Directory.CreateDirectory(trailingGame);
        await File.WriteAllTextAsync(
            Path.Combine(trailingSteamapps, "appmanifest_3259780.acf"),
            "\"AppState\" { \"buildid\" \"555123\" \"name\" \"FFR\" \"LastUpdated\" \"1700000000\" }");
        var steamTrailing = GameVersionDetector.ReadSteamBuildInfo(trailingGame + Path.DirectorySeparatorChar, null);
        Assert.Equal("555123", steamTrailing.BuildId);
        Assert.Equal("appmanifest", steamTrailing.Source);
    }

    private static string CreateFakeFfrDbScript(string directory)
    {
        var sourceDir = Path.Combine(AppContext.BaseDirectory, "fake-ffr-db");
        if (!Directory.Exists(sourceDir))
        {
            throw new DirectoryNotFoundException($"假 ffr-db 测试辅助产物目录不存在：{sourceDir}");
        }

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
        if (!File.Exists(exePath))
        {
            throw new FileNotFoundException($"假 ffr-db 可执行文件不存在：{exePath}");
        }

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
