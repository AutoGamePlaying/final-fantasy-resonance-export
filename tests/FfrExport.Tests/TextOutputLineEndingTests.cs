using System.Globalization;
using System.Text;
using System.Text.Json;
using FfrExport.Gui.Core;
using Newtonsoft.Json;
using Xunit;

namespace FfrExport.Tests;

public sealed class TextOutputLineEndingTests
{
    [Fact]
    public void NewtonsoftJson_ForcesLfEvenWhenWriterDefaultsToCrlf()
    {
        var payload = new
        {
            command = "export-assets",
            label = "中文内容",
            embeddedLineBreak = "line1\nline2",
            items = new[]
            {
                new { name = "DT_Demo", bytes = 1234 },
                new { name = "BP_Hero", bytes = 5678 },
            },
        };

        // First verify that raw Newtonsoft.Json on a CRLF StringWriter (Windows default) DOES emit \r\n.
        var rawBuffer = new StringBuilder();
        using (var rawWriter = new StringWriter(rawBuffer, CultureInfo.InvariantCulture) { NewLine = "\r\n" })
        using (var rawJsonWriter = new JsonTextWriter(rawWriter) { Formatting = Formatting.Indented })
        {
            Newtonsoft.Json.JsonSerializer.CreateDefault().Serialize(rawJsonWriter, payload);
        }

        Assert.Contains("\r\n", rawBuffer.ToString(), StringComparison.Ordinal);

        // Passing a CRLF-initialized TextWriter into TextOutput must force LF ('\n') with zero CR bytes.
        var lfBuffer = new StringBuilder();
        using var simulatedWindowsWriter = new StringWriter(lfBuffer, CultureInfo.InvariantCulture) { NewLine = "\r\n" };
        TextOutput.SerializeJsonToWriter(simulatedWindowsWriter, payload, Formatting.Indented, trailingNewline: false);

        var lfJson = lfBuffer.ToString();
        Assert.Equal("\n", simulatedWindowsWriter.NewLine);
        Assert.Contains("\n", lfJson, StringComparison.Ordinal);
        Assert.DoesNotContain("\r", lfJson, StringComparison.Ordinal);
        Assert.False(lfJson.EndsWith('\n'));
        Assert.Equal(TextOutput.SerializeJson(payload, Formatting.Indented, trailingNewline: false), lfJson);

        var lfJsonWithTrailing = TextOutput.SerializeJson(payload, Formatting.Indented, trailingNewline: true);
        Assert.Equal(lfJson + "\n", lfJsonWithTrailing);
        Assert.DoesNotContain((byte)'\r', TextOutput.Utf8WithoutBom.GetBytes(lfJsonWithTrailing));

        var compact = TextOutput.SerializeJsonCompact(payload);
        Assert.DoesNotContain("\r", compact, StringComparison.Ordinal);
        Assert.DoesNotContain("\n", compact, StringComparison.Ordinal);
    }

    [Fact]
    public void SystemTextJson_ForcesLfEvenWhenOptionsSpecifyCrlf()
    {
        var settings = new GuiSettings("/tmp/sample-output", true);
        var windowsOptions = new JsonSerializerOptions
        {
            WriteIndented = true,
            NewLine = "\r\n",
        };

        // Raw System.Text.Json with CRLF options emits \r\n.
        var rawCrlf = System.Text.Json.JsonSerializer.Serialize(settings, windowsOptions);
        Assert.Contains("\r\n", rawCrlf, StringComparison.Ordinal);

        // TextOutput overrides NewLine to LF and preserves indentation.
        var normalized = TextOutput.SerializeSystemJson(settings, windowsOptions, trailingNewline: true);
        Assert.Contains("\n", normalized, StringComparison.Ordinal);
        Assert.DoesNotContain("\r", normalized, StringComparison.Ordinal);
        Assert.EndsWith("\n", normalized, StringComparison.Ordinal);
        Assert.DoesNotContain((byte)'\r', TextOutput.Utf8WithoutBom.GetBytes(normalized));

        var defaultOptions = TextOutput.CreateSystemJsonOptions(writeIndented: true);
        Assert.Equal("\n", defaultOptions.NewLine);
        Assert.Equal(normalized, TextOutput.SerializeSystemJson(settings, defaultOptions, trailingNewline: true));
    }

    [Fact]
    public async Task FileWritersAndLoggers_WriteBomlessUtf8WithLfOnly()
    {
        using var workspace = new TempWorkspace("ffr-text-output-lf");

        var payload = new { a = 1, b = new[] { "x", "y" } };
        var crlfInput = "第一行\r\n第二行\r第三行\r\n";

        var textSync = Path.Combine(workspace.Root, "sync.txt");
        var textAsync = Path.Combine(workspace.Root, "async.txt");
        var textAtomic = Path.Combine(workspace.Root, "sub", "atomic.txt");
        var textAtomicAsync = Path.Combine(workspace.Root, "sub", "atomic-async.txt");
        var jsonPath = Path.Combine(workspace.Root, "doc.json");
        var jsonAsyncPath = Path.Combine(workspace.Root, "doc-async.json");
        var jsonAtomicPath = Path.Combine(workspace.Root, "nested", "doc-atomic.json");
        var jsonAtomicAsyncPath = Path.Combine(workspace.Root, "nested", "doc-atomic-async.json");
        var jsonlPath = Path.Combine(workspace.Root, "records.jsonl");
        var newLogPath = Path.Combine(workspace.Root, "new.log");

        TextOutput.WriteText(textSync, crlfInput);
        await TextOutput.WriteTextAsync(textAsync, crlfInput);
        var atomicLen = TextOutput.WriteTextAtomic(textAtomic, crlfInput);
        var atomicAsyncLen = await TextOutput.WriteTextAtomicAsync(textAtomicAsync, crlfInput);
        TextOutput.WriteJson(jsonPath, payload);
        await TextOutput.WriteJsonAsync(jsonAsyncPath, payload, trailingNewline: true);
        var jsonAtomicLen = TextOutput.WriteJsonAtomic(jsonAtomicPath, payload);
        var jsonAtomicAsyncLen = await TextOutput.WriteJsonAtomicAsync(jsonAtomicAsyncPath, payload, trailingNewline: true);

        await using (var sw = TextOutput.CreateStreamWriter(jsonlPath))
        {
            Assert.Equal("\n", sw.NewLine);
            await sw.WriteLineAsync(TextOutput.SerializeJsonCompact(new { id = 1 }));
            await sw.WriteLineAsync(TextOutput.SerializeJsonCompact(new { id = 2 }));
        }

        TextOutput.WriteNewTextFile(newLogPath, "start\r\n");
        TextOutput.AppendText(newLogPath, "next\r\n");

        Assert.Equal(new FileInfo(textAtomic).Length, atomicLen);
        Assert.Equal(new FileInfo(textAtomicAsync).Length, atomicAsyncLen);
        Assert.Equal(new FileInfo(jsonAtomicPath).Length, jsonAtomicLen);
        Assert.Equal(new FileInfo(jsonAtomicAsyncPath).Length, jsonAtomicAsyncLen);

        // GUI settings, GUI startup log, and CLI log files must also be BOM-less LF UTF-8.
        var settingsPath = Path.Combine(workspace.Root, "settings.json");
        GuiSettingsStore.Save(new GuiSettings(workspace.Root, true), settingsPath);

        var guiLogSession = GuiStartupLog.StartSession(
            GuiStartupLog.CollectStartupInfo(typeof(TextOutputLineEndingTests).Assembly, "PerMonitorV2", "test"),
            localAppDataOverride: Path.Combine(workspace.Root, "LocalAppData"),
            tempDirectoryOverride: workspace.Root);
        guiLogSession.WriteInfo("line-with-info");
        guiLogSession.WriteException("test-stage", new InvalidOperationException("line1\r\nline2"));
        Assert.NotNull(guiLogSession.LogFilePath);

        var cliLogsDir = Path.Combine(workspace.Root, "cli-logs");
        string? cliLogPath;
        using (var cliLog = CliLogWriter.Create(new StringWriter(), cliLogsDir))
        {
            cliLog.WriteLine("cli line 1\r\ncli line 2");
            cliLogPath = cliLog.LogFilePath;
        }

        Assert.NotNull(cliLogPath);

        string[] writtenFiles =
        [
            textSync,
            textAsync,
            textAtomic,
            textAtomicAsync,
            jsonPath,
            jsonAsyncPath,
            jsonAtomicPath,
            jsonAtomicAsyncPath,
            jsonlPath,
            newLogPath,
            settingsPath,
            guiLogSession.LogFilePath!,
            cliLogPath!,
        ];

        foreach (var file in writtenFiles)
        {
            var bytes = await File.ReadAllBytesAsync(file);
            AssertBomlessLfUtf8(bytes, file);
        }

        // Verify trailing newline preservation semantics match pre-change Linux output.
        Assert.False((await File.ReadAllBytesAsync(jsonPath)).EndsWith([(byte)'\n']));
        Assert.True((await File.ReadAllBytesAsync(jsonAsyncPath)).EndsWith([(byte)'\n']));
        Assert.False((await File.ReadAllBytesAsync(jsonAtomicPath)).EndsWith([(byte)'\n']));
        Assert.True((await File.ReadAllBytesAsync(jsonAtomicAsyncPath)).EndsWith([(byte)'\n']));
    }

    [Fact]
    public async Task AllExportCommands_EmitBomlessLfTextAndKeepRawCopiedFilesVerbatim()
    {
        using var workspace = new TempWorkspace("ffr-exports-lf-e2e");
        var paksDir = Path.Combine(workspace.Root, "paks");
        Directory.CreateDirectory(paksDir);

        FixtureAsset[] assets =
        [
            new("/Game/Datatable/DT_Sample", "/Game/Datatable", "DT_Sample", "DataTable"),
            new("/Game/Textures/T_Sample", "/Game/Textures", "T_Sample", "Texture2D"),
            new("/Game/Maps/L_Demo", "/Game/Maps", "L_Demo", "World"),
        ];

        var arBytes = SyntheticFixtureBuilder.BuildAssetRegistry(assets);
        var zhLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string> { ["Title"] = "示例标题", ["Sub"] = "副标题" },
        });
        var enLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string> { ["Title"] = "Sample Title", ["Sub"] = "Subtitle" },
        });
        var jaLocres = SyntheticFixtureBuilder.BuildLocres(new Dictionary<string, IReadOnlyDictionary<string, string>>
        {
            ["ST_Sample"] = new Dictionary<string, string> { ["Title"] = "サンプル", ["Sub"] = "サブ" },
        });

        // Deliberately store CRLF in DefaultGame.ini to prove raw game files are copied verbatim byte-for-byte.
        var rawCrlfIni = Encoding.UTF8.GetBytes("[/Script/Test.Settings]\r\nMoveSpeed=600\r\n");
        var rawLfUproject = Encoding.UTF8.GetBytes("{\"FileVersion\":3}\n");

        var pakBytes = SyntheticFixtureBuilder.BuildPak([
            new("FFRS/AssetRegistry.bin", arBytes),
            new("FFRS/Content/Localization/Game/zh-Hans/Game.locres", zhLocres),
            new("FFRS/Content/Localization/Game/en/Game.locres", enLocres),
            new("FFRS/Content/Localization/Game/ja/Game.locres", jaLocres),
            new("FFRS/Config/DefaultGame.ini", rawCrlfIni),
            new("FFRS/FFRS.uproject", rawLfUproject),
            new("FFRS/Content/Datatable/DT_Sample.uasset", [1, 2, 3, 4]),
            new("FFRS/Content/Textures/T_Sample.uasset", [5, 6, 7]),
            new("FFRS/Content/Maps/L_Demo.umap", [8, 9, 10]),
        ]);
        await File.WriteAllBytesAsync(Path.Combine(paksDir, "Fixture-Windows.pak"), pakBytes);

        var usmapPath = Path.Combine(workspace.Root, "Fixture.usmap");
        await File.WriteAllBytesAsync(usmapPath, SyntheticFixtureBuilder.BuildUsmap());

        // Map fixture JSON input for export-maps --from-json
        var mapFixtureDir = Path.Combine(workspace.Root, "map-fixtures");
        Directory.CreateDirectory(mapFixtureDir);
        var mapDataPath = Path.Combine(workspace.Root, "CDT_MapData_Prod.json");
        await File.WriteAllTextAsync(mapDataPath, """
        [
          {
            "Type": "CompositeDataTable",
            "Rows": {
              "Sample_01": {
                "ID": 9901,
                "Level": {
                  "AssetPathName": "/Game/Map/Dng/99Sample/Dng_99Sample_01_PL.Dng_99Sample_01_PL"
                },
                "MapType": "eMapType::Dungeon",
                "mainProgressLoadLevelList": []
              }
            }
          }
        ]
        """);
        await File.WriteAllTextAsync(Path.Combine(mapFixtureDir, "Dng_99Sample_01_PL.json"), """
        {
          "path": "FFRS/Content/Map/Dng/99Sample/Dng_99Sample_01_PL.umap",
          "actors": [
            {
              "name": "CPP_MuchaWorldSetting",
              "class": "/Script/mucha.CPP_MuchaWorldSetting",
              "isGameClass": true,
              "properties": { "isUseMapManager": true }
            }
          ]
        }
        """);

        var exportRes = await TestHarness.RunCliAsync(
            ["export", "--paks", paksDir, "--usmap", usmapPath, "--prefix", "FFRS/Content/Datatable/"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, exportRes.ExitCode);

        var locresRes = await TestHarness.RunCliAsync(
            ["locres", "--paks", paksDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, locresRes.ExitCode);

        var invRes = await TestHarness.RunCliAsync(
            ["inventory", "--paks", paksDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, invRes.ExitCode);

        var fullRes = await TestHarness.RunCliAsync(
            ["export-assets", "--paks", paksDir, "--usmap", usmapPath, "--jobs", "2"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, fullRes.ExitCode);

        var mapsRes = await TestHarness.RunCliAsync(
            ["export-maps", "--from-json", mapFixtureDir, "--map-data", mapDataPath, "--jobs", "2"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, mapsRes.ExitCode);

        // Verify verbatim raw copies preserve original bytes (including CRLF in DefaultGame.ini).
        var copiedIniPath = Path.Combine(workspace.DepsDir, "ffr", "export-full", "FFRS", "Config", "DefaultGame.ini");
        var copiedUprojectPath = Path.Combine(workspace.DepsDir, "ffr", "export-full", "FFRS", "FFRS.uproject");
        Assert.Equal(rawCrlfIni, await File.ReadAllBytesAsync(copiedIniPath));
        Assert.Equal(rawLfUproject, await File.ReadAllBytesAsync(copiedUprojectPath));

        // Verify every tool-written JSON and JSONL file across all 5 commands is BOM-less LF UTF-8.
        var ffrOutRoot = Path.Combine(workspace.DepsDir, "ffr");
        var generatedTextFiles = Directory.EnumerateFiles(ffrOutRoot, "*", SearchOption.AllDirectories)
            .Where(path =>
                path.EndsWith(".json", StringComparison.OrdinalIgnoreCase)
                || path.EndsWith(".jsonl", StringComparison.OrdinalIgnoreCase))
            .OrderBy(path => path, StringComparer.Ordinal)
            .ToArray();

        Assert.True(generatedTextFiles.Length >= 10, $"预期生成至少 10 个 JSON/JSONL 文件，实际：{generatedTextFiles.Length}");
        foreach (var file in generatedTextFiles)
        {
            var bytes = await File.ReadAllBytesAsync(file);
            AssertBomlessLfUtf8(bytes, file);
        }
    }

    private static void AssertBomlessLfUtf8(byte[] bytes, string path)
    {
        var hasBom = bytes.Length >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF;
        Assert.False(hasBom, $"文件 {path} 不得包含 UTF-8 BOM");
        Assert.DoesNotContain((byte)'\r', bytes);
        Assert.Contains((byte)'\n', bytes);
    }
}
