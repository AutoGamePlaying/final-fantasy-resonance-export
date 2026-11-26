using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class MapExportTests
{
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
        """);

        // Map 9901: persistent level with streaming level declarations
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
            },
            {
              "name": "Dng_99Sample_01_01_BG",
              "worldAsset": "/Game/Map/Dng/99Sample/99Sample_01/BG/Dng_99Sample_01_01_BG.Dng_99Sample_01_01_BG",
              "streamingClass": "LevelStreamingDynamic"
            }
          ],
          "actors": [
            {
              "name": "CPP_MuchaWorldSetting",
              "class": "/Script/mucha.CPP_MuchaWorldSetting",
              "isGameClass": true,
              "location": null,
              "rotation": null,
              "scale": null,
              "properties": { "isUseMapManager": true }
            }
          ]
        }
        """);

        // Map 9901: GD level with transitions, transition points, save point, chests, gimmicks, encounters
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
              },
              "subobjects": [
                {
                  "name": "RootBoxComponent",
                  "class": "/Script/Engine.BoxComponent",
                  "isGameClass": false,
                  "boxExtent": { "x": 80, "y": 120, "z": 60 }
                }
              ]
            },
            {
              "name": "BP_MapTransitionTrigger_C_1",
              "class": "/Game/BP/Map/Transition/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
              "isGameClass": true,
              "location": { "x": 300, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 2,
                "m_IsUseCondion": true,
                "mTransitionDataList": [
                  {
                    "FlagCondition": "!{flag:10}",
                    "isEventOnly": false,
                    "mapId": 9902,
                    "pointId": 2,
                    "afterTransitionEventList": [{ "Condition": "!{flag:10}", "EventId": "Sample_After" }]
                  },
                  {
                    "FlagCondition": "{flag:10}",
                    "isEventOnly": false,
                    "mapId": 9902,
                    "pointId": 3,
                    "afterTransitionEventList": []
                  }
                ]
              }
            },
            {
              "name": "BP_MapTransitionTrigger_C_2",
              "class": "/Game/BP/Map/Transition/BP_MapTransitionTrigger.BP_MapTransitionTrigger_C",
              "isGameClass": true,
              "location": { "x": 400, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 3,
                "m_IsSameMapTrasition": true,
                "m_PointID": 5
              }
            },
            {
              "name": "BP_MapTransitionPoint_C_0",
              "class": "/Game/BP/Map/Transition/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
              "isGameClass": true,
              "location": { "x": 10, "y": 20, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_PointID": 0,
                "m_Direction": "eMapDirectionType::North",
                "m_PointName": { "LocalizedString": "SampleEntry" }
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
            },
            {
              "name": "BP_MapGimmick_TreasureBox002_C_1",
              "class": "/Game/BP/Map/Gimmick/Child/TreasureBox/BP_MapGimmick_TreasureBox002.BP_MapGimmick_TreasureBox002_C",
              "isGameClass": true,
              "location": { "x": -60, "y": 40, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 11,
                "m_Id": 1,
                "interactType": "eMapInteractType::Treasure",
                "m_TreasureBoxData": {
                  "ID": 1,
                  "mapId": 9901,
                  "ItemId": -1,
                  "Money": 500,
                  "IsTrap": true,
                  "battleGroupId": 77
                }
              }
            },
            {
              "name": "BP_MapGimmick_SavePoint_C_0",
              "class": "/Game/BP/Map/Gimmick/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
              "isGameClass": true,
              "location": { "x": 500, "y": 100, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 25,
                "interactType": "eMapInteractType::SavePoint",
                "m_IsTeleport": true,
                "m_TranditionMapId": 9902,
                "m_TranditionPoint": 10000,
                "m_ParentSavePointData": { "mapId": 9902, "UniqueID": 16 }
              }
            },
            {
              "name": "BP_MapGimmick_BlockingBase_C_0",
              "class": "/Game/BP/Map/Gimmick/BP_MapGimmick_BlockingBase.BP_MapGimmick_BlockingBase_C",
              "isGameClass": true,
              "location": { "x": 0, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": { "m_UniqueId": 30 }
            },
            {
              "name": "BP_MapEncountBox_C_0",
              "class": "/Game/BP/Map/Encount/BP_MapEncountBox.BP_MapEncountBox_C",
              "isGameClass": true,
              "location": { "x": 1000, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_Id": 1,
                "m_Priority": 100,
                "m_pEncountDatatable": { "ObjectPath": "/Game/Datatable/Encount/DT_Sample_Encount.0" },
                "m_EncountDatatableList": [
                  {
                    "terrain": "eMapTerrainType::Forest",
                    "pEncountDatatable": { "ObjectPath": "/Game/Datatable/Encount/DT_Sample_Forest.0" }
                  }
                ]
              },
              "subobjects": [
                {
                  "name": "RootBoxComponent",
                  "class": "/Script/Engine.BoxComponent",
                  "isGameClass": false,
                  "boxExtent": { "x": 500, "y": 500, "z": 200 }
                }
              ]
            },
            {
              "name": "BP_MapEncountOverwirteBox_C_0",
              "class": "/Game/BP/Map/Encount/BP_MapEncountOverwirteBox.BP_MapEncountOverwirteBox_C",
              "isGameClass": true,
              "location": { "x": 1100, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_OverwriteEncountRatio": 0.0,
                "m_IsResetEncountDistance": true
              }
            }
          ]
        }
        """);

        // Map 9901: EV level with NPCs (shop, various shop, inn, decorative object) and event triggers
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
                "ShopEntry": { "ShopID": 42 },
                "m_EventSettings": {
                  "EventList": [{ "Condition": "", "EventList": [{ "EventId": "Sample_ShopTalk" }] }]
                }
              }
            },
            {
              "name": "BP_MapUnit_NPC_C_1",
              "class": "/Game/BP/Map/Unit/BP_MapUnit_NPC.BP_MapUnit_NPC_C",
              "isGameClass": true,
              "location": { "x": 20, "y": 20, "z": 30 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_MapUnitId": 102,
                "m_UniqueId": 2,
                "m_MapInteractType": "eMapInteractType::Shop",
                "ShopEntry": { "ShopID": 10005 }
              }
            },
            {
              "name": "BP_MapUnit_NPC_C_2",
              "class": "/Game/BP/Map/Unit/BP_MapUnit_NPC.BP_MapUnit_NPC_C",
              "isGameClass": true,
              "location": { "x": 30, "y": 20, "z": 30 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_MapUnitId": 103,
                "m_UniqueId": 3,
                "m_MapInteractType": "eMapInteractType::Inn"
              }
            },
            {
              "name": "BP_MapUnit_Object_Shadow_C_0",
              "class": "/Game/BP/Map/Unit/Object/BP_MapUnit_Object_Shadow.BP_MapUnit_Object_Shadow_C",
              "isGameClass": true,
              "location": { "x": 40, "y": 20, "z": 30 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": { "m_MapUnitId": 201 }
            },
            {
              "name": "BP_MapGimmick_EventTrigger_C_0",
              "class": "/Game/BP/Map/Gimmick/BP_MapGimmick_EventTrigger.BP_MapGimmick_EventTrigger_C",
              "isGameClass": true,
              "location": { "x": 50, "y": 50, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 200,
                "m_EventList": [{ "Condition": "!{flag:99}", "EventId": "Sample_TriggerEv" }]
              },
              "subobjects": [
                {
                  "name": "RootBoxComponent",
                  "class": "/Script/Engine.BoxComponent",
                  "isGameClass": false,
                  "boxExtent": { "x": 32, "y": 400, "z": 32 }
                }
              ]
            },
            {
              "name": "BP_CustomInteract_C_0",
              "class": "/Game/BP/Sample/BP_CustomInteract.BP_CustomInteract_C",
              "isGameClass": true,
              "location": { "x": 60, "y": 50, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_InteractEventId": { "EventId": "Sample_InteractEv" }
              }
            }
          ]
        }
        """);

        // Map 9901: BG level with only engine actors
        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Dng_99Sample_01_01_BG.json"), """
        {
          "path": "FFRS/Content/Map/Dng/99Sample/99Sample_01/BG/Dng_99Sample_01_01_BG.umap",
          "actors": [
            {
              "name": "StaticMeshActor_0",
              "class": "/Script/Engine.StaticMeshActor",
              "isGameClass": false,
              "location": { "x": 0, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 90, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "assetRefs": ["/Game/Env/Sample/SM_Pillar"]
            }
          ]
        }
        """);

        // Map 9902: PL + GD with same-map save point teleport and transition point
        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Town_99Sample_02_PL.json"), """
        {
          "path": "FFRS/Content/Map/Town/99Sample/99Sample_02/Town_99Sample_02_PL.umap",
          "streamingLevels": [
            {
              "name": "Town_99Sample_02_GD",
              "worldAsset": "/Game/Map/Town/99Sample/99Sample_02/GD/Town_99Sample_02_GD.Town_99Sample_02_GD",
              "streamingClass": "LevelStreamingDynamic"
            }
          ],
          "actors": []
        }
        """);

        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Town_99Sample_02_GD.json"), """
        {
          "path": "FFRS/Content/Map/Town/99Sample/99Sample_02/GD/Town_99Sample_02_GD.umap",
          "actors": [
            {
              "name": "BP_MapTransitionPoint_C_0",
              "class": "/Game/BP/Map/Transition/BP_MapTransitionPoint.BP_MapTransitionPoint_C",
              "isGameClass": true,
              "location": { "x": 0, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": { "m_PointID": 10000 }
            },
            {
              "name": "BP_MapGimmick_SavePoint_C_0",
              "class": "/Game/BP/Map/Gimmick/BP_MapGimmick_SavePoint.BP_MapGimmick_SavePoint_C",
              "isGameClass": true,
              "location": { "x": 20, "y": 0, "z": 0 },
              "rotation": { "pitch": 0, "yaw": 0, "roll": 0 },
              "scale": { "x": 1, "y": 1, "z": 1 },
              "properties": {
                "m_UniqueId": 16,
                "m_Type": "eMapGimmick_SavePointType::Boss",
                "m_IsTeleport": true,
                "m_TranditionPoint": 10000
              }
            }
          ]
        }
        """);

        return (levelsDir, mapDataPath);
    }

    [Fact]
    public async Task ExportMaps_FromJsonFixture_ExtractsSummaryBuildsGraphResumesAndForces()
    {
        using var workspace = new TempWorkspace("ffr-map-fixture");
        var fixtureRoot = Path.Combine(workspace.Root, "fixture");
        var (levelsDir, mapDataPath) = await WriteSyntheticMapFixtureAsync(fixtureRoot);
        var outDir = Path.Combine(workspace.DepsDir, "ffr", "export-full", "maps");

        // 1. Fresh run (uses default $FFR_DEPS/ffr/export-full/maps output)
        var first = await TestHarness.RunCliAsync(
            ["export-maps", "--from-json", levelsDir, "--map-data", mapDataPath],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, first.ExitCode);

        var manifest1 = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "manifest.json")));
        Assert.Equal("export-maps", manifest1["command"]?.Value<string>());
        Assert.Equal("completed", manifest1["status"]?.Value<string>());
        Assert.Equal(6, manifest1["totalLevels"]?.Value<int>());
        Assert.Equal(6, manifest1["exportedCount"]?.Value<int>());
        Assert.Equal(0, manifest1["skippedCount"]?.Value<int>());
        Assert.Equal(0, manifest1["failureCount"]?.Value<int>());
        Assert.Equal("_graph.json", manifest1["graphFile"]?.Value<string>());

        // Inspect GD level summary
        var gdDoc = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "Dng_99Sample_01_GD.json")));
        Assert.Equal("GD", gdDoc["level"]?["sublevelType"]?.Value<string>());
        Assert.Equal("Dng_99Sample_01_PL", gdDoc["level"]?["persistentLevel"]?.Value<string>());
        Assert.Equal(9901, gdDoc["level"]?["mapId"]?.Value<int>());

        var transitions = Assert.IsType<JArray>(gdDoc["summary"]?["transitions"]);
        Assert.Equal(3, transitions.Count);
        Assert.Equal(9902, transitions[0]["targets"]?[0]?["targetMapId"]?.Value<int>());
        Assert.Equal(1, transitions[0]["targets"]?[0]?["targetPointId"]?.Value<int>());
        Assert.Equal(80, transitions[0]["boxExtent"]?["x"]?.Value<double>());
        Assert.Equal(120, transitions[0]["boxExtent"]?["y"]?.Value<double>());
        Assert.Equal(60, transitions[0]["boxExtent"]?["z"]?.Value<double>());
        Assert.Equal(2, Assert.IsType<JArray>(transitions[1]["targets"]).Count);
        Assert.Equal("Sample_After", transitions[1]["targets"]?[0]?["afterTransitionEvents"]?[0]?["eventId"]?.Value<string>());
        Assert.Equal(true, transitions[2]["isSameMap"]?.Value<bool>());
        Assert.Equal(9901, transitions[2]["targets"]?[0]?["targetMapId"]?.Value<int>());
        Assert.Equal(5, transitions[2]["targets"]?[0]?["targetPointId"]?.Value<int>());

        var tps = Assert.IsType<JArray>(gdDoc["summary"]?["transitionPoints"]);
        Assert.Single(tps);
        Assert.Equal(0, tps[0]["pointId"]?.Value<int>());
        Assert.Equal("SampleEntry", tps[0]["pointName"]?.Value<string>());

        var chests = Assert.IsType<JArray>(gdDoc["summary"]?["treasureBoxes"]);
        Assert.Equal(2, chests.Count);
        Assert.Equal(11, chests[0]["itemId"]?.Value<int>());
        Assert.Equal(2, chests[0]["itemNum"]?.Value<int>());
        Assert.Equal(JTokenType.Null, chests[1]["itemId"]?.Type);
        Assert.Equal(500, chests[1]["money"]?.Value<int>());
        Assert.Equal(true, chests[1]["isTrap"]?.Value<bool>());
        Assert.Equal(77, chests[1]["battleGroupId"]?.Value<int>());

        var savePoints = Assert.IsType<JArray>(gdDoc["summary"]?["savePoints"]);
        Assert.Single(savePoints);
        Assert.Equal(true, savePoints[0]["isTeleport"]?.Value<bool>());
        Assert.Equal(9902, savePoints[0]["targetMapId"]?.Value<int>());
        Assert.Equal(10000, savePoints[0]["targetPointId"]?.Value<int>());

        var gimmicks = Assert.IsType<JArray>(gdDoc["summary"]?["gimmicks"]);
        Assert.Single(gimmicks);
        Assert.Equal("Blocking", gimmicks[0]["category"]?.Value<string>());

        var encounters = Assert.IsType<JArray>(gdDoc["summary"]?["encounterZones"]);
        Assert.Equal(2, encounters.Count);
        Assert.Equal("Box", encounters[0]["kind"]?.Value<string>());
        Assert.Equal("/Game/Datatable/Encount/DT_Sample_Encount", encounters[0]["encountDatatable"]?.Value<string>());
        Assert.Single(Assert.IsType<JArray>(encounters[0]["terrainDatatables"]));
        Assert.Equal("OverwriteBox", encounters[1]["kind"]?.Value<string>());
        Assert.Equal(0.0, encounters[1]["overwriteRatio"]?.Value<double>());
        Assert.Equal(true, encounters[1]["isResetDistance"]?.Value<bool>());

        // Inspect EV level summary
        var evDoc = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "Dng_99Sample_01_EV.json")));
        var mapUnits = Assert.IsType<JArray>(evDoc["summary"]?["mapUnits"]);
        Assert.Equal(4, mapUnits.Count);
        Assert.Equal(3, mapUnits.Count(u => u["kind"]?.Value<string>() == "npc"));
        Assert.Equal(1, mapUnits.Count(u => u["kind"]?.Value<string>() == "object"));

        var shops = Assert.IsType<JArray>(evDoc["summary"]?["shops"]);
        Assert.Equal(2, shops.Count);
        Assert.Equal(42, shops[0]["shopId"]?.Value<int>());
        Assert.Equal(false, shops[0]["isVariousShop"]?.Value<bool>());
        Assert.Equal(10005, shops[1]["shopId"]?.Value<int>());
        Assert.Equal(true, shops[1]["isVariousShop"]?.Value<bool>());

        var inns = Assert.IsType<JArray>(evDoc["summary"]?["inns"]);
        Assert.Single(inns);
        Assert.Equal(0, inns[0]["innId"]?.Value<int>());

        var eventTriggers = Assert.IsType<JArray>(evDoc["summary"]?["eventTriggers"]);
        Assert.Equal(2, eventTriggers.Count);
        Assert.Equal("Sample_TriggerEv", eventTriggers[0]["events"]?[0]?["eventId"]?.Value<string>());
        Assert.Equal("Sample_InteractEv", eventTriggers[1]["interactEventId"]?.Value<string>());

        // Engine-only BG level has empty summary buckets
        var bgDoc = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "Dng_99Sample_01_01_BG.json")));
        Assert.Equal("BG", bgDoc["level"]?["sublevelType"]?.Value<string>());
        Assert.Equal(1, bgDoc["level"]?["engineActorCount"]?.Value<int>());
        Assert.Equal(0, bgDoc["level"]?["gameActorCount"]?.Value<int>());
        Assert.Empty(Assert.IsType<JArray>(bgDoc["summary"]?["mapUnits"]));
        Assert.Empty(Assert.IsType<JArray>(bgDoc["summary"]?["gimmicks"]));

        // Inspect aggregated _graph.json
        var graph = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "_graph.json")));
        Assert.Equal(2, graph["stats"]?["mappedMapCount"]?.Value<int>());
        Assert.Equal(0, graph["stats"]?["unmappedLevelCount"]?.Value<int>());
        Assert.Equal(3, graph["stats"]?["totalNpcs"]?.Value<int>());
        Assert.Equal(4, graph["stats"]?["totalMapUnits"]?.Value<int>());
        Assert.Equal(2, graph["stats"]?["totalTreasureBoxes"]?.Value<int>());
        Assert.Equal(2, graph["stats"]?["totalShops"]?.Value<int>());
        Assert.Equal(2, graph["stats"]?["uniqueShopIds"]?.Value<int>());
        Assert.Equal(2, graph["stats"]?["totalSavePoints"]?.Value<int>());
        Assert.Equal(2, graph["stats"]?["savePointTeleportExits"]?.Value<int>());
        Assert.Equal([42, 10005], Assert.IsType<JArray>(graph["shopsByMap"]?["9901"]).Values<int>().ToArray());
        Assert.Equal([9901], Assert.IsType<JArray>(graph["shopToMaps"]?["42"]).Values<int>().ToArray());
        Assert.Equal([9901], Assert.IsType<JArray>(graph["shopToMaps"]?["10005"]).Values<int>().ToArray());

        var map9901 = Assert.IsType<JObject>(graph["maps"]?["9901"]);
        Assert.Equal(3, map9901["counts"]?["npcs"]?.Value<int>());
        Assert.Equal(2, map9901["counts"]?["treasureBoxes"]?.Value<int>());
        Assert.Equal(2, map9901["counts"]?["shops"]?.Value<int>());
        Assert.Equal(1, map9901["counts"]?["savePoints"]?.Value<int>());
        Assert.Equal(2, map9901["counts"]?["eventTriggers"]?.Value<int>());
        var exits9901 = Assert.IsType<JArray>(map9901["exits"]);
        Assert.Equal(5, exits9901.Count);
        Assert.Equal(1, exits9901.Count(e => e["kind"]?.Value<string>() == "savepoint_teleport"));

        var map9902 = Assert.IsType<JObject>(graph["maps"]?["9902"]);
        var exits9902 = Assert.IsType<JArray>(map9902["exits"]);
        Assert.Single(exits9902);
        Assert.Equal("savepoint_teleport", exits9902[0]["kind"]?.Value<string>());
        Assert.Equal(9902, exits9902[0]["targetMapId"]?.Value<int>());
        Assert.Equal(true, exits9902[0]["isSameMap"]?.Value<bool>());

        // 2. Resume without --force skips all 6 files and rebuilds complete graph
        var resumed = await TestHarness.RunCliAsync(
            ["export-maps", "--from-json", levelsDir, "--map-data", mapDataPath],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, resumed.ExitCode);
        var manifest2 = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "manifest.json")));
        Assert.Equal(0, manifest2["exportedCount"]?.Value<int>());
        Assert.Equal(6, manifest2["skippedCount"]?.Value<int>());
        var graph2 = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "_graph.json")));
        Assert.Equal(2, graph2["stats"]?["mappedMapCount"]?.Value<int>());
        Assert.Equal(5, Assert.IsType<JArray>(graph2["maps"]?["9901"]?["exits"]).Count);

        // 3. --force re-exports all 6 files
        var forced = await TestHarness.RunCliAsync(
            ["export-maps", "--from-json", levelsDir, "--map-data", mapDataPath, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(0, forced.ExitCode);
        var manifest3 = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "manifest.json")));
        Assert.Equal(6, manifest3["exportedCount"]?.Value<int>());
        Assert.Equal(0, manifest3["skippedCount"]?.Value<int>());
    }

    [Fact]
    public async Task ExportMaps_FromJsonFixture_InvalidSingleFileDoesNotAbortOthers()
    {
        using var workspace = new TempWorkspace("ffr-map-partial-fail");
        var fixtureRoot = Path.Combine(workspace.Root, "fixture");
        var (levelsDir, mapDataPath) = await WriteSyntheticMapFixtureAsync(fixtureRoot);
        await File.WriteAllTextAsync(Path.Combine(levelsDir, "Broken_Level.json"), "{ not valid json");

        var outDir = Path.Combine(workspace.DepsDir, "ffr", "export-full", "maps");
        var result = await TestHarness.RunCliAsync(
            ["export-maps", "--from-json", levelsDir, "--map-data", mapDataPath, "--out", outDir],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(1, result.ExitCode);

        var manifest = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "manifest.json")));
        Assert.Equal(7, manifest["totalLevels"]?.Value<int>());
        Assert.Equal(6, manifest["exportedCount"]?.Value<int>());
        Assert.Equal(1, manifest["failureCount"]?.Value<int>());
        Assert.Single(Assert.IsType<JArray>(manifest["failures"]));

        var graph = JObject.Parse(await File.ReadAllTextAsync(Path.Combine(outDir, "_graph.json")));
        Assert.Equal(2, graph["stats"]?["mappedMapCount"]?.Value<int>());
        Assert.Equal(1, graph["stats"]?["failedLevels"]?.Value<int>());
    }

    [Fact]
    public async Task ExportMaps_EnforcesOutputDirectoryGuardrails()
    {
        using var workspace = new TempWorkspace("ffr-map-guards");

        // Outside .deps/ when FFR_DEPS is unset
        var outside = await TestHarness.RunCliAsync(
            ["export-maps", "--out", Path.Combine(workspace.Root, "outside-maps")],
            ffrDeps: null,
            workingDirectory: workspace.Root);
        Assert.Equal(2, outside.ExitCode);
        Assert.Contains(".deps/", outside.Stderr, StringComparison.Ordinal);

        // Protected ffr/files directory
        var protectedDir = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "export-maps", "--out", Path.Combine(workspace.DepsDir, "ffr", "files")],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, protectedDir.ExitCode);
        Assert.Contains("受保护目录", protectedDir.Stderr, StringComparison.Ordinal);

        // Foreign non-empty directory refused with --force
        var emptyFixture = Path.Combine(workspace.Root, "empty-fixture");
        Directory.CreateDirectory(emptyFixture);
        var foreignDir = Path.Combine(workspace.DepsDir, "tmp", "foreign-maps");
        Directory.CreateDirectory(foreignDir);
        await File.WriteAllTextAsync(Path.Combine(foreignDir, "other.txt"), "not ffr-export");

        var refused = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "export-maps", "--from-json", emptyFixture, "--out", foreignDir, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, refused.ExitCode);
        Assert.Contains("不是 ffr-export 的输出目录", refused.Stderr, StringComparison.Ordinal);
        Assert.Equal("not ffr-export", await File.ReadAllTextAsync(Path.Combine(foreignDir, "other.txt")));

        // Wrong-command manifest refused with --force
        var inventoryDir = Path.Combine(workspace.DepsDir, "tmp", "inv-dir");
        Directory.CreateDirectory(inventoryDir);
        await File.WriteAllTextAsync(
            Path.Combine(inventoryDir, "manifest.json"),
            JsonConvert.SerializeObject(new { cue4ParseVersion = "1.2.2.202611", command = "inventory" }));
        var wrongCmd = await TestHarness.RunCliAsync(
            ["--lang", "zh-Hans", "export-maps", "--from-json", emptyFixture, "--out", inventoryDir, "--force"],
            ffrDeps: workspace.DepsDir);
        Assert.Equal(2, wrongCmd.ExitCode);
        Assert.Contains("不能由 export-maps --force 清空", wrongCmd.Stderr, StringComparison.Ordinal);
    }

    [Theory]
    [InlineData("Dng_01Gra_43_PL", null, "PL")]
    [InlineData("Dng_01Gra_43_EV_01", null, "EV")]
    [InlineData("Dng_01Gra_43_GD", null, "GD")]
    [InlineData("Dng_01Gra_43_01_BG0", null, "BG")]
    [InlineData("Dng_01Gra_43_LT_02", null, "LT")]
    [InlineData("Dng_01Gra_43_KD", null, "KD")]
    [InlineData("Dng_01Gra_43_SD", null, "SD")]
    [InlineData("Wld_FG", null, "FG")]
    [InlineData("Dng_01Gra_43_NAV", null, "NAV")]
    [InlineData("Test_MOCK", null, "MOCK")]
    [InlineData("Wld_COL", null, "COL")]
    [InlineData("CustomSublevel", "FFRS/Content/Map/Dng/01/GD/CustomSublevel.umap", "GD")]
    [InlineData("MUCHA_Title", "FFRS/Content/Map/Title/MUCHA_Title.umap", "OTHER")]
    public void MapExtractor_ClassifySublevelType_HandlesSuffixesAndFolderFallback(
        string levelName,
        string? filePath,
        string expected)
    {
        Assert.Equal(expected, MapExtractor.ClassifySublevelType(levelName, filePath));
    }

    [Fact]
    public void MapHelpers_ConvertPathsAndDisambiguateCollidingFilenames()
    {
        Assert.Equal(
            "/Game/Map/Dng/01/Dng_01_PL",
            MapExtractor.ToGamePackagePath("FFRS/Content/Map/Dng/01/Dng_01_PL.umap"));
        Assert.Equal(
            "Dng_01_PL",
            MapExtractor.ExtractLevelStem("/Game/Map/Dng/01/Dng_01_PL.Dng_01_PL:PersistentLevel"));
        Assert.True(MapExtractor.IsGameClass("/Script/mucha.CPP_MapUnit_NPC"));
        Assert.True(MapExtractor.IsGameClass("/Game/BP/Map/BP_Sample.BP_Sample_C"));
        Assert.False(MapExtractor.IsGameClass("/Script/Engine.StaticMeshActor"));

        var filenames = Program.AssignMapOutputFilenames([
            "FFRS/Content/Map/Dng/01Gra/01Gra_44/KD/Dng_01Gra_44_01_KD.umap",
            "FFRS/Content/Map/Dng/05Old/05Old_45/KD/Dng_01Gra_44_01_KD.umap",
            "FFRS/Content/Map/Dng/01Gra/01Gra_44/Dng_01Gra_44_PL.umap",
        ]);
        Assert.Equal("Dng_01Gra_44_PL.json", filenames["FFRS/Content/Map/Dng/01Gra/01Gra_44/Dng_01Gra_44_PL.umap"]);
        Assert.Equal("Dng_01Gra_44_01_KD.json", filenames["FFRS/Content/Map/Dng/01Gra/01Gra_44/KD/Dng_01Gra_44_01_KD.umap"]);
        Assert.Equal("05Old_45__Dng_01Gra_44_01_KD.json", filenames["FFRS/Content/Map/Dng/05Old/05Old_45/KD/Dng_01Gra_44_01_KD.umap"]);
    }
}
