using System.Security.Cryptography;
using System.Text;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class SubcommandsSmokeTests
{
    [Fact]
    public async Task ListAndRaw_WorkOnSyntheticPakAndEnforceOutputGuardrails()
    {
        using var workspace = new TempWorkspace("ffr-list-raw");
        var paksDir = Path.Combine(workspace.Root, "paks");
        Directory.CreateDirectory(paksDir);

        var dtBytes = new byte[] { 0x10, 0x20, 0x30, 0x40, 0x50 };
        var l10nBytes = Encoding.UTF8.GetBytes("synthetic-l10n-bytes");
        var otherBytes = Encoding.UTF8.GetBytes("other-file");

        var pak = SyntheticFixtureBuilder.BuildPak([
            new PakFileEntry("FFRS/Content/Datatable/DT_Demo.uasset", dtBytes),
            new PakFileEntry("FFRS/Content/L10N/zh-Hans/T_Ui.uasset", l10nBytes),
            new PakFileEntry("FFRS/Config/DefaultGame.ini", otherBytes),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pak);

        // 1. `list` with default prefixes (Datatable + L10N)
        var listDefault = await TestHarness.RunCliAsync(
            ["list", "--paks", paksDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, listDefault.ExitCode);
        Assert.Contains("FFRS/Content/Datatable/DT_Demo.uasset\t5 字节", listDefault.Stdout, StringComparison.Ordinal);
        Assert.Contains("FFRS/Content/L10N/zh-Hans/T_Ui.uasset\t20 字节", listDefault.Stdout, StringComparison.Ordinal);
        Assert.DoesNotContain("DefaultGame.ini", listDefault.Stdout, StringComparison.Ordinal);
        Assert.Contains("总数：2", listDefault.Stdout, StringComparison.Ordinal);

        // 2. `list` with custom prefix
        var listCustom = await TestHarness.RunCliAsync(
            ["list", "--paks", paksDir, "--prefix", "FFRS/Config/"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, listCustom.ExitCode);
        Assert.Contains("FFRS/Config/DefaultGame.ini", listCustom.Stdout, StringComparison.Ordinal);
        Assert.Contains("总数：1", listCustom.Stdout, StringComparison.Ordinal);

        // 3. `raw` extracts exact bytes and reports length + SHA-256
        var rawOut = Path.Combine(workspace.DepsDir, "tmp", "DT_Demo.bin");
        var rawResult = await TestHarness.RunCliAsync(
            ["raw", "FFRS/Content/Datatable/DT_Demo.uasset", "--paks", paksDir, "--out", rawOut],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, rawResult.ExitCode);
        Assert.Equal(dtBytes, await File.ReadAllBytesAsync(rawOut));
        Assert.Contains("长度：5 字节", rawResult.Stdout, StringComparison.Ordinal);
        Assert.Contains($"SHA-256：{Convert.ToHexString(SHA256.HashData(dtBytes))}", rawResult.Stdout, StringComparison.Ordinal);

        // 4. `raw` rejects missing package and out-of-deps output path
        var missingResult = await TestHarness.RunCliAsync(
            ["raw", "FFRS/Content/Missing.uasset", "--paks", paksDir, "--out", rawOut],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, missingResult.ExitCode);
        Assert.Contains("找不到或无法读取包", missingResult.Stderr, StringComparison.Ordinal);

        var outsideOut = Path.Combine(workspace.Root, "outside.bin");
        var outsideResult = await TestHarness.RunCliAsync(
            ["raw", "FFRS/Content/Datatable/DT_Demo.uasset", "--paks", paksDir, "--out", outsideOut],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, outsideResult.ExitCode);
        Assert.Contains("FFR_DEPS", outsideResult.Stderr, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Locres_ExportsSyntheticLocalizationAndEnforcesManifestGuards()
    {
        using var workspace = new TempWorkspace("ffr-locres");
        var paksDir = Path.Combine(workspace.DepsDir, "ffr", "files", "FFRS", "Content", "Paks");
        Directory.CreateDirectory(paksDir);

        var zhLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string>
            {
                ["Title"] = "示例标题",
                ["Start"] = "开始",
            },
        });
        var enLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string>
            {
                ["Title"] = "Sample Title",
                ["Start"] = "Start",
            },
        });
        var jaLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string>
            {
                ["Title"] = "サンプルタイトル",
                ["Start"] = "スタート",
            },
        });
        var frLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string>
            {
                ["Title"] = "Titre exemple",
            },
        });

        var pak = SyntheticFixtureBuilder.BuildPak([
            new PakFileEntry("FFRS/Content/Localization/Game/zh-Hans/Game.locres", zhLocres),
            new PakFileEntry("FFRS/Content/Localization/Game/en/Game.locres", enLocres),
            new PakFileEntry("FFRS/Content/Localization/Game/ja/Game.locres", jaLocres),
            new PakFileEntry("FFRS/Content/Localization/Game/fr/Game.locres", frLocres),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pak);

        // 1. Default invocation uses FFR_DEPS for both paks input and Localization output
        var defaultRun = await TestHarness.RunCliAsync(["locres"], ffrDeps: workspace.DepsDir);
        Assert.Equal(0, defaultRun.ExitCode);

        var defaultOutDir = Path.Combine(workspace.DepsDir, "ffr", "export", "Localization");
        var zhJson = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(defaultOutDir, "zh-Hans.json")));
        var enJson = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(defaultOutDir, "en.json")));
        var jaJson = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(defaultOutDir, "ja.json")));
        Assert.Equal("示例标题", zhJson["ST_Sample"]?["Title"]?.Value<string>());
        Assert.Equal("Sample Title", enJson["ST_Sample"]?["Title"]?.Value<string>());
        Assert.Equal("サンプルタイトル", jaJson["ST_Sample"]?["Title"]?.Value<string>());

        var manifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(defaultOutDir, "manifest.json")));
        Assert.Equal("locres", manifest["command"]?.Value<string>());
        Assert.Equal("1.2.2.202611", manifest["cue4ParseVersion"]?.Value<string>());
        var languages = Assert.IsType<JArray>(manifest["languages"]);
        Assert.Equal(3, languages.Count);
        Assert.Equal(Convert.ToHexString(SHA256.HashData(zhLocres)), languages[0]["sourceSha256"]?.Value<string>());
        Assert.Equal(2, languages[0]["entryCount"]?.Value<int>());

        // 2. Non-empty output directory refused without --force
        var refuseNonEmpty = await TestHarness.RunCliAsync(["locres"], ffrDeps: workspace.DepsDir);
        Assert.Equal(2, refuseNonEmpty.ExitCode);
        Assert.Contains("输出目录非空", refuseNonEmpty.Stderr, StringComparison.Ordinal);

        // 3. --force succeeds on own locres manifest with custom comma-separated --lang
        var forceRun = await TestHarness.RunCliAsync(
            ["locres", "--lang", "zh-Hans,fr", "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, forceRun.ExitCode);
        Assert.True(File.Exists(Path.Combine(defaultOutDir, "fr.json")));
        Assert.False(File.Exists(Path.Combine(defaultOutDir, "en.json")));

        // 4. Missing language reports available languages
        var missingLang = await TestHarness.RunCliAsync(
            ["locres", "--lang", "de", "--out", Path.Combine(workspace.DepsDir, "tmp", "locres-de")],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, missingLang.ExitCode);
        Assert.Contains("包内可用语言", missingLang.Stderr, StringComparison.Ordinal);
        Assert.Contains("zh-Hans", missingLang.Stderr, StringComparison.Ordinal);

        // 5. --force refuses a directory containing an `export` manifest
        var exportDir = Path.Combine(workspace.DepsDir, "tmp", "export-dir");
        Directory.CreateDirectory(exportDir);
        await File.WriteAllTextAsync(
            Path.Combine(exportDir, "manifest.json"),
            JsonConvert.SerializeObject(new { cue4ParseVersion = "1.2.2.202611", successCount = 1 }));
        var wrongCommand = await TestHarness.RunCliAsync(
            ["locres", "--out", exportDir, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, wrongCommand.ExitCode);
        Assert.Contains("不能由 locres --force 清空", wrongCommand.Stderr, StringComparison.Ordinal);
    }

    [Fact]
    public async Task Export_RecordsSingleFileFailuresWithoutAbortingAndEnforcesGuardrails()
    {
        using var workspace = new TempWorkspace("ffr-export");
        var paksDir = Path.Combine(workspace.Root, "paks");
        Directory.CreateDirectory(paksDir);

        var pak = SyntheticFixtureBuilder.BuildPak([
            new PakFileEntry("FFRS/Content/Datatable/DT_Bad1.uasset", [1, 2, 3, 4]),
            new PakFileEntry("FFRS/Content/Datatable/DT_Bad2.uasset", [5, 6, 7, 8]),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pak);

        var usmapBytes = SyntheticFixtureBuilder.BuildUsmap();
        var usmapPath = Path.Combine(workspace.Root, "Fixture.usmap");
        await File.WriteAllBytesAsync(usmapPath, usmapBytes);
        var expectedSha = Convert.ToHexString(SHA256.HashData(usmapBytes));

        // 1. Missing --usmap fails with exit code 2
        var missingUsmap = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, missingUsmap.ExitCode);
        Assert.Contains("--usmap", missingUsmap.Stderr, StringComparison.Ordinal);

        // 2. Corrupted dummy .uasset files fail individually without aborting, writing manifest with exit code 1
        var outDir = Path.Combine(workspace.DepsDir, "ffr", "export");
        var exportResult = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir, "--usmap", usmapPath, "--prefix", "FFRS/Content/Datatable/"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, exportResult.ExitCode);

        var manifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "manifest.json")));
        Assert.Equal("1.2.2.202611", manifest["cue4ParseVersion"]?.Value<string>());
        Assert.Equal(expectedSha, manifest["usmapSha256"]?.Value<string>());
        Assert.Equal(0, manifest["successCount"]?.Value<int>());
        Assert.Equal(2, manifest["failureCount"]?.Value<int>());
        var failures = Assert.IsType<JArray>(manifest["failures"]);
        Assert.Equal(2, failures.Count);
        Assert.Equal("FFRS/Content/Datatable/DT_Bad1.uasset", failures[0]["path"]?.Value<string>());
        Assert.Equal("FFRS/Content/Datatable/DT_Bad2.uasset", failures[1]["path"]?.Value<string>());

        // 3. Re-running without --force fails because directory is non-empty
        var nonEmpty = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir, "--usmap", usmapPath, "--prefix", "FFRS/Content/Datatable/"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, nonEmpty.ExitCode);
        Assert.Contains("输出目录非空", nonEmpty.Stderr, StringComparison.Ordinal);

        // 4. --force clears own export directory and re-runs
        var forced = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir, "--usmap", usmapPath, "--prefix", "FFRS/Content/Datatable/", "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, forced.ExitCode);
        Assert.Contains("即将清空输出目录", forced.Stdout, StringComparison.Ordinal);

        // 5. --force refuses foreign non-empty directory
        var foreignDir = Path.Combine(workspace.DepsDir, "tmp", "foreign");
        Directory.CreateDirectory(foreignDir);
        await File.WriteAllTextAsync(Path.Combine(foreignDir, "keep.txt"), "safe");
        var refuseForeign = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir, "--usmap", usmapPath, "--out", foreignDir, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, refuseForeign.ExitCode);
        Assert.Contains("不是 ffr-export 的输出目录", refuseForeign.Stderr, StringComparison.Ordinal);
        Assert.Equal("safe", await File.ReadAllTextAsync(Path.Combine(foreignDir, "keep.txt")));
    }

    [Fact]
    public async Task InventoryAndExportAssets_SyntheticPakSmokeTest()
    {
        using var workspace = new TempWorkspace("ffr-inventory-export-assets");
        var paksDir = Path.Combine(workspace.Root, "paks");
        var invOutDir = Path.Combine(workspace.DepsDir, "ffr", "inventory");
        var exportOutDir = Path.Combine(workspace.DepsDir, "ffr", "export-full");
        var usmapPath = Path.Combine(workspace.Root, "Fixture.usmap");
        Directory.CreateDirectory(paksDir);

        FixtureAsset[] assets =
        [
            new("/Game/Datatable/DT_Test", "/Game/Datatable", "DT_Test", "DataTable",
                new Dictionary<string, string> { ["RowStructure"] = "/Script/Test.TestRow" }),
            new("/Game/Textures/T_Sample", "/Game/Textures", "T_Sample", "Texture2D"),
            new("/Game/Mesh/SM_Rock", "/Game/Mesh", "SM_Rock", "StaticMesh"),
            new("/Game/Maps/L_Demo", "/Game/Maps", "L_Demo", "World"),
            new("/Game/Maps/L_Demo", "/Game/Maps", "L_Demo_C", "BlueprintGeneratedClass"),
            new("/Game/BP/BP_Hero", "/Game/BP", "BP_Hero", "Blueprint",
                new Dictionary<string, string> { ["BlueprintType"] = "BPTYPE_Normal" }),
            new("/Game/BP/BP_Hero", "/Game/BP", "BP_Hero_C", "BlueprintGeneratedClass",
                new Dictionary<string, string> { ["NativeParentClass"] = "/Script/mucha.CPP_MapUnit_Player" }),
            new("/Game/Sequencer/SEQ_Intro", "/Game/Sequencer", "SEQ_Intro", "LevelSequence"),
            new("/Game/Sequencer/SEQ_Intro", "/Game/Sequencer", "SEQ_Intro_DirectorBP_C", "BlueprintGeneratedClass"),
            new("/Game/Env/PM_Floor", "/Game/Env", "PM_Floor", "CPP_MapPhysicalMaterial"),
            new("/SamplePlugin/Data/DA_PluginConfig", "/SamplePlugin/Data", "DA_PluginConfig", "CPP_DA_SampleConfig"),
        ];

        var arBytes = SyntheticFixtureBuilder.BuildAssetRegistry(assets);
        var defaultGameIni = Encoding.UTF8.GetBytes("[/Script/Test.Settings]\nMoveSpeed=600\n");
        var defaultTagsIni = Encoding.UTF8.GetBytes("[/Script/GameplayTags.GameplayTagsSettings]\n+GameplayTagList=(Tag=\"UI.Menu\")\n");
        var uprojectBytes = Encoding.UTF8.GetBytes("{\"FileVersion\":3,\"EngineAssociation\":\"5.6\"}\n");
        var upluginBytes = Encoding.UTF8.GetBytes("{\"FileVersion\":3,\"FriendlyName\":\"SamplePlugin\"}\n");

        var bulkBytes = new byte[64];
        Array.Fill(bulkBytes, (byte)0xAA);
        var meshBytes = new byte[32];
        Array.Fill(meshBytes, (byte)0xBB);
        var mapBytes = new byte[48];
        Array.Fill(mapBytes, (byte)0xCC);

        var pakBytes = SyntheticFixtureBuilder.BuildPak([
            new("FFRS/AssetRegistry.bin", arBytes),
            new("FFRS/Config/DefaultGame.ini", defaultGameIni),
            new("FFRS/Config/DefaultGameplayTags.ini", defaultTagsIni),
            new("FFRS/FFRS.uproject", uprojectBytes),
            new("FFRS/Plugins/SamplePlugin/SamplePlugin.uplugin", upluginBytes),
            new("FFRS/Content/Datatable/DT_Test.uasset", [1, 2, 3, 4]),
            new("FFRS/Content/Textures/T_Sample.uasset", [5, 6, 7, 8, 9]),
            new("FFRS/Content/Textures/T_Sample.ubulk", bulkBytes),
            new("FFRS/Content/Mesh/SM_Rock.uasset", meshBytes),
            new("FFRS/Content/Maps/L_Demo.umap", mapBytes),
            new("FFRS/Content/BP/BP_Hero.uasset", [10, 11, 12]),
            new("FFRS/Content/Sequencer/SEQ_Intro.uasset", [13, 14, 15]),
            new("FFRS/Content/Env/PM_Floor.uasset", [16, 17]),
            new("FFRS/Plugins/SamplePlugin/Content/Data/DA_PluginConfig.uasset", [18, 19]),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pakBytes);

        var usmapBytes = SyntheticFixtureBuilder.BuildUsmap();
        var expectedUsmapSha = Convert.ToHexString(SHA256.HashData(usmapBytes));
        await File.WriteAllBytesAsync(usmapPath, usmapBytes);

        // 1. Run `inventory` (using default output path under FFR_DEPS)
        var invResult = await TestHarness.RunCliAsync(
            ["inventory", "--paks", paksDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, invResult.ExitCode);

        var jsonlLines = (await File.ReadAllLinesAsync(Path.Combine(invOutDir, "registry.jsonl")))
            .Where(line => !string.IsNullOrWhiteSpace(line))
            .Select(JObject.Parse)
            .ToArray();
        Assert.Equal(8, jsonlLines.Length);

        var byPath = jsonlLines.ToDictionary(row => row["path"]!.Value<string>()!, StringComparer.Ordinal);

        var dtRow = byPath["FFRS/Content/Datatable/DT_Test.uasset"];
        Assert.Equal("DataTable", dtRow["primaryClass"]?.Value<string>());
        Assert.Equal(4, dtRow["size"]?.Value<long>());
        Assert.Equal(JTokenType.Null, dtRow["dependencyCount"]?.Type);
        Assert.Equal("/Script/Test.TestRow", dtRow["tags"]?["RowStructure"]?.Value<string>());

        var texRow = byPath["FFRS/Content/Textures/T_Sample.uasset"];
        Assert.Equal("Texture2D", texRow["primaryClass"]?.Value<string>());
        Assert.Equal(5, texRow["packageSize"]?.Value<long>());
        Assert.Equal(64, texRow["bulkSize"]?.Value<long>());
        Assert.Equal(69, texRow["size"]?.Value<long>());
        Assert.Equal("skip_image", texRow["decision"]?.Value<string>());

        var mapRow = byPath["FFRS/Content/Maps/L_Demo.umap"];
        Assert.Equal("World", mapRow["primaryClass"]?.Value<string>());
        Assert.Equal("skip_level", mapRow["decision"]?.Value<string>());

        var seqRow = byPath["FFRS/Content/Sequencer/SEQ_Intro.uasset"];
        Assert.Equal("LevelSequence", seqRow["primaryClass"]?.Value<string>());

        var bpRow = byPath["FFRS/Content/BP/BP_Hero.uasset"];
        Assert.Equal("BlueprintGeneratedClass", bpRow["primaryClass"]?.Value<string>());
        Assert.Equal("export_blueprint_cdo", bpRow["decision"]?.Value<string>());
        Assert.Equal("BPTYPE_Normal", bpRow["tags"]?["BlueprintType"]?.Value<string>());
        Assert.Equal("/Script/mucha.CPP_MapUnit_Player", bpRow["tags"]?["NativeParentClass"]?.Value<string>());

        var pluginRow = byPath["FFRS/Plugins/SamplePlugin/Content/Data/DA_PluginConfig.uasset"];
        Assert.Equal("export", pluginRow["decision"]?.Value<string>());

        var summary = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(invOutDir, "summary.json")));
        Assert.Equal(8, summary["assetRegistry"]?["packageCount"]?.Value<int>());
        Assert.Equal(8, summary["assetRegistry"]?["matchedVfsPackages"]?.Value<int>());
        Assert.Equal(0, summary["assetRegistry"]?["unmatchedRegistryPackages"]?.Value<int>());
        Assert.Equal(false, summary["assetRegistry"]?["dependencyGraphAvailable"]?.Value<bool>());

        var invManifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(invOutDir, "manifest.json")));
        Assert.Equal("inventory", invManifest["command"]?.Value<string>());
        Assert.Equal(8, invManifest["packageCount"]?.Value<int>());

        // 2. Run `export-assets` (default output path under FFR_DEPS)
        var exportResult = await TestHarness.RunCliAsync(
            ["export-assets", "--paks", paksDir, "--usmap", usmapPath, "--jobs", "2"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, exportResult.ExitCode);

        Assert.Equal(defaultGameIni, await File.ReadAllBytesAsync(Path.Combine(exportOutDir, "FFRS", "Config", "DefaultGame.ini")));
        Assert.Equal(defaultTagsIni, await File.ReadAllBytesAsync(Path.Combine(exportOutDir, "FFRS", "Config", "DefaultGameplayTags.ini")));
        Assert.Equal(uprojectBytes, await File.ReadAllBytesAsync(Path.Combine(exportOutDir, "FFRS", "FFRS.uproject")));
        Assert.Equal(upluginBytes, await File.ReadAllBytesAsync(Path.Combine(exportOutDir, "FFRS", "Plugins", "SamplePlugin", "SamplePlugin.uplugin")));

        var exportManifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(exportOutDir, "manifest.json")));
        Assert.Equal("export-assets", exportManifest["command"]?.Value<string>());
        Assert.Equal("completed", exportManifest["status"]?.Value<string>());
        Assert.Equal(expectedUsmapSha, exportManifest["usmapSha256"]?.Value<string>());
        Assert.Equal(8, exportManifest["totals"]?["consideredPackages"]?.Value<int>());
        Assert.Equal(3, exportManifest["totals"]?["skippedPackages"]?.Value<int>());
        Assert.Equal(1, exportManifest["totals"]?["undecidedPackages"]?.Value<int>());
        Assert.Equal(4, exportManifest["totals"]?["failedPackages"]?.Value<int>());
        Assert.Equal(4, exportManifest["totals"]?["rawCopiedFiles"]?.Value<int>());
        Assert.Equal(4, exportManifest["totals"]?["rawNewlyCopiedFiles"]?.Value<int>());
        Assert.Equal(4, Assert.IsType<JArray>(exportManifest["failures"]).Count);

        // 3. Resume without --force skips all 4 copied raw files
        var resumeResult = await TestHarness.RunCliAsync(
            ["export-assets", "--paks", paksDir, "--usmap", usmapPath],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, resumeResult.ExitCode);
        var resumeManifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(exportOutDir, "manifest.json")));
        Assert.Equal(4, resumeManifest["totals"]?["rawResumedFiles"]?.Value<int>());
        Assert.Equal(0, resumeManifest["totals"]?["rawNewlyCopiedFiles"]?.Value<int>());

        // 4. --force preserves sibling maps/ subdirectory carrying export-maps manifest
        var siblingMapsDir = Path.Combine(exportOutDir, "maps");
        Directory.CreateDirectory(siblingMapsDir);
        var siblingManifestContent = JsonConvert.SerializeObject(new
        {
            cue4ParseVersion = "1.2.2.202611",
            command = "export-maps",
        });
        await File.WriteAllTextAsync(Path.Combine(siblingMapsDir, "manifest.json"), siblingManifestContent);

        var forceResult = await TestHarness.RunCliAsync(
            ["export-assets", "--paks", paksDir, "--usmap", usmapPath, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, forceResult.ExitCode);
        Assert.Equal(siblingManifestContent, await File.ReadAllTextAsync(Path.Combine(siblingMapsDir, "manifest.json")));
        var forceManifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(exportOutDir, "manifest.json")));
        Assert.Equal(4, forceManifest["totals"]?["rawNewlyCopiedFiles"]?.Value<int>());

        // 5. Refuse non-empty unrelated directory
        var unrelatedDir = Path.Combine(workspace.DepsDir, "unrelated");
        Directory.CreateDirectory(unrelatedDir);
        await File.WriteAllTextAsync(Path.Combine(unrelatedDir, "keep.txt"), "do not touch");
        var refuseResult = await TestHarness.RunCliAsync(
            ["export-assets", "--paks", paksDir, "--usmap", usmapPath, "--out", unrelatedDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, refuseResult.ExitCode);
        Assert.Equal("do not touch", await File.ReadAllTextAsync(Path.Combine(unrelatedDir, "keep.txt")));
    }

    [Fact]
    public async Task DepsGuardrails_WorkWithAndWithoutFfrDepsEnvVar()
    {
        using var workspace = new TempWorkspace("ffr-deps-guard");

        // Without FFR_DEPS (cwd = workspace.Root where .deps exists): rejects outside .deps and protected dirs
        var outsideDefault = await TestHarness.RunCliAsync(
            ["inventory", "--out", Path.Combine(workspace.Root, "outside")],
            ffrDeps: null,
            workingDirectory: workspace.Root);
        Assert.Equal(2, outsideDefault.ExitCode);
        Assert.Contains(".deps/", outsideDefault.Stderr, StringComparison.Ordinal);

        // Protected paths under FFR_DEPS
        string[] protectedSubpaths =
        [
            "ffr",
            "ffr/files",
            "ffr/files/FFRS",
            "r1",
            "r1/dump",
            "dotnet",
            "nuget",
            "dotnet-home",
            "nuget-http",
        ];

        foreach (var subpath in protectedSubpaths)
        {
            var target = Path.Combine(workspace.DepsDir, subpath.Replace('/', Path.DirectorySeparatorChar));
            var result = await TestHarness.RunCliAsync(
                ["inventory", "--out", target],
                ffrDeps: workspace.DepsDir);
            Assert.Equal(2, result.ExitCode);
            Assert.Contains("受保护目录", result.Stderr, StringComparison.Ordinal);
        }

        // Exact FFR_DEPS root itself is rejected
        var rootTarget = await TestHarness.RunCliAsync(
            ["inventory", "--out", workspace.DepsDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, rootTarget.ExitCode);
        Assert.Contains("FFR_DEPS", rootTarget.Stderr, StringComparison.Ordinal);
    }

    [Fact]
    public async Task CliArguments_NoArgsEntersWizardAndUnknownSubcommandReturnsExitCode2()
    {
        var wizardEof = await TestHarness.RunCliAsync([], stdinInput: string.Empty);
        Assert.Equal(2, wizardEof.ExitCode);
        Assert.Contains("资源导出与建库向导", wizardEof.Stdout, StringComparison.Ordinal);
        Assert.DoesNotContain("缺少子命令", wizardEof.Stderr, StringComparison.Ordinal);

        var unknown = await TestHarness.RunCliAsync(["unknown-cmd"]);
        Assert.Equal(2, unknown.ExitCode);
        Assert.Contains("未知子命令", unknown.Stderr, StringComparison.Ordinal);
    }

    [Fact]
    public async Task RunScript_RespectsDotnetRootOverride()
    {
        var repoRoot = TestHarness.FindRepoRoot();

        var startInfo = new System.Diagnostics.ProcessStartInfo
        {
            FileName = ResolveBashExecutable(),
            WorkingDirectory = repoRoot,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Encoding.UTF8,
            StandardErrorEncoding = Encoding.UTF8,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        startInfo.ArgumentList.Add("scripts/run.sh");
        startInfo.ArgumentList.Add("export-assets");
        startInfo.ArgumentList.Add("--classify");
        startInfo.ArgumentList.Add("DataTable");
        startInfo.Environment["DOTNET_ROOT"] = Path.Combine(Path.GetTempPath(), $"missing-dotnet-{Guid.NewGuid():N}");

        using var process = System.Diagnostics.Process.Start(startInfo)
            ?? throw new InvalidOperationException("无法启动 scripts/run.sh");
        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        _ = await stdoutTask;
        var stderr = await stderrTask;

        Assert.Equal(2, process.ExitCode);
        Assert.Contains("找不到 dotnet 可执行文件", stderr, StringComparison.Ordinal);
    }

    private static string ResolveBashExecutable()
    {
        if (!OperatingSystem.IsWindows())
        {
            return "bash";
        }

        // On Windows, prefer Git Bash over the WSL System32\bash.exe launcher stub.
        var systemDir = Environment.SystemDirectory;
        var pathEnv = Environment.GetEnvironmentVariable("PATH");
        if (!string.IsNullOrWhiteSpace(pathEnv))
        {
            foreach (var rawDir in pathEnv.Split(Path.PathSeparator, StringSplitOptions.RemoveEmptyEntries | StringSplitOptions.TrimEntries))
            {
                var dir = rawDir.Trim('"');
                if (string.IsNullOrEmpty(dir))
                {
                    continue;
                }

                if (!string.IsNullOrEmpty(systemDir)
                    && Path.GetFullPath(dir).TrimEnd(Path.DirectorySeparatorChar)
                        .Equals(Path.GetFullPath(systemDir).TrimEnd(Path.DirectorySeparatorChar), StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var candidate = Path.Combine(dir, "bash.exe");
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        foreach (var programFiles in new[]
        {
            Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles),
            Environment.GetFolderPath(Environment.SpecialFolder.ProgramFilesX86),
        })
        {
            if (string.IsNullOrEmpty(programFiles))
            {
                continue;
            }

            foreach (var rel in new[] { Path.Combine("Git", "bin", "bash.exe"), Path.Combine("Git", "usr", "bin", "bash.exe") })
            {
                var candidate = Path.Combine(programFiles, rel);
                if (File.Exists(candidate))
                {
                    return candidate;
                }
            }
        }

        return "bash";
    }
}
