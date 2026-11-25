using System.Globalization;
using Newtonsoft.Json;

namespace FfrExport;

internal sealed record GraphSublevelRefDto(
    [property: JsonProperty("name")] string Name,
    [property: JsonProperty("type")] string Type,
    [property: JsonProperty("outputFile")] string OutputFile);

internal sealed class GraphMapCountsDto
{
    [JsonProperty("npcs")]
    public int Npcs { get; set; }

    [JsonProperty("mapUnits")]
    public int MapUnits { get; set; }

    [JsonProperty("treasureBoxes")]
    public int TreasureBoxes { get; set; }

    [JsonProperty("shops")]
    public int Shops { get; set; }

    [JsonProperty("inns")]
    public int Inns { get; set; }

    [JsonProperty("savePoints")]
    public int SavePoints { get; set; }

    [JsonProperty("eventTriggers")]
    public int EventTriggers { get; set; }

    [JsonProperty("gimmicks")]
    public int Gimmicks { get; set; }

    [JsonProperty("encounterZones")]
    public int EncounterZones { get; set; }

    [JsonProperty("transitions")]
    public int Transitions { get; set; }

    [JsonProperty("transitionPoints")]
    public int TransitionPoints { get; set; }
}

internal sealed class GraphExitDto
{
    [JsonProperty("kind")]
    public required string Kind { get; init; }

    [JsonProperty("sourceLevel")]
    public required string SourceLevel { get; init; }

    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("targetMapId")]
    public int? TargetMapId { get; init; }

    [JsonProperty("targetPointId")]
    public int? TargetPointId { get; init; }

    [JsonProperty("isSameMap")]
    public bool IsSameMap { get; init; }

    [JsonProperty("isAutoTransition")]
    public bool IsAutoTransition { get; init; }

    [JsonProperty("isEventOnly")]
    public bool IsEventOnly { get; init; }

    [JsonProperty("condition")]
    public string? Condition { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GraphTransitionPointDto
{
    [JsonProperty("sourceLevel")]
    public required string SourceLevel { get; init; }

    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("pointId")]
    public int PointId { get; init; }

    [JsonProperty("direction")]
    public string? Direction { get; init; }

    [JsonProperty("pointName")]
    public string? PointName { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GraphShopDto
{
    [JsonProperty("sourceLevel")]
    public required string SourceLevel { get; init; }

    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("mapUnitId")]
    public int? MapUnitId { get; init; }

    [JsonProperty("shopId")]
    public int ShopId { get; init; }

    [JsonProperty("isVariousShop")]
    public bool IsVariousShop { get; init; }

    [JsonProperty("isTest")]
    public bool IsTest { get; init; }

    [JsonProperty("interactType")]
    public string? InteractType { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GraphSavePointDto
{
    [JsonProperty("sourceLevel")]
    public required string SourceLevel { get; init; }

    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("savePointType")]
    public string? SavePointType { get; init; }

    [JsonProperty("isTeleport")]
    public bool IsTeleport { get; init; }

    [JsonProperty("targetMapId")]
    public int? TargetMapId { get; init; }

    [JsonProperty("targetPointId")]
    public int? TargetPointId { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GraphTreasureBoxDto
{
    [JsonProperty("sourceLevel")]
    public required string SourceLevel { get; init; }

    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("chestId")]
    public int? ChestId { get; init; }

    [JsonProperty("itemId")]
    public int? ItemId { get; init; }

    [JsonProperty("money")]
    public int? Money { get; init; }

    [JsonProperty("isTrap")]
    public bool? IsTrap { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GraphMapNodeDto
{
    [JsonProperty("mapId")]
    public int MapId { get; init; }

    [JsonProperty("mapRowKey")]
    public string? MapRowKey { get; set; }

    [JsonProperty("mapType")]
    public string? MapType { get; set; }

    [JsonProperty("persistentLevel")]
    public string? PersistentLevel { get; set; }

    [JsonProperty("sublevels")]
    public required List<GraphSublevelRefDto> Sublevels { get; init; }

    [JsonProperty("counts")]
    public required GraphMapCountsDto Counts { get; init; }

    [JsonProperty("exits")]
    public required List<GraphExitDto> Exits { get; init; }

    [JsonProperty("transitionPoints")]
    public required List<GraphTransitionPointDto> TransitionPoints { get; init; }

    [JsonProperty("shops")]
    public required List<GraphShopDto> Shops { get; init; }

    [JsonProperty("savePoints")]
    public required List<GraphSavePointDto> SavePoints { get; init; }

    [JsonProperty("treasureBoxes")]
    public required List<GraphTreasureBoxDto> TreasureBoxes { get; init; }
}

internal sealed record GraphUnmappedLevelDto(
    [property: JsonProperty("name")] string Name,
    [property: JsonProperty("type")] string Type,
    [property: JsonProperty("path")] string Path,
    [property: JsonProperty("outputFile")] string OutputFile);

internal sealed class GraphStatsDto
{
    [JsonProperty("totalLevels")]
    public int TotalLevels { get; init; }

    [JsonProperty("exportedLevels")]
    public int ExportedLevels { get; init; }

    [JsonProperty("failedLevels")]
    public int FailedLevels { get; init; }

    [JsonProperty("sublevelTypeCounts")]
    public required SortedDictionary<string, int> SublevelTypeCounts { get; init; }

    [JsonProperty("mappedMapCount")]
    public int MappedMapCount { get; init; }

    [JsonProperty("unmappedLevelCount")]
    public int UnmappedLevelCount { get; init; }

    [JsonProperty("totalExits")]
    public int TotalExits { get; init; }

    [JsonProperty("crossMapExits")]
    public int CrossMapExits { get; init; }

    [JsonProperty("sameMapExits")]
    public int SameMapExits { get; init; }

    [JsonProperty("savePointTeleportExits")]
    public int SavePointTeleportExits { get; init; }

    [JsonProperty("totalTransitionPoints")]
    public int TotalTransitionPoints { get; init; }

    [JsonProperty("totalNpcs")]
    public int TotalNpcs { get; init; }

    [JsonProperty("totalMapUnits")]
    public int TotalMapUnits { get; init; }

    [JsonProperty("totalTreasureBoxes")]
    public int TotalTreasureBoxes { get; init; }

    [JsonProperty("totalSavePoints")]
    public int TotalSavePoints { get; init; }

    [JsonProperty("totalShops")]
    public int TotalShops { get; init; }

    [JsonProperty("uniqueShopIds")]
    public int UniqueShopIds { get; init; }

    [JsonProperty("mapsWithShops")]
    public int MapsWithShops { get; init; }

    [JsonProperty("totalInns")]
    public int TotalInns { get; init; }

    [JsonProperty("totalEventTriggers")]
    public int TotalEventTriggers { get; init; }

    [JsonProperty("totalGimmicks")]
    public int TotalGimmicks { get; init; }

    [JsonProperty("totalEncounterZones")]
    public int TotalEncounterZones { get; init; }
}

internal sealed class MapGraphDocumentDto
{
    [JsonProperty("schemaVersion")]
    public int SchemaVersion { get; init; } = 1;

    [JsonProperty("generatedAt")]
    public required string GeneratedAt { get; init; }

    [JsonProperty("stats")]
    public required GraphStatsDto Stats { get; init; }

    [JsonProperty("maps")]
    public required Dictionary<string, GraphMapNodeDto> Maps { get; init; }

    [JsonProperty("shopsByMap")]
    public required Dictionary<string, List<int>> ShopsByMap { get; init; }

    [JsonProperty("shopToMaps")]
    public required Dictionary<string, List<int>> ShopToMaps { get; init; }

    [JsonProperty("unmappedLevels")]
    public required List<GraphUnmappedLevelDto> UnmappedLevels { get; init; }
}

internal sealed record LevelGraphContribution(
    LevelMetadataDto Level,
    LevelSummaryDto Summary);

internal static class MapGraphBuilder
{
    public static MapGraphDocumentDto Build(
        IReadOnlyList<LevelGraphContribution> levels,
        IReadOnlyDictionary<int, MapDataEntry> mapDataById,
        int totalLevels,
        int failedLevels)
    {
        var sublevelTypeCounts = new SortedDictionary<string, int>(StringComparer.Ordinal);
        var mapNodes = new SortedDictionary<int, GraphMapNodeDto>();
        var unmapped = new List<GraphUnmappedLevelDto>();

        var sortedLevels = levels
            .OrderBy(item => item.Level.Path, StringComparer.Ordinal)
            .ThenBy(item => item.Level.Name, StringComparer.Ordinal)
            .ToArray();

        foreach (var entry in sortedLevels)
        {
            var level = entry.Level;
            var summary = entry.Summary;
            sublevelTypeCounts[level.SublevelType] = sublevelTypeCounts.GetValueOrDefault(level.SublevelType) + 1;

            var targetMapIds = level.MapIds.Count > 0
                ? level.MapIds
                : (level.MapId is not null ? [level.MapId.Value] : []);

            if (targetMapIds.Count == 0)
            {
                unmapped.Add(new GraphUnmappedLevelDto(
                    level.Name,
                    level.SublevelType,
                    level.Path,
                    level.OutputFile));
                continue;
            }

            foreach (var mapId in targetMapIds.Distinct().OrderBy(id => id))
            {
                if (!mapNodes.TryGetValue(mapId, out var node))
                {
                    mapDataById.TryGetValue(mapId, out var dataEntry);
                    node = new GraphMapNodeDto
                    {
                        MapId = mapId,
                        MapRowKey = dataEntry?.RowKey ?? level.MapRowKey,
                        MapType = dataEntry?.MapType ?? level.MapType,
                        PersistentLevel = dataEntry?.LevelName ?? level.PersistentLevel,
                        Sublevels = [],
                        Counts = new GraphMapCountsDto(),
                        Exits = [],
                        TransitionPoints = [],
                        Shops = [],
                        SavePoints = [],
                        TreasureBoxes = [],
                    };
                    mapNodes[mapId] = node;
                }

                node.Sublevels.Add(new GraphSublevelRefDto(level.Name, level.SublevelType, level.OutputFile));
                if (level.SublevelType == "PL" && node.PersistentLevel is null)
                {
                    node.PersistentLevel = level.Name;
                }

                var npcCount = summary.MapUnits.Count(unit => unit.Kind == "npc");
                node.Counts.Npcs += npcCount;
                node.Counts.MapUnits += summary.MapUnits.Count;
                node.Counts.TreasureBoxes += summary.TreasureBoxes.Count;
                node.Counts.Shops += summary.Shops.Count;
                node.Counts.Inns += summary.Inns.Count;
                node.Counts.SavePoints += summary.SavePoints.Count;
                node.Counts.EventTriggers += summary.EventTriggers.Count;
                node.Counts.Gimmicks += summary.Gimmicks.Count;
                node.Counts.EncounterZones += summary.EncounterZones.Count;
                node.Counts.Transitions += summary.Transitions.Count;
                node.Counts.TransitionPoints += summary.TransitionPoints.Count;

                foreach (var transition in summary.Transitions)
                {
                    foreach (var target in transition.Targets)
                    {
                        var effectiveTargetMapId = target.TargetMapId ?? (transition.IsSameMap ? mapId : null);
                        var isSameMap = transition.IsSameMap || effectiveTargetMapId == mapId;
                        node.Exits.Add(new GraphExitDto
                        {
                            Kind = "transition_trigger",
                            SourceLevel = level.Name,
                            ActorName = transition.ActorName,
                            UniqueId = transition.UniqueId,
                            TargetMapId = effectiveTargetMapId,
                            TargetPointId = target.TargetPointId,
                            IsSameMap = isSameMap,
                            IsAutoTransition = transition.IsAutoTransition,
                            IsEventOnly = transition.IsEventOnly || target.IsEventOnly,
                            Condition = target.Condition ?? transition.Condition,
                            Location = transition.Location,
                        });
                    }
                }

                foreach (var sp in summary.SavePoints)
                {
                    var effectiveTargetMapId = sp.TargetMapId ?? (sp.IsTeleport ? mapId : null);
                    node.SavePoints.Add(new GraphSavePointDto
                    {
                        SourceLevel = level.Name,
                        ActorName = sp.ActorName,
                        UniqueId = sp.UniqueId,
                        SavePointType = sp.SavePointType,
                        IsTeleport = sp.IsTeleport,
                        TargetMapId = effectiveTargetMapId,
                        TargetPointId = sp.TargetPointId,
                        Location = sp.Location,
                    });

                    if (sp.IsTeleport && effectiveTargetMapId is not null)
                    {
                        node.Exits.Add(new GraphExitDto
                        {
                            Kind = "savepoint_teleport",
                            SourceLevel = level.Name,
                            ActorName = sp.ActorName,
                            UniqueId = sp.UniqueId,
                            TargetMapId = effectiveTargetMapId,
                            TargetPointId = sp.TargetPointId,
                            IsSameMap = effectiveTargetMapId == mapId,
                            IsAutoTransition = false,
                            IsEventOnly = false,
                            Condition = sp.VisibleCondition,
                            Location = sp.Location,
                        });
                    }
                }

                foreach (var tp in summary.TransitionPoints)
                {
                    node.TransitionPoints.Add(new GraphTransitionPointDto
                    {
                        SourceLevel = level.Name,
                        ActorName = tp.ActorName,
                        PointId = tp.PointId,
                        Direction = tp.Direction,
                        PointName = tp.PointName,
                        Location = tp.Location,
                    });
                }

                foreach (var shop in summary.Shops)
                {
                    node.Shops.Add(new GraphShopDto
                    {
                        SourceLevel = level.Name,
                        ActorName = shop.ActorName,
                        UniqueId = shop.UniqueId,
                        MapUnitId = shop.MapUnitId,
                        ShopId = shop.ShopId,
                        IsVariousShop = shop.IsVariousShop,
                        IsTest = shop.IsTest,
                        InteractType = shop.InteractType,
                        VisibleCondition = shop.VisibleCondition,
                        Location = shop.Location,
                    });
                }

                foreach (var tb in summary.TreasureBoxes)
                {
                    node.TreasureBoxes.Add(new GraphTreasureBoxDto
                    {
                        SourceLevel = level.Name,
                        ActorName = tb.ActorName,
                        UniqueId = tb.UniqueId,
                        ChestId = tb.ChestId,
                        ItemId = tb.ItemId,
                        Money = tb.Money,
                        IsTrap = tb.IsTrap,
                        Location = tb.Location,
                    });
                }
            }
        }

        // Build shop mappings and aggregate stats.
        var mapsDict = new Dictionary<string, GraphMapNodeDto>(mapNodes.Count, StringComparer.Ordinal);
        var shopsByMap = new Dictionary<string, List<int>>(StringComparer.Ordinal);
        var shopToMapsTemp = new SortedDictionary<int, SortedSet<int>>();

        var totalExits = 0;
        var crossMapExits = 0;
        var sameMapExits = 0;
        var savePointTeleportExits = 0;
        var totalTransitionPoints = 0;
        var totalNpcs = 0;
        var totalMapUnits = 0;
        var totalTreasureBoxes = 0;
        var totalSavePoints = 0;
        var totalShops = 0;
        var totalInns = 0;
        var totalEventTriggers = 0;
        var totalGimmicks = 0;
        var totalEncounterZones = 0;

        foreach (var (mapId, node) in mapNodes)
        {
            var key = mapId.ToString(CultureInfo.InvariantCulture);
            mapsDict[key] = node;

            totalExits += node.Exits.Count;
            foreach (var exit in node.Exits)
            {
                if (exit.Kind == "savepoint_teleport")
                {
                    savePointTeleportExits++;
                }

                if (exit.TargetMapId is not null)
                {
                    if (exit.IsSameMap || exit.TargetMapId == mapId)
                    {
                        sameMapExits++;
                    }
                    else
                    {
                        crossMapExits++;
                    }
                }
            }

            totalTransitionPoints += node.Counts.TransitionPoints;
            totalNpcs += node.Counts.Npcs;
            totalMapUnits += node.Counts.MapUnits;
            totalTreasureBoxes += node.Counts.TreasureBoxes;
            totalSavePoints += node.Counts.SavePoints;
            totalShops += node.Counts.Shops;
            totalInns += node.Counts.Inns;
            totalEventTriggers += node.Counts.EventTriggers;
            totalGimmicks += node.Counts.Gimmicks;
            totalEncounterZones += node.Counts.EncounterZones;

            if (node.Shops.Count > 0)
            {
                var ids = node.Shops.Select(s => s.ShopId).Distinct().OrderBy(id => id).ToList();
                shopsByMap[key] = ids;
                foreach (var shopId in ids)
                {
                    if (!shopToMapsTemp.TryGetValue(shopId, out var set))
                    {
                        set = [];
                        shopToMapsTemp[shopId] = set;
                    }

                    set.Add(mapId);
                }
            }
        }

        var shopToMaps = new Dictionary<string, List<int>>(shopToMapsTemp.Count, StringComparer.Ordinal);
        foreach (var (shopId, mapSet) in shopToMapsTemp)
        {
            shopToMaps[shopId.ToString(CultureInfo.InvariantCulture)] = [.. mapSet];
        }

        var stats = new GraphStatsDto
        {
            TotalLevels = totalLevels,
            ExportedLevels = sortedLevels.Length,
            FailedLevels = failedLevels,
            SublevelTypeCounts = sublevelTypeCounts,
            MappedMapCount = mapNodes.Count,
            UnmappedLevelCount = unmapped.Count,
            TotalExits = totalExits,
            CrossMapExits = crossMapExits,
            SameMapExits = sameMapExits,
            SavePointTeleportExits = savePointTeleportExits,
            TotalTransitionPoints = totalTransitionPoints,
            TotalNpcs = totalNpcs,
            TotalMapUnits = totalMapUnits,
            TotalTreasureBoxes = totalTreasureBoxes,
            TotalSavePoints = totalSavePoints,
            TotalShops = totalShops,
            UniqueShopIds = shopToMaps.Count,
            MapsWithShops = shopsByMap.Count,
            TotalInns = totalInns,
            TotalEventTriggers = totalEventTriggers,
            TotalGimmicks = totalGimmicks,
            TotalEncounterZones = totalEncounterZones,
        };

        return new MapGraphDocumentDto
        {
            GeneratedAt = DateTimeOffset.UtcNow.UtcDateTime.ToString("O", CultureInfo.InvariantCulture),
            Stats = stats,
            Maps = mapsDict,
            ShopsByMap = shopsByMap,
            ShopToMaps = shopToMaps,
            UnmappedLevels = unmapped,
        };
    }
}
