using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using System.Text.RegularExpressions;
using FfrExport.Tests;

namespace FfrExport.TestCliHost;

/// <summary>
/// Generates synthetic UE5 fixtures matching the smoke tests, executes all export subcommands
/// (export, locres, inventory, export-assets, export-maps) via the compiled FfrExport binary,
/// and emits a deterministic per-file SHA-256 manifest for cross-platform CI verification.
/// </summary>
internal static class FixtureExportRunner
{
    private static readonly UTF8Encoding Utf8WithoutBom = new(encoderShouldEmitUTF8Identifier: false);

    private static readonly JsonSerializerOptions ManifestJsonOptions = new()
    {
        WriteIndented = true,
        NewLine = "\n",
    };

    // Mask only volatile wall-clock values in-place so formatting, indentation, BOM, and line endings
    // (\r\n vs \n) remain completely intact when computing the SHA-256 digest.
    private static readonly Regex VolatileTimestampRegex = new(
        @"(?<=""(?:startedAt|finishedAt|generatedAt)"":\s*"")[^""]*(?="")",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private static readonly Regex VolatileElapsedRegex = new(
        @"(?<=""(?:elapsedSeconds|elapsedMs)"":\s*)-?[0-9]+(?:\.[0-9]+)?",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private static readonly HashSet<string> VolatileTimingFiles = new(StringComparer.Ordinal)
    {
        "export/manifest.json",
        "export/Localization/manifest.json",
        "inventory/manifest.json",
        "inventory/summary.json",
        "export-full/manifest.json",
        "export-full/maps/manifest.json",
        "export-full/maps/_graph.json",
    };

    internal static async Task<int> RunAsync(string[] args)
    {
        string? manifestOut = null;
        for (var i = 0; i < args.Length; i++)
        {
            switch (args[i])
            {
                case "--manifest-out":
                    if (i + 1 >= args.Length)
                    {
                        Console.Error.WriteLine("错误：--manifest-out 缺少文件路径参数");
                        return 2;
                    }

                    manifestOut = args[++i];
                    break;
                default:
                    Console.Error.WriteLine($"错误：--fixture-export 收到未知参数：{args[i]}");
                    return 2;
            }
        }

        var ffrExportDll = Path.Combine(AppContext.BaseDirectory, "FfrExport.dll");
        if (!File.Exists(ffrExportDll))
        {
            Console.Error.WriteLine($"错误：找不到 FfrExport.dll：{ffrExportDll}");
            return 2;
        }

        var dotnetExe = ResolveDotnetExecutable();
        var tempRoot = Path.Combine(
            Path.GetTempPath(),
            $"ffr-fixture-export-{Environment.ProcessId}-{Guid.NewGuid():N}");

        try
        {
            var paksDir = Path.Combine(tempRoot, "paks");
            var depsDir = Path.Combine(tempRoot, ".deps");
            var agpHomeDir = Path.Combine(tempRoot, "agp-home");
            var mapFixtureRoot = Path.Combine(tempRoot, "map-fixture");
            Directory.CreateDirectory(paksDir);
            Directory.CreateDirectory(depsDir);
            Directory.CreateDirectory(agpHomeDir);
            Directory.CreateDirectory(mapFixtureRoot);

            var usmapPath = Path.Combine(tempRoot, "Fixture.usmap");
            await WriteSyntheticPakAndUsmapAsync(paksDir, usmapPath);
            var (levelsDir, mapDataPath) = await WriteSyntheticMapFixtureAsync(mapFixtureRoot);

            // 1. export (corrupted dummy .uasset fails individually -> exit code 1, writes manifest.json)
            await InvokeExportAsync(
                dotnetExe,
                ffrExportDll,
                depsDir,
                agpHomeDir,
                ["export", "--paks", paksDir, "--usmap", usmapPath, "--prefix", "FFRS/Content/Datatable/"],
                expectedExitCode: 1,
                stageLabel: "export");

            // 2. locres (exit code 0, writes zh-Hans.json, en.json, ja.json, manifest.json)
            await InvokeExportAsync(
                dotnetExe,
                ffrExportDll,
                depsDir,
                agpHomeDir,
                ["locres", "--paks", paksDir],
                expectedExitCode: 0,
                stageLabel: "locres");

            // 3. inventory (exit code 0, writes registry.jsonl, summary.json, manifest.json)
            await InvokeExportAsync(
                dotnetExe,
                ffrExportDll,
                depsDir,
                agpHomeDir,
                ["inventory", "--paks", paksDir],
                expectedExitCode: 0,
                stageLabel: "inventory");

            // 4. export-assets (fixed --jobs 2, copies raw files verbatim + writes manifest.json -> exit code 1)
            await InvokeExportAsync(
                dotnetExe,
                ffrExportDll,
                depsDir,
                agpHomeDir,
                ["export-assets", "--paks", paksDir, "--usmap", usmapPath, "--jobs", "2"],
                expectedExitCode: 1,
                stageLabel: "export-assets");

            // 5. export-maps --from-json (fixed --jobs 2, writes level JSONs + _graph.json + manifest.json -> exit code 0)
            await InvokeExportAsync(
                dotnetExe,
                ffrExportDll,
                depsDir,
                agpHomeDir,
                ["export-maps", "--from-json", levelsDir, "--map-data", mapDataPath, "--jobs", "2"],
                expectedExitCode: 0,
                stageLabel: "export-maps");

            var ffrOutputRoot = Path.Combine(depsDir, "ffr");
            var entries = await CollectFileEntriesAsync(ffrOutputRoot);
            if (entries.Count == 0)
            {
                Console.Error.WriteLine($"错误：合成导出未生成任何输出文件（目录：{ffrOutputRoot}）");
                return 2;
            }

            var report = new
            {
                format = 1,
                os = RuntimeInformation.OSDescription.Trim(),
                file_count = entries.Count,
                files = entries,
            };
            var json = JsonSerializer.Serialize(report, ManifestJsonOptions) + "\n";

            if (!string.IsNullOrWhiteSpace(manifestOut))
            {
                var fullOut = Path.GetFullPath(manifestOut);
                var parent = Path.GetDirectoryName(fullOut);
                if (!string.IsNullOrWhiteSpace(parent))
                {
                    Directory.CreateDirectory(parent);
                }

                await File.WriteAllTextAsync(fullOut, json, Utf8WithoutBom);
                Console.Error.WriteLine($"==> 合成导出完成：共 {entries.Count} 个文件，已写入清单：{fullOut}");
            }
            else
            {
                Console.Write(json);
            }

            return 0;
        }
        finally
        {
            try
            {
                if (Directory.Exists(tempRoot))
                {
                    Directory.Delete(tempRoot, recursive: true);
                }
            }
            catch
            {
                // Ignore transient cleanup failures in temporary directories.
            }
        }
    }

    private static async Task WriteSyntheticPakAndUsmapAsync(string paksDir, string usmapPath)
    {
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

        // Include CRLF in DefaultGame.ini to also verify raw game files remain verbatim byte-for-byte on both OSes.
        var defaultGameIni = Encoding.UTF8.GetBytes("[/Script/Test.Settings]\r\nMoveSpeed=600\r\n");
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
            new("FFRS/Content/Localization/Game/zh-Hans/Game.locres", zhLocres),
            new("FFRS/Content/Localization/Game/en/Game.locres", enLocres),
            new("FFRS/Content/Localization/Game/ja/Game.locres", jaLocres),
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

        var usmapBytes = SyntheticFixtureBuilder.BuildUsmapV0(
            ["EDemo", "Zero", "One", "FDemoRow", "Id"],
            [new("EDemo", ["Zero", "One"])],
            [new("FDemoRow", null, 1, [new(0, 1, "Id", 2)])]);
        await File.WriteAllBytesAsync(usmapPath, usmapBytes);
    }

    private static async Task<(string LevelsDir, string MapDataPath)> WriteSyntheticMapFixtureAsync(string root)
    {
        var levelsDir = Path.Combine(root, "levels");
        Directory.CreateDirectory(levelsDir);

        var mapDataPath = Path.Combine(root, "CDT_MapData_Prod.json");
        await File.WriteAllTextAsync(mapDataPath, """
        [
          {
            "Type": "CompositeDataTable",
            "Rows": {
              "Sample_01": {
                "ID": 9901,
                "Level": {
                  "AssetPathName": "/Game/Map/Dng/99Sample/99Sample_01/Dng_99Sample_01_PL.Dng_99Sample_01_PL"
                },
                "MapType": "eMapType::Dungeon",
                "mainProgressLoadLevelList": []
              },
              "Sample_02": {
                "ID": 9902,
                "Level": {
                  "AssetPathName": "/Game/Map/Town/99Sample/99Sample_02/Town_99Sample_02_PL.Town_99Sample_02_PL"
                },
                "MapType": "eMapType::Town",
                "mainProgressLoadLevelList": []
              }
            }
          }
        ]
        """, Utf8WithoutBom);

        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Dng_99Sample_01_PL.json"), """
        {
          "path": "FFRS/Content/Map/Dng/99Sample/99Sample_01/Dng_99Sample_01_PL.umap",
          "streamingLevels": [
            {
              "name": "Dng_99Sample_01_GD",
              "worldAsset": "/Game/Map/Dng/99Sample/99Sample_01/GD/Dng_99Sample_01_GD.Dng_99Sample_01_GD",
              "streamingClass": "LevelStreamingDynamic"
            },
            {
              "name": "Dng_99Sample_01_EV",
              "worldAsset": "/Game/Map/Dng/99Sample/99Sample_01/EV/Dng_99Sample_01_EV.Dng_99Sample_01_EV",
              "streamingClass": "LevelStreamingDynamic"
            }
          ],
          "actors": [
            {
              "name": "CPP_MuchaWorldSetting",
              "class": "/Script/mucha.CPP_MuchaWorldSetting",
              "isGameClass": true,
              "properties": { "isUseMapManager": true }
            }
          ]
        }
        """, Utf8WithoutBom);

        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Dng_99Sample_01_GD.json"), """
        {
          "path": "FFRS/Content/Map/Dng/99Sample/99Sample_01/GD/Dng_99Sample_01_GD.umap",
          "actors": [
            {
              "name": "BP_MapTransitionTrigger_C_0",
              "class": "/Game/BP/Map/Transition/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
              "isGameClass": true,
              "location": { "x": 100, "y": 200, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 1,
                "m_IsAutoTransition": true,
                "m_MapId": 9902,
                "m_PointID": 1
              }
            },
            {
              "name": "BP_MapGimmick_TreasureBox_C_0",
              "class": "/Game/BP/Map/Gimmick/BP_MapGimmick_TreasureBox.BP_MapGimmick_TreasureBox_C",
              "isGameClass": true,
              "location": { "x": -50, "y": 40, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 10,
                "m_Id": 0,
                "interactType": "eMapInteractType::Treasure",
                "m_TreasureBoxData": { "ID": 0, "mapId": 9901, "ItemId": 11, "ItemNum": 2 }
              }
            }
          ]
        }
        """, Utf8WithoutBom);

        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Dng_99Sample_01_EV.json"), """
        {
          "path": "FFRS/Content/Map/Dng/99Sample/99Sample_01/EV/Dng_99Sample_01_EV.umap",
          "actors": [
            {
              "name": "BP_MapUnit_NPC_C_0",
              "class": "/Game/BP/Map/Unit/BP_MapUnit_NPC.BP_MapUnit_NPC_C",
              "isGameClass": true,
              "location": { "x": 10, "y": 20, "z": 30 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_MapUnitId": 101,
                "m_UniqueId": 1,
                "m_MapInteractType": "eMapInteractType::Shop",
                "ShopEntry": { "ShopID": 42 }
              }
            }
          ]
        }
        """, Utf8WithoutBom);

        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Town_99Sample_02_PL.json"), """
        {
          "path": "FFRS/Content/Map/Town/99Sample/99Sample_02/Town_99Sample_02_PL.umap",
          "actors": []
        }
        """, Utf8WithoutBom);

        return (levelsDir, mapDataPath);
    }

    private static async Task InvokeExportAsync(
        string dotnetExe,
        string ffrExportDll,
        string depsDir,
        string agpHomeDir,
        IReadOnlyList<string> args,
        int expectedExitCode,
        string stageLabel)
    {
        var startInfo = new ProcessStartInfo
        {
            FileName = dotnetExe,
            RedirectStandardOutput = true,
            RedirectStandardError = true,
            StandardOutputEncoding = Utf8WithoutBom,
            StandardErrorEncoding = Utf8WithoutBom,
            UseShellExecute = false,
            CreateNoWindow = true,
        };
        startInfo.ArgumentList.Add("exec");
        startInfo.ArgumentList.Add(ffrExportDll);
        foreach (var arg in args)
        {
            startInfo.ArgumentList.Add(arg);
        }

        startInfo.Environment["FFR_DEPS"] = depsDir;
        startInfo.Environment["AGP_HOME"] = agpHomeDir;

        using var process = Process.Start(startInfo)
            ?? throw new InvalidOperationException($"无法启动 FfrExport 子进程（阶段：{stageLabel}）");

        var stdoutTask = process.StandardOutput.ReadToEndAsync();
        var stderrTask = process.StandardError.ReadToEndAsync();
        await process.WaitForExitAsync();
        var stdout = await stdoutTask;
        var stderr = await stderrTask;

        if (process.ExitCode != expectedExitCode)
        {
            throw new InvalidOperationException(
                $"合成导出阶段 {stageLabel} 退出码异常：期望 {expectedExitCode}，实际 {process.ExitCode}\n" +
                $"stdout:\n{stdout}\nstderr:\n{stderr}");
        }
    }

    private static async Task<List<object>> CollectFileEntriesAsync(string ffrOutputRoot)
    {
        var files = Directory.EnumerateFiles(ffrOutputRoot, "*", SearchOption.AllDirectories)
            .Select(path => (FullPath: path, RelativePath: Path.GetRelativePath(ffrOutputRoot, path).Replace('\\', '/')))
            .OrderBy(item => item.RelativePath, StringComparer.Ordinal)
            .ToArray();

        var entries = new List<object>(files.Length);
        foreach (var (fullPath, relativePath) in files)
        {
            var rawBytes = await File.ReadAllBytesAsync(fullPath);
            var hasBom = rawBytes.Length >= 3 && rawBytes[0] == 0xEF && rawBytes[1] == 0xBB && rawBytes[2] == 0xBF;
            var hasCr = Array.IndexOf(rawBytes, (byte)'\r') >= 0;

            var normalizedVolatile = VolatileTimingFiles.Contains(relativePath);
            byte[] bytesToHash;
            if (normalizedVolatile)
            {
                var rawText = Utf8WithoutBom.GetString(rawBytes);
                var masked = VolatileTimestampRegex.Replace(rawText, "<NORMALIZED>");
                masked = VolatileElapsedRegex.Replace(masked, "0");
                bytesToHash = Utf8WithoutBom.GetBytes(masked);
            }
            else
            {
                bytesToHash = rawBytes;
            }

            var sha256 = Convert.ToHexStringLower(SHA256.HashData(bytesToHash));
            entries.Add(new
            {
                path = relativePath,
                size = bytesToHash.LongLength,
                sha256,
                normalized_volatile = normalizedVolatile,
                has_cr = hasCr,
                has_bom = hasBom,
            });
        }

        return entries;
    }

    private static string ResolveDotnetExecutable()
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

        if (!string.IsNullOrWhiteSpace(Environment.ProcessPath)
            && Path.GetFileNameWithoutExtension(Environment.ProcessPath).Equals("dotnet", StringComparison.OrdinalIgnoreCase))
        {
            return Environment.ProcessPath;
        }

        return "dotnet";
    }
}
