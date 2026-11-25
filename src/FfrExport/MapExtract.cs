using System.Text.RegularExpressions;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Exports;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.Assets.Objects.Properties;
using CUE4Parse.UE4.Objects.Core.Math;
using CUE4Parse.UE4.Objects.Engine;
using CUE4Parse.UE4.Objects.UObject;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal sealed record Vec3Dto(
    [property: JsonProperty("x")] double X,
    [property: JsonProperty("y")] double Y,
    [property: JsonProperty("z")] double Z);

internal sealed record Rot3Dto(
    [property: JsonProperty("pitch")] double Pitch,
    [property: JsonProperty("yaw")] double Yaw,
    [property: JsonProperty("roll")] double Roll);

internal sealed record StreamingLevelDto(
    [property: JsonProperty("name")] string Name,
    [property: JsonProperty("worldAsset")] string WorldAsset,
    [property: JsonProperty("streamingClass")] string StreamingClass);

internal sealed record MapDataEntry(
    int MapId,
    string RowKey,
    string LevelName,
    string LevelAssetPath,
    string MapType,
    IReadOnlyList<string> ProgressSublevelNames);

internal sealed class LevelMetadataDto
{
    [JsonProperty("name")]
    public required string Name { get; init; }

    [JsonProperty("outputFile")]
    public required string OutputFile { get; init; }

    [JsonProperty("path")]
    public required string Path { get; init; }

    [JsonProperty("packagePath")]
    public required string PackagePath { get; init; }

    [JsonProperty("sublevelType")]
    public required string SublevelType { get; init; }

    [JsonProperty("persistentLevel")]
    public string? PersistentLevel { get; init; }

    [JsonProperty("persistentLevels")]
    public required List<string> PersistentLevels { get; init; }

    [JsonProperty("mapId")]
    public int? MapId { get; init; }

    [JsonProperty("mapIds")]
    public required List<int> MapIds { get; init; }

    [JsonProperty("mapRowKey")]
    public string? MapRowKey { get; init; }

    [JsonProperty("mapType")]
    public string? MapType { get; init; }

    [JsonProperty("streamingLevels")]
    public required List<StreamingLevelDto> StreamingLevels { get; init; }

    [JsonProperty("actorCount")]
    public int ActorCount { get; set; }

    [JsonProperty("gameActorCount")]
    public int GameActorCount { get; set; }

    [JsonProperty("engineActorCount")]
    public int EngineActorCount { get; set; }
}

internal sealed class SubobjectDto
{
    [JsonProperty("name")]
    public required string Name { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("isGameClass")]
    public bool IsGameClass { get; init; }

    [JsonProperty("location", NullValueHandling = NullValueHandling.Ignore)]
    public Vec3Dto? Location { get; init; }

    [JsonProperty("rotation", NullValueHandling = NullValueHandling.Ignore)]
    public Rot3Dto? Rotation { get; init; }

    [JsonProperty("scale", NullValueHandling = NullValueHandling.Ignore)]
    public Vec3Dto? Scale { get; init; }

    [JsonProperty("boxExtent", NullValueHandling = NullValueHandling.Ignore)]
    public Vec3Dto? BoxExtent { get; init; }

    [JsonProperty("sphereRadius", NullValueHandling = NullValueHandling.Ignore)]
    public double? SphereRadius { get; init; }

    [JsonProperty("capsuleHalfHeight", NullValueHandling = NullValueHandling.Ignore)]
    public double? CapsuleHalfHeight { get; init; }

    [JsonProperty("capsuleRadius", NullValueHandling = NullValueHandling.Ignore)]
    public double? CapsuleRadius { get; init; }

    [JsonProperty("assetRefs", NullValueHandling = NullValueHandling.Ignore)]
    public List<string>? AssetRefs { get; init; }

    [JsonProperty("properties", NullValueHandling = NullValueHandling.Ignore)]
    public JObject? Properties { get; init; }
}

internal sealed class ActorDto
{
    [JsonProperty("name")]
    public required string Name { get; init; }

    [JsonProperty("label", NullValueHandling = NullValueHandling.Ignore)]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("isGameClass")]
    public bool IsGameClass { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }

    [JsonProperty("rotation")]
    public Rot3Dto? Rotation { get; init; }

    [JsonProperty("scale")]
    public Vec3Dto? Scale { get; init; }

    [JsonProperty("assetRefs", NullValueHandling = NullValueHandling.Ignore)]
    public List<string>? AssetRefs { get; init; }

    [JsonProperty("properties", NullValueHandling = NullValueHandling.Ignore)]
    public JObject? Properties { get; init; }

    [JsonProperty("subobjects", NullValueHandling = NullValueHandling.Ignore)]
    public List<SubobjectDto>? Subobjects { get; init; }
}

internal sealed record EventEntryDto(
    [property: JsonProperty("condition")] string? Condition,
    [property: JsonProperty("eventId")] string EventId);

internal sealed class MapUnitSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("kind")]
    public required string Kind { get; init; }

    [JsonProperty("mapUnitId")]
    public int? MapUnitId { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("direction")]
    public string? Direction { get; init; }

    [JsonProperty("interactType")]
    public string? InteractType { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("shopId")]
    public int? ShopId { get; init; }

    [JsonProperty("innId")]
    public int? InnId { get; init; }

    [JsonProperty("events")]
    public required List<EventEntryDto> Events { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class TreasureBoxSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("chestId")]
    public int? ChestId { get; init; }

    [JsonProperty("mapId")]
    public int? MapId { get; init; }

    [JsonProperty("itemId")]
    public int? ItemId { get; init; }

    [JsonProperty("itemNum")]
    public int? ItemNum { get; init; }

    [JsonProperty("money")]
    public int? Money { get; init; }

    [JsonProperty("isTrap")]
    public bool? IsTrap { get; init; }

    [JsonProperty("battleGroupId")]
    public int? BattleGroupId { get; init; }

    [JsonProperty("battleFinishEventId")]
    public string? BattleFinishEventId { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class EventTriggerSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("interactType")]
    public string? InteractType { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("flagCondition")]
    public string? FlagCondition { get; init; }

    [JsonProperty("interactEventId")]
    public string? InteractEventId { get; init; }

    [JsonProperty("events")]
    public required List<EventEntryDto> Events { get; init; }

    [JsonProperty("boxExtent")]
    public Vec3Dto? BoxExtent { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class TransitionTargetDto
{
    [JsonProperty("condition")]
    public string? Condition { get; init; }

    [JsonProperty("targetMapId")]
    public int? TargetMapId { get; init; }

    [JsonProperty("targetPointId")]
    public int? TargetPointId { get; init; }

    [JsonProperty("isEventOnly")]
    public bool IsEventOnly { get; init; }

    [JsonProperty("events")]
    public required List<EventEntryDto> Events { get; init; }

    [JsonProperty("afterTransitionEvents")]
    public required List<EventEntryDto> AfterTransitionEvents { get; init; }
}

internal sealed class TransitionSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("isSameMap")]
    public bool IsSameMap { get; init; }

    [JsonProperty("isAutoTransition")]
    public bool IsAutoTransition { get; init; }

    [JsonProperty("isEventOnly")]
    public bool IsEventOnly { get; init; }

    [JsonProperty("condition")]
    public string? Condition { get; init; }

    [JsonProperty("targets")]
    public required List<TransitionTargetDto> Targets { get; init; }

    [JsonProperty("boxExtent")]
    public Vec3Dto? BoxExtent { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class TransitionPointSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("pointId")]
    public int PointId { get; init; }

    [JsonProperty("direction")]
    public string? Direction { get; init; }

    [JsonProperty("pointName")]
    public string? PointName { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class SavePointSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("savePointType")]
    public string? SavePointType { get; init; }

    [JsonProperty("isTeleport")]
    public bool IsTeleport { get; init; }

    [JsonProperty("transitionMapId")]
    public int? TransitionMapId { get; init; }

    [JsonProperty("transitionPointId")]
    public int? TransitionPointId { get; init; }

    [JsonProperty("parentMapId")]
    public int? ParentMapId { get; init; }

    [JsonProperty("parentUniqueId")]
    public int? ParentUniqueId { get; init; }

    [JsonProperty("targetMapId")]
    public int? TargetMapId { get; init; }

    [JsonProperty("targetPointId")]
    public int? TargetPointId { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class ShopSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

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

internal sealed class InnSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("mapUnitId")]
    public int? MapUnitId { get; init; }

    [JsonProperty("innId")]
    public int InnId { get; init; }

    [JsonProperty("interactType")]
    public string? InteractType { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class GimmickSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("category")]
    public required string Category { get; init; }

    [JsonProperty("uniqueId")]
    public int? UniqueId { get; init; }

    [JsonProperty("interactType")]
    public string? InteractType { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("flagCondition")]
    public string? FlagCondition { get; init; }

    [JsonProperty("events")]
    public required List<EventEntryDto> Events { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class EncounterTerrainDto
{
    [JsonProperty("terrain")]
    public string? Terrain { get; init; }

    [JsonProperty("datatable")]
    public string? Datatable { get; init; }
}

internal sealed class EncounterZoneSummaryDto
{
    [JsonProperty("actorName")]
    public required string ActorName { get; init; }

    [JsonProperty("label")]
    public string? Label { get; init; }

    [JsonProperty("class")]
    public required string Class { get; init; }

    [JsonProperty("kind")]
    public required string Kind { get; init; }

    [JsonProperty("id")]
    public int? Id { get; init; }

    [JsonProperty("priority")]
    public int? Priority { get; init; }

    [JsonProperty("terrainId")]
    public int? TerrainId { get; init; }

    [JsonProperty("battleStageId")]
    public int? BattleStageId { get; init; }

    [JsonProperty("overwriteRatio")]
    public double? OverwriteRatio { get; init; }

    [JsonProperty("isResetDistance")]
    public bool? IsResetDistance { get; init; }

    [JsonProperty("visibleCondition")]
    public string? VisibleCondition { get; init; }

    [JsonProperty("flagCondition")]
    public string? FlagCondition { get; init; }

    [JsonProperty("encountDatatable")]
    public string? EncountDatatable { get; init; }

    [JsonProperty("terrainDatatables")]
    public required List<EncounterTerrainDto> TerrainDatatables { get; init; }

    [JsonProperty("boxExtent")]
    public Vec3Dto? BoxExtent { get; init; }

    [JsonProperty("sphereRadius")]
    public double? SphereRadius { get; init; }

    [JsonProperty("location")]
    public Vec3Dto? Location { get; init; }
}

internal sealed class LevelSummaryDto
{
    [JsonProperty("mapUnits")]
    public required List<MapUnitSummaryDto> MapUnits { get; init; }

    [JsonProperty("treasureBoxes")]
    public required List<TreasureBoxSummaryDto> TreasureBoxes { get; init; }

    [JsonProperty("eventTriggers")]
    public required List<EventTriggerSummaryDto> EventTriggers { get; init; }

    [JsonProperty("transitions")]
    public required List<TransitionSummaryDto> Transitions { get; init; }

    [JsonProperty("transitionPoints")]
    public required List<TransitionPointSummaryDto> TransitionPoints { get; init; }

    [JsonProperty("savePoints")]
    public required List<SavePointSummaryDto> SavePoints { get; init; }

    [JsonProperty("shops")]
    public required List<ShopSummaryDto> Shops { get; init; }

    [JsonProperty("inns")]
    public required List<InnSummaryDto> Inns { get; init; }

    [JsonProperty("gimmicks")]
    public required List<GimmickSummaryDto> Gimmicks { get; init; }

    [JsonProperty("encounterZones")]
    public required List<EncounterZoneSummaryDto> EncounterZones { get; init; }
}

internal sealed class MapExportDocumentDto
{
    [JsonProperty("schemaVersion")]
    public int SchemaVersion { get; init; } = 1;

    [JsonProperty("level")]
    public required LevelMetadataDto Level { get; init; }

    [JsonProperty("actors")]
    public required List<ActorDto> Actors { get; init; }

    [JsonProperty("summary")]
    public required LevelSummaryDto Summary { get; init; }
}

internal static class MapExtractor
{
    private static readonly Regex SuffixPl = new("_PL$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixEv = new("_EV(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixGd = new("_GD(?:\\d+|_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixBg = new("_BG\\d*$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixLt = new("_LT(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixKd = new("_KD(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixSd = new("_SD(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixFg = new("_FG(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixNav = new("_NAV(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixMock = new("_MOCK(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);
    private static readonly Regex SuffixCol = new("_COL(?:_\\d+)?$", RegexOptions.CultureInvariant | RegexOptions.IgnoreCase);

    private static readonly HashSet<string> SkippedEngineGeometryTypes = new(StringComparer.Ordinal)
    {
        "Model",
        "BodySetup",
        "RecastNavMeshDataChunk",
        "Polys",
    };

    private static readonly HashSet<string> SkippedEngineAssetRefProps = new(StringComparer.Ordinal)
    {
        "RootComponent",
        "AttachParent",
        "AttachChildren",
        "BlueprintCreatedComponents",
        "InstanceComponents",
        "OwnedComponents",
        "StreamingTextureData",
        "LODData",
        "PerInstanceSMData",
        "PerInstanceSMCustomData",
        "BodyInstance",
        "AggGeom",
        "OverrideParameters",
        "ParameterData",
        "OnComponentBeginOverlap",
        "OnComponentEndOverlap",
        "OnSystemFinished",
    };

    private static readonly JsonSerializer SafePropertySerializer = CreatePropertySerializer();

    private static JsonSerializer CreatePropertySerializer()
    {
        return JsonSerializer.Create(new JsonSerializerSettings
        {
            ReferenceLoopHandling = ReferenceLoopHandling.Ignore,
            NullValueHandling = NullValueHandling.Include,
        });
    }

    public static string ClassifySublevelType(string levelName, string? filePath = null)
    {
        if (SuffixPl.IsMatch(levelName)) return "PL";
        if (SuffixEv.IsMatch(levelName)) return "EV";
        if (SuffixGd.IsMatch(levelName)) return "GD";
        if (SuffixBg.IsMatch(levelName)) return "BG";
        if (SuffixLt.IsMatch(levelName)) return "LT";
        if (SuffixKd.IsMatch(levelName)) return "KD";
        if (SuffixSd.IsMatch(levelName)) return "SD";
        if (SuffixFg.IsMatch(levelName)) return "FG";
        if (SuffixNav.IsMatch(levelName)) return "NAV";
        if (SuffixMock.IsMatch(levelName)) return "MOCK";
        if (SuffixCol.IsMatch(levelName)) return "COL";

        if (!string.IsNullOrEmpty(filePath))
        {
            var segments = filePath.Replace('\\', '/').Split('/');
            if (segments.Length >= 2)
            {
                var parent = segments[^2].ToUpperInvariant();
                if (parent is "EV" or "GD" or "BG" or "LT" or "KD" or "SD" or "FG" or "NAV" or "MOCK" or "COL")
                {
                    return parent;
                }
            }
        }

        return "OTHER";
    }

    public static bool IsGameClass(string classFullName)
    {
        return classFullName.StartsWith("/Script/mucha.", StringComparison.Ordinal)
            || classFullName.StartsWith("/Game/", StringComparison.Ordinal);
    }

    public static string ToGamePackagePath(string pakPath)
    {
        var normalized = pakPath.Replace('\\', '/');
        var extIndex = normalized.LastIndexOf('.');
        if (extIndex > normalized.LastIndexOf('/'))
        {
            normalized = normalized[..extIndex];
        }

        if (normalized.StartsWith("FFRS/Content/", StringComparison.OrdinalIgnoreCase))
        {
            return "/Game/" + normalized["FFRS/Content/".Length..];
        }

        return "/" + normalized.TrimStart('/');
    }

    public static string ExtractLevelStem(string rawPath)
    {
        var trimmed = rawPath.Trim();
        var slash = trimmed.LastIndexOf('/');
        var segment = slash >= 0 ? trimmed[(slash + 1)..] : trimmed;
        var colon = segment.IndexOf(':');
        if (colon >= 0)
        {
            segment = segment[..colon];
        }

        var dot = segment.IndexOf('.');
        if (dot >= 0)
        {
            segment = segment[..dot];
        }

        return segment;
    }

    public static List<StreamingLevelDto> ExtractStreamingLevels(UObject[] exports)
    {
        var result = new List<StreamingLevelDto>();
        foreach (var export in exports)
        {
            if (export is not ULevelStreaming && !export.ExportType.StartsWith("LevelStreaming", StringComparison.Ordinal))
            {
                continue;
            }

            string? worldAsset = null;
            if (export is ULevelStreaming streaming && streaming.WorldAsset is { } wa && !string.IsNullOrEmpty(wa.AssetPathName.Text))
            {
                worldAsset = wa.AssetPathName.Text;
            }
            else if (export.TryGetValue(out FSoftObjectPath softPath, "WorldAsset") && !string.IsNullOrEmpty(softPath.AssetPathName.Text))
            {
                worldAsset = softPath.AssetPathName.Text;
            }

            if (string.IsNullOrWhiteSpace(worldAsset))
            {
                continue;
            }

            var stem = ExtractLevelStem(worldAsset);
            if (string.IsNullOrEmpty(stem))
            {
                continue;
            }

            result.Add(new StreamingLevelDto(stem, worldAsset, export.ExportType));
        }

        result.Sort((a, b) => string.CompareOrdinal(a.Name, b.Name));
        return result;
    }

    public static List<ActorDto> ExtractActors(IPackage package, UObject[] exports, string levelPackagePath)
    {
        var level = exports.OfType<ULevel>().FirstOrDefault();
        if (level is null)
        {
            return [];
        }

        var actorObjects = new List<UObject>();
        var actorSet = new HashSet<UObject>(ReferenceEqualityComparer.Instance);
        foreach (var actorRef in level.Actors)
        {
            if (actorRef is null || actorRef.IsNull)
            {
                continue;
            }

            var actor = actorRef.Load();
            if (actor is not null && actorSet.Add(actor))
            {
                actorObjects.Add(actor);
            }
        }

        // Map every export to its owning actor (walking Outer chain).
        var subobjectsByActor = new Dictionary<UObject, List<UObject>>(ReferenceEqualityComparer.Instance);
        foreach (var export in exports)
        {
            if (actorSet.Contains(export) || export is UWorld or ULevel or ULevelStreaming)
            {
                continue;
            }

            var ownerActor = FindOwningActor(export, actorSet);
            if (ownerActor is not null)
            {
                if (!subobjectsByActor.TryGetValue(ownerActor, out var list))
                {
                    list = [];
                    subobjectsByActor[ownerActor] = list;
                }

                list.Add(export);
            }
        }

        var actors = new List<ActorDto>(actorObjects.Count);
        foreach (var actor in actorObjects)
        {
            var classPath = GetClassFullName(actor);
            var isGame = IsGameClass(classPath);
            subobjectsByActor.TryGetValue(actor, out var ownedExports);
            ownedExports ??= [];

            var rootComponent = ResolveRootComponent(actor, ownedExports);
            var (location, rotation, scale) = ComputeWorldTransform(rootComponent, new HashSet<UObject>(ReferenceEqualityComparer.Instance));
            var label = TryGetActorLabel(actor);

            if (isGame)
            {
                var props = SerializeProperties(actor.Properties);
                var subobjects = new List<SubobjectDto>();
                foreach (var sub in ownedExports)
                {
                    if (SkippedEngineGeometryTypes.Contains(sub.ExportType))
                    {
                        continue;
                    }

                    var subClass = GetClassFullName(sub);
                    var subIsGame = IsGameClass(subClass);
                    var (subLoc, subRot, subScale) = ReadLocalTransform(sub);
                    var (boxExtent, sphereRadius, capHalfHeight, capRadius) = ReadShapeDimensions(sub);

                    if (subIsGame)
                    {
                        var subProps = SerializeProperties(sub.Properties);
                        subobjects.Add(new SubobjectDto
                        {
                            Name = sub.Name,
                            Class = subClass,
                            IsGameClass = true,
                            Location = subLoc,
                            Rotation = subRot,
                            Scale = subScale,
                            Properties = subProps.Count > 0 ? subProps : null,
                        });
                    }
                    else
                    {
                        var refs = CollectAssetRefs(sub, levelPackagePath);
                        if (subLoc is not null || subRot is not null || subScale is not null
                            || boxExtent is not null || sphereRadius is not null
                            || capHalfHeight is not null || capRadius is not null
                            || refs.Count > 0)
                        {
                            subobjects.Add(new SubobjectDto
                            {
                                Name = sub.Name,
                                Class = subClass,
                                IsGameClass = false,
                                Location = subLoc,
                                Rotation = subRot,
                                Scale = subScale,
                                BoxExtent = boxExtent,
                                SphereRadius = sphereRadius,
                                CapsuleHalfHeight = capHalfHeight,
                                CapsuleRadius = capRadius,
                                AssetRefs = refs.Count > 0 ? refs : null,
                            });
                        }
                    }
                }

                actors.Add(new ActorDto
                {
                    Name = actor.Name,
                    Label = label,
                    Class = classPath,
                    IsGameClass = true,
                    Location = location,
                    Rotation = rotation,
                    Scale = scale,
                    Properties = props,
                    Subobjects = subobjects.Count > 0 ? subobjects : null,
                });
            }
            else
            {
                var assetRefs = new SortedSet<string>(StringComparer.Ordinal);
                CollectAssetRefsInto(actor, levelPackagePath, assetRefs);
                foreach (var sub in ownedExports)
                {
                    if (SkippedEngineGeometryTypes.Contains(sub.ExportType))
                    {
                        continue;
                    }

                    CollectAssetRefsInto(sub, levelPackagePath, assetRefs);
                }

                actors.Add(new ActorDto
                {
                    Name = actor.Name,
                    Label = label,
                    Class = classPath,
                    IsGameClass = false,
                    Location = location,
                    Rotation = rotation,
                    Scale = scale,
                    AssetRefs = assetRefs.Count > 0 ? [.. assetRefs] : null,
                });
            }
        }

        return actors;
    }

    public static LevelSummaryDto BuildSummary(IReadOnlyList<ActorDto> actors, int? levelMapId)
    {
        var mapUnits = new List<MapUnitSummaryDto>();
        var treasureBoxes = new List<TreasureBoxSummaryDto>();
        var eventTriggers = new List<EventTriggerSummaryDto>();
        var transitions = new List<TransitionSummaryDto>();
        var transitionPoints = new List<TransitionPointSummaryDto>();
        var savePoints = new List<SavePointSummaryDto>();
        var shops = new List<ShopSummaryDto>();
        var inns = new List<InnSummaryDto>();
        var gimmicks = new List<GimmickSummaryDto>();
        var encounterZones = new List<EncounterZoneSummaryDto>();

        foreach (var actor in actors)
        {
            if (!actor.IsGameClass)
            {
                continue;
            }

            var props = actor.Properties;
            var shortClass = GetShortClassName(actor.Class);
            var uniqueId = GetIntProp(props, "m_UniqueId") ?? GetNestedIntProp(props, "TypeData", "ID");
            var interactType = GetStringProp(props, "m_MapInteractType") ?? GetStringProp(props, "interactType");
            var visibleCondition = GetStringProp(props, "m_VisiblFlagCondition") ?? GetStringProp(props, "VisibleCondition");
            var flagCondition = GetStringProp(props, "m_FlagCondition");
            var boxExtent = FindActorBoxExtent(actor);
            var sphereRadius = FindActorSphereRadius(actor);

            // 1. Check shops on actor or any game subobject (ShopEntry.ShopID)
            var shopEntry = FindShopEntry(actor);
            if (shopEntry is { ShopId: >= 0 })
            {
                shops.Add(new ShopSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    MapUnitId = GetIntProp(props, "m_MapUnitId"),
                    ShopId = shopEntry.Value.ShopId,
                    IsVariousShop = shopEntry.Value.ShopId >= 10000,
                    IsTest = shopEntry.Value.IsTest,
                    InteractType = interactType,
                    VisibleCondition = visibleCondition,
                    Location = actor.Location,
                });
            }

            // 2. Check inns (InnEntry.InnID or interactType == Inn)
            var innId = FindInnId(actor, interactType);
            if (innId is not null)
            {
                inns.Add(new InnSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    MapUnitId = GetIntProp(props, "m_MapUnitId"),
                    InnId = innId.Value,
                    InteractType = interactType,
                    VisibleCondition = visibleCondition,
                    Location = actor.Location,
                });
            }

            // 3. NPCs & MapUnits
            if (IsMapUnit(shortClass, props))
            {
                var unitEvents = ExtractActorEvents(props);
                mapUnits.Add(new MapUnitSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    Kind = ClassifyMapUnitKind(shortClass, props),
                    MapUnitId = GetIntProp(props, "m_MapUnitId"),
                    UniqueId = uniqueId,
                    Direction = GetStringProp(props, "m_Direction"),
                    InteractType = interactType,
                    VisibleCondition = visibleCondition,
                    ShopId = shopEntry is { ShopId: >= 0 } ? shopEntry.Value.ShopId : null,
                    InnId = innId,
                    Events = unitEvents,
                    Location = actor.Location,
                });
                continue;
            }

            // 4. Treasure boxes
            if (IsTreasureBox(shortClass, props, interactType))
            {
                var tbData = props?["m_TreasureBoxData"] as JObject;
                var chestId = GetIntProp(tbData, "ID") ?? GetIntProp(props, "m_Id");
                var tbMapId = GetIntProp(tbData, "mapId") ?? levelMapId;
                var itemId = GetIntProp(tbData, "ItemId");
                var itemNum = GetIntProp(tbData, "ItemNum");
                var money = GetIntProp(tbData, "Money");
                var isTrap = GetBoolProp(tbData, "IsTrap");
                var battleGroupId = GetIntProp(tbData, "battleGroupId");
                var finishEvent = GetStringProp(tbData, "battleFinishEventId");
                if (finishEvent == "None")
                {
                    finishEvent = null;
                }

                treasureBoxes.Add(new TreasureBoxSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    ChestId = chestId,
                    MapId = tbMapId,
                    ItemId = itemId >= 0 ? itemId : null,
                    ItemNum = itemNum,
                    Money = money >= 0 ? money : null,
                    IsTrap = isTrap,
                    BattleGroupId = battleGroupId >= 0 ? battleGroupId : null,
                    BattleFinishEventId = finishEvent,
                    VisibleCondition = visibleCondition,
                    Location = actor.Location,
                });
                continue;
            }

            // 5. Save points
            if (IsSavePoint(shortClass, interactType))
            {
                var isTeleport = GetBoolProp(props, "m_IsTeleport") ?? false;
                var trMapId = GetIntProp(props, "m_TranditionMapId");
                var trPointId = GetIntProp(props, "m_TranditionPoint");
                var parentData = props?["m_ParentSavePointData"] as JObject;
                var parentMapId = GetIntProp(parentData, "mapId");
                var parentUniqueId = GetIntProp(parentData, "UniqueID");

                int? resolvedTargetMapId = null;
                int? resolvedTargetPointId = null;
                if (isTeleport || trMapId is >= 0 || trPointId is not null)
                {
                    resolvedTargetMapId = trMapId is >= 0
                        ? trMapId
                        : (parentMapId is >= 0 ? parentMapId : levelMapId);
                    resolvedTargetPointId = trPointId ?? 0;
                }

                savePoints.Add(new SavePointSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    SavePointType = GetStringProp(props, "m_Type"),
                    IsTeleport = isTeleport,
                    TransitionMapId = trMapId,
                    TransitionPointId = trPointId,
                    ParentMapId = parentMapId,
                    ParentUniqueId = parentUniqueId,
                    TargetMapId = resolvedTargetMapId,
                    TargetPointId = resolvedTargetPointId,
                    VisibleCondition = visibleCondition,
                    Location = actor.Location,
                });
                continue;
            }

            // 6. Map transition triggers
            if (IsTransitionTrigger(shortClass))
            {
                var isSameMap = GetBoolProp(props, "m_IsSameMapTrasition") ?? false;
                var isAuto = GetBoolProp(props, "m_IsAutoTransition") ?? false;
                var isEventOnly = GetBoolProp(props, "m_IsEventOnly") ?? false;
                var condition = GetStringProp(props, "m_Condition");
                var targets = ExtractTransitionTargets(props, levelMapId, isSameMap, isEventOnly, condition);

                transitions.Add(new TransitionSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    IsSameMap = isSameMap,
                    IsAutoTransition = isAuto,
                    IsEventOnly = isEventOnly,
                    Condition = condition,
                    Targets = targets,
                    BoxExtent = boxExtent,
                    Location = actor.Location,
                });
                continue;
            }

            // 7. Map transition points
            if (IsTransitionPoint(shortClass))
            {
                transitionPoints.Add(new TransitionPointSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    PointId = GetIntProp(props, "m_PointID") ?? 0,
                    Direction = GetStringProp(props, "m_Direction"),
                    PointName = ExtractFTextString(props?["m_PointName"]),
                    Location = actor.Location,
                });
                continue;
            }

            // 8. Enemy encounter zones
            if (IsEncounterActor(shortClass))
            {
                var encountDt = ExtractObjectRefPath(props?["m_pEncountDatatable"]);
                var terrainList = new List<EncounterTerrainDto>();
                if (props?["m_EncountDatatableList"] is JArray terrainArr)
                {
                    foreach (var item in terrainArr.OfType<JObject>())
                    {
                        terrainList.Add(new EncounterTerrainDto
                        {
                            Terrain = GetStringProp(item, "terrain"),
                            Datatable = ExtractObjectRefPath(item["pEncountDatatable"]),
                        });
                    }
                }

                encounterZones.Add(new EncounterZoneSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    Kind = ClassifyEncounterKind(shortClass),
                    Id = GetIntProp(props, "m_Id"),
                    Priority = GetIntProp(props, "m_Priority"),
                    TerrainId = GetIntProp(props, "m_TerrainId"),
                    BattleStageId = GetIntProp(props, "m_BattleStageId"),
                    OverwriteRatio = GetDoubleProp(props, "m_OverwriteEncountRatio"),
                    IsResetDistance = GetBoolProp(props, "m_IsResetEncountDistance"),
                    VisibleCondition = visibleCondition,
                    FlagCondition = flagCondition,
                    EncountDatatable = encountDt,
                    TerrainDatatables = terrainList,
                    BoxExtent = boxExtent,
                    SphereRadius = sphereRadius,
                    Location = actor.Location,
                });
                continue;
            }

            // 9. Event triggers (EventTrigger, InteractEventTrigger, ChatTrigger, TalkPoint, or m_InteractEventId)
            var interactEventId = ExtractInteractEventId(props);
            if (IsEventTrigger(shortClass, interactEventId))
            {
                var triggerEvents = ExtractActorEvents(props);
                eventTriggers.Add(new EventTriggerSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    UniqueId = uniqueId,
                    InteractType = interactType,
                    VisibleCondition = visibleCondition,
                    FlagCondition = flagCondition,
                    InteractEventId = interactEventId,
                    Events = triggerEvents,
                    BoxExtent = boxExtent,
                    Location = actor.Location,
                });
                continue;
            }

            // 10. Other gimmicks (CPP_MapGimmick* / BP_MapGimmick*)
            if (IsGimmick(shortClass))
            {
                gimmicks.Add(new GimmickSummaryDto
                {
                    ActorName = actor.Name,
                    Label = actor.Label,
                    Class = actor.Class,
                    Category = ClassifyGimmickCategory(shortClass),
                    UniqueId = uniqueId,
                    InteractType = interactType,
                    VisibleCondition = visibleCondition,
                    FlagCondition = flagCondition,
                    Events = ExtractActorEvents(props),
                    Location = actor.Location,
                });
            }
        }

        return new LevelSummaryDto
        {
            MapUnits = mapUnits,
            TreasureBoxes = treasureBoxes,
            EventTriggers = eventTriggers,
            Transitions = transitions,
            TransitionPoints = transitionPoints,
            SavePoints = savePoints,
            Shops = shops,
            Inns = inns,
            Gimmicks = gimmicks,
            EncounterZones = encounterZones,
        };
    }

    private static UObject? FindOwningActor(UObject export, HashSet<UObject> actorSet)
    {
        var current = export.Outer;
        var guard = 0;
        while (current is not null && guard++ < 16)
        {
            var loaded = current.Load();
            if (loaded is null)
            {
                return null;
            }

            if (actorSet.Contains(loaded))
            {
                return loaded;
            }

            current = loaded.Outer;
        }

        return null;
    }

    private static string GetClassFullName(UObject obj)
    {
        var path = obj.Class?.GetPathName();
        if (!string.IsNullOrEmpty(path))
        {
            return path;
        }

        return obj.ExportType;
    }

    private static string GetShortClassName(string fullClass)
    {
        var dot = fullClass.LastIndexOf('.');
        return dot >= 0 ? fullClass[(dot + 1)..] : fullClass;
    }

    private static string? TryGetActorLabel(UObject actor)
    {
        if (actor.TryGetValue(out string? label, "ActorLabel") && !string.IsNullOrEmpty(label))
        {
            return label;
        }

        // In UE5 cooked maps, ActorLabel may live on AActor fields serialized outside Properties.
        var prop = actor.GetType().GetProperty("ActorLabel");
        if (prop?.GetValue(actor) is string propLabel && !string.IsNullOrEmpty(propLabel))
        {
            return propLabel;
        }

        var field = actor.GetType().GetField("ActorLabel");
        if (field?.GetValue(actor) is string fieldLabel && !string.IsNullOrEmpty(fieldLabel))
        {
            return fieldLabel;
        }

        return null;
    }

    private static UObject? ResolveRootComponent(UObject actor, List<UObject> ownedExports)
    {
        if (actor.TryGetValue(out FPackageIndex? rootIdx, "RootComponent") && rootIdx is not null && !rootIdx.IsNull)
        {
            var loaded = rootIdx.Load();
            if (loaded is not null)
            {
                return loaded;
            }
        }

        foreach (var candidateName in new[]
        {
            "m_pRootBoxComponent",
            "m_pRootComponent",
            "m_RootComponent",
            "m_pBoxComponent",
            "m_pRootShapeComponent",
            "CapsuleComponent",
            "StaticMeshComponent",
            "BrushComponent",
        })
        {
            if (actor.TryGetValue(out FPackageIndex? idx, candidateName) && idx is not null && !idx.IsNull)
            {
                var loaded = idx.Load();
                if (loaded is not null)
                {
                    return loaded;
                }
            }
        }

        // Fallback: first owned component that defines a RelativeLocation without an AttachParent.
        UObject? fallbackWithLocation = null;
        foreach (var sub in ownedExports)
        {
            if (HasProperty(sub, "RelativeLocation"))
            {
                if (!HasProperty(sub, "AttachParent"))
                {
                    return sub;
                }

                fallbackWithLocation ??= sub;
            }
        }

        return fallbackWithLocation;
    }

    private static (Vec3Dto? Location, Rot3Dto? Rotation, Vec3Dto? Scale) ReadLocalTransform(UObject component)
    {
        Vec3Dto? loc = null;
        Rot3Dto? rot = null;
        Vec3Dto? scale = null;

        if (component.TryGetValue(out FVector vLoc, "RelativeLocation"))
        {
            loc = new Vec3Dto(RoundCoord(vLoc.X), RoundCoord(vLoc.Y), RoundCoord(vLoc.Z));
        }

        if (component.TryGetValue(out FRotator vRot, "RelativeRotation"))
        {
            rot = new Rot3Dto(RoundCoord(vRot.Pitch), RoundCoord(vRot.Yaw), RoundCoord(vRot.Roll));
        }

        if (component.TryGetValue(out FVector vScale, "RelativeScale3D"))
        {
            scale = new Vec3Dto(RoundCoord(vScale.X), RoundCoord(vScale.Y), RoundCoord(vScale.Z));
        }

        return (loc, rot, scale);
    }

    private static (Vec3Dto? Location, Rot3Dto? Rotation, Vec3Dto? Scale) ComputeWorldTransform(
        UObject? component,
        HashSet<UObject> visited)
    {
        if (component is null || !visited.Add(component))
        {
            return (null, null, null);
        }

        var (localLoc, localRot, localScale) = ReadLocalTransform(component);

        if (!component.TryGetValue(out FPackageIndex? attachParentIdx, "AttachParent")
            || attachParentIdx is null
            || attachParentIdx.IsNull)
        {
            // Component exists as root: default missing components to UE identity values.
            return (
                localLoc ?? new Vec3Dto(0, 0, 0),
                localRot ?? new Rot3Dto(0, 0, 0),
                localScale ?? new Vec3Dto(1, 1, 1));
        }

        var parent = attachParentIdx.Load();
        if (parent is null)
        {
            return (
                localLoc ?? new Vec3Dto(0, 0, 0),
                localRot ?? new Rot3Dto(0, 0, 0),
                localScale ?? new Vec3Dto(1, 1, 1));
        }

        var (parentLoc, parentRot, parentScale) = ComputeWorldTransform(parent, visited);
        if (parentLoc is null)
        {
            return (
                localLoc ?? new Vec3Dto(0, 0, 0),
                localRot ?? new Rot3Dto(0, 0, 0),
                localScale ?? new Vec3Dto(1, 1, 1));
        }

        var pScale = parentScale ?? new Vec3Dto(1, 1, 1);
        var pRot = parentRot ?? new Rot3Dto(0, 0, 0);
        var cLoc = localLoc ?? new Vec3Dto(0, 0, 0);
        var cRot = localRot ?? new Rot3Dto(0, 0, 0);
        var cScale = localScale ?? new Vec3Dto(1, 1, 1);

        var scaled = new FVector(
            (float)(cLoc.X * pScale.X),
            (float)(cLoc.Y * pScale.Y),
            (float)(cLoc.Z * pScale.Z));
        var parentQuat = new FRotator((float)pRot.Pitch, (float)pRot.Yaw, (float)pRot.Roll).Quaternion();
        var rotated = parentQuat.RotateVector(scaled);

        var worldLoc = new Vec3Dto(
            RoundCoord(parentLoc.X + rotated.X),
            RoundCoord(parentLoc.Y + rotated.Y),
            RoundCoord(parentLoc.Z + rotated.Z));
        var combinedQuat = parentQuat * new FRotator((float)cRot.Pitch, (float)cRot.Yaw, (float)cRot.Roll).Quaternion();
        var combinedRot = combinedQuat.Rotator();
        var worldRot = new Rot3Dto(
            RoundCoord(combinedRot.Pitch),
            RoundCoord(combinedRot.Yaw),
            RoundCoord(combinedRot.Roll));
        var worldScale = new Vec3Dto(
            RoundCoord(pScale.X * cScale.X),
            RoundCoord(pScale.Y * cScale.Y),
            RoundCoord(pScale.Z * cScale.Z));

        return (worldLoc, worldRot, worldScale);
    }

    private static (Vec3Dto? BoxExtent, double? SphereRadius, double? CapsuleHalfHeight, double? CapsuleRadius)
        ReadShapeDimensions(UObject component)
    {
        Vec3Dto? boxExtent = null;
        double? sphereRadius = null;
        double? capHalfHeight = null;
        double? capRadius = null;

        if (component.TryGetValue(out FVector extent, "BoxExtent"))
        {
            boxExtent = new Vec3Dto(RoundCoord(extent.X), RoundCoord(extent.Y), RoundCoord(extent.Z));
        }

        if (component.TryGetValue(out float sr, "SphereRadius"))
        {
            sphereRadius = RoundCoord(sr);
        }

        if (component.TryGetValue(out float chh, "CapsuleHalfHeight"))
        {
            capHalfHeight = RoundCoord(chh);
        }

        if (component.TryGetValue(out float cr, "CapsuleRadius"))
        {
            capRadius = RoundCoord(cr);
        }

        return (boxExtent, sphereRadius, capHalfHeight, capRadius);
    }

    private static bool HasProperty(UObject obj, string propertyName)
    {
        foreach (var tag in obj.Properties)
        {
            if (string.Equals(tag.Name.Text, propertyName, StringComparison.Ordinal))
            {
                return true;
            }
        }

        return false;
    }

    private static JObject SerializeProperties(List<FPropertyTag> properties)
    {
        var obj = new JObject();
        foreach (var tag in properties)
        {
            var name = tag.Name.Text;
            if (string.IsNullOrEmpty(name))
            {
                continue;
            }

            if (tag.ArrayIndex > 0)
            {
                name = $"{name}[{tag.ArrayIndex}]";
            }

            var value = tag.Tag?.GenericValue;
            obj[name] = value is null
                ? JValue.CreateNull()
                : JToken.FromObject(value, SafePropertySerializer);
        }

        return obj;
    }

    private static List<string> CollectAssetRefs(UObject obj, string levelPackagePath)
    {
        var refs = new SortedSet<string>(StringComparer.Ordinal);
        CollectAssetRefsInto(obj, levelPackagePath, refs);
        return [.. refs];
    }

    private static void CollectAssetRefsInto(UObject obj, string levelPackagePath, SortedSet<string> refs)
    {
        foreach (var tag in obj.Properties)
        {
            if (SkippedEngineAssetRefProps.Contains(tag.Name.Text))
            {
                continue;
            }

            CollectAssetRefsFromTag(tag.Tag, levelPackagePath, refs, 0);
        }
    }

    private static void CollectAssetRefsFromTag(FPropertyTagType? tag, string levelPackagePath, SortedSet<string> refs, int depth)
    {
        if (tag is null || depth > 4)
        {
            return;
        }

        switch (tag)
        {
            case ObjectProperty objProp:
                AddPackageIndexRef(objProp.Value, levelPackagePath, refs);
                break;
            case SoftObjectProperty softProp:
                AddSoftAssetRef(softProp.Value.AssetPathName.Text, levelPackagePath, refs);
                break;
            case ArrayProperty arrProp when arrProp.Value is not null:
                foreach (var item in arrProp.Value.Properties)
                {
                    CollectAssetRefsFromTag(item, levelPackagePath, refs, depth + 1);
                }
                break;
            case SetProperty setProp when setProp.Value is not null:
                foreach (var item in setProp.Value.Properties)
                {
                    CollectAssetRefsFromTag(item, levelPackagePath, refs, depth + 1);
                }
                break;
            case MapProperty mapProp when mapProp.Value is not null:
                foreach (var kv in mapProp.Value.Properties)
                {
                    CollectAssetRefsFromTag(kv.Key, levelPackagePath, refs, depth + 1);
                    CollectAssetRefsFromTag(kv.Value, levelPackagePath, refs, depth + 1);
                }
                break;
            case StructProperty structProp when structProp.Value?.StructType is FStructFallback fallback:
                foreach (var nested in fallback.Properties)
                {
                    if (!SkippedEngineAssetRefProps.Contains(nested.Name.Text))
                    {
                        CollectAssetRefsFromTag(nested.Tag, levelPackagePath, refs, depth + 1);
                    }
                }
                break;
        }
    }

    private static void AddPackageIndexRef(FPackageIndex? index, string levelPackagePath, SortedSet<string> refs)
    {
        if (index is null || index.IsNull)
        {
            return;
        }

        var resolved = index.ResolvedObject;
        if (resolved is null)
        {
            return;
        }

        var path = resolved.GetPathName();
        AddAssetRefPath(path, levelPackagePath, refs);
    }

    private static void AddSoftAssetRef(string? path, string levelPackagePath, SortedSet<string> refs)
    {
        if (!string.IsNullOrEmpty(path))
        {
            AddAssetRefPath(path, levelPackagePath, refs);
        }
    }

    private static void AddAssetRefPath(string path, string levelPackagePath, SortedSet<string> refs)
    {
        if (string.IsNullOrWhiteSpace(path) || path == "None")
        {
            return;
        }

        // Ignore references to objects inside the same level package or /Script/ classes.
        if (path.StartsWith("/Script/", StringComparison.Ordinal))
        {
            return;
        }

        if (path.StartsWith(levelPackagePath + ".", StringComparison.Ordinal)
            || path.StartsWith(levelPackagePath + ":", StringComparison.Ordinal)
            || string.Equals(path, levelPackagePath, StringComparison.Ordinal))
        {
            return;
        }

        if (path.StartsWith("/Game/", StringComparison.Ordinal) || path.StartsWith("/Engine/", StringComparison.Ordinal))
        {
            refs.Add(path);
        }
    }

    private static double RoundCoord(double value)
    {
        var rounded = Math.Round(value, 3);
        return rounded == 0 ? 0 : rounded;
    }

    private static bool IsMapUnit(string shortClass, JObject? props)
    {
        return shortClass.Contains("MapUnit", StringComparison.OrdinalIgnoreCase)
            || props?["m_MapUnitId"] is not null;
    }

    private static string ClassifyMapUnitKind(string shortClass, JObject? props)
    {
        if (shortClass.Contains("NPC", StringComparison.OrdinalIgnoreCase)
            || string.Equals(GetStringProp(props, "m_MapObjectType"), "eMapObjectType::NPC", StringComparison.Ordinal))
        {
            return "npc";
        }

        if (shortClass.Contains("Player", StringComparison.OrdinalIgnoreCase))
        {
            return "player";
        }

        if (shortClass.Contains("AirShip", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("Chocobo", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("Ship", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("Vehicle", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("MagicAromor", StringComparison.OrdinalIgnoreCase))
        {
            return "vehicle";
        }

        if (shortClass.Contains("Object", StringComparison.OrdinalIgnoreCase))
        {
            return "object";
        }

        return "unit";
    }

    private static bool IsTreasureBox(string shortClass, JObject? props, string? interactType)
    {
        return shortClass.Contains("TreasureBox", StringComparison.OrdinalIgnoreCase)
            || props?["m_TreasureBoxData"] is not null
            || string.Equals(interactType, "eMapInteractType::Treasure", StringComparison.Ordinal);
    }

    private static bool IsSavePoint(string shortClass, string? interactType)
    {
        return shortClass.Contains("SavePoint", StringComparison.OrdinalIgnoreCase)
            || string.Equals(interactType, "eMapInteractType::SavePoint", StringComparison.Ordinal);
    }

    private static bool IsTransitionTrigger(string shortClass)
    {
        return shortClass.Contains("MapTransitionTrigger", StringComparison.OrdinalIgnoreCase);
    }

    private static bool IsTransitionPoint(string shortClass)
    {
        return shortClass.Contains("MapTransitionPoint", StringComparison.OrdinalIgnoreCase);
    }

    private static bool IsEncounterActor(string shortClass)
    {
        return shortClass.Contains("MapEncount", StringComparison.OrdinalIgnoreCase);
    }

    private static string ClassifyEncounterKind(string shortClass)
    {
        if (shortClass.Contains("OverrideBattleStage", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("OverrideBtlStgId", StringComparison.OrdinalIgnoreCase))
        {
            return "OverrideBattleStageBox";
        }

        if (shortClass.Contains("Overwirte", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("Overwrite", StringComparison.OrdinalIgnoreCase))
        {
            return "OverwriteBox";
        }

        if (shortClass.Contains("Pile", StringComparison.OrdinalIgnoreCase))
        {
            return "Pile";
        }

        if (shortClass.Contains("Sphere", StringComparison.OrdinalIgnoreCase))
        {
            return "Sphere";
        }

        if (shortClass.Contains("Box", StringComparison.OrdinalIgnoreCase))
        {
            return "Box";
        }

        return shortClass;
    }

    private static bool IsEventTrigger(string shortClass, string? interactEventId)
    {
        return shortClass.Contains("EventTrigger", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("ChatTrigger", StringComparison.OrdinalIgnoreCase)
            || shortClass.Contains("TalkPoint", StringComparison.OrdinalIgnoreCase)
            || !string.IsNullOrEmpty(interactEventId);
    }

    private static bool IsGimmick(string shortClass)
    {
        return shortClass.Contains("MapGimmick", StringComparison.OrdinalIgnoreCase);
    }

    private static string ClassifyGimmickCategory(string shortClass)
    {
        if (shortClass.Contains("FieldItem", StringComparison.OrdinalIgnoreCase)) return "FieldItem";
        if (shortClass.Contains("Blocking", StringComparison.OrdinalIgnoreCase) || shortClass.Contains("BlockObject", StringComparison.OrdinalIgnoreCase)) return "Blocking";
        if (shortClass.Contains("NoEntry", StringComparison.OrdinalIgnoreCase)) return "NoEntry";
        if (shortClass.Contains("Door", StringComparison.OrdinalIgnoreCase)) return "Door";
        if (shortClass.Contains("Ladder", StringComparison.OrdinalIgnoreCase)) return "Ladder";
        if (shortClass.Contains("Elevator", StringComparison.OrdinalIgnoreCase)) return "Elevator";
        if (shortClass.Contains("Switch", StringComparison.OrdinalIgnoreCase)) return "Switch";
        if (shortClass.Contains("Lamp", StringComparison.OrdinalIgnoreCase)) return "Lamp";
        if (shortClass.Contains("Raft", StringComparison.OrdinalIgnoreCase)) return "Raft";
        if (shortClass.Contains("MerchantShip", StringComparison.OrdinalIgnoreCase)) return "MerchantShip";
        if (shortClass.Contains("AirShip", StringComparison.OrdinalIgnoreCase) || shortClass.Contains("AirShipe", StringComparison.OrdinalIgnoreCase)) return "AirShip";
        if (shortClass.Contains("Warp", StringComparison.OrdinalIgnoreCase)) return "Warp";
        if (shortClass.Contains("Jump", StringComparison.OrdinalIgnoreCase)) return "JumpPoint";
        if (shortClass.Contains("VisionSmallShrine", StringComparison.OrdinalIgnoreCase)) return "VisionSmallShrine";
        if (shortClass.Contains("Floor", StringComparison.OrdinalIgnoreCase)) return "Floor";
        if (shortClass.Contains("Effect", StringComparison.OrdinalIgnoreCase)) return "Effect";
        if (shortClass.Contains("Bird", StringComparison.OrdinalIgnoreCase)) return "Bird";
        if (shortClass.Contains("BreakObject", StringComparison.OrdinalIgnoreCase)) return "BreakObject";
        if (shortClass.Contains("PushObject", StringComparison.OrdinalIgnoreCase)) return "PushObject";
        return shortClass;
    }

    private static (int ShopId, bool IsTest)? FindShopEntry(ActorDto actor)
    {
        var direct = ReadShopEntryFromProps(actor.Properties);
        if (direct is not null)
        {
            return direct;
        }

        if (actor.Subobjects is not null)
        {
            foreach (var sub in actor.Subobjects)
            {
                var nested = ReadShopEntryFromProps(sub.Properties);
                if (nested is not null)
                {
                    return nested;
                }
            }
        }

        return null;
    }

    private static (int ShopId, bool IsTest)? ReadShopEntryFromProps(JObject? props)
    {
        if (props is null)
        {
            return null;
        }

        if (props["ShopEntry"] is JObject entry)
        {
            var shopId = GetIntProp(entry, "ShopID");
            if (shopId is >= 0)
            {
                return (shopId.Value, GetBoolProp(entry, "IsTest") ?? false);
            }
        }

        return null;
    }

    private static int? FindInnId(ActorDto actor, string? interactType)
    {
        if (actor.Properties?["InnEntry"] is JObject entry)
        {
            var id = GetIntProp(entry, "InnID");
            if (id is >= 0)
            {
                return id.Value;
            }
        }

        if (string.Equals(interactType, "eMapInteractType::Inn", StringComparison.Ordinal))
        {
            return 0;
        }

        return null;
    }

    private static List<TransitionTargetDto> ExtractTransitionTargets(
        JObject? props,
        int? levelMapId,
        bool isSameMap,
        bool defaultIsEventOnly,
        string? defaultCondition)
    {
        var targets = new List<TransitionTargetDto>();
        if (props?["mTransitionDataList"] is JArray list && list.Count > 0)
        {
            foreach (var item in list.OfType<JObject>())
            {
                var rawMapId = GetIntProp(item, "mapId");
                var targetMapId = rawMapId is >= 0 ? rawMapId : (isSameMap ? levelMapId : null);
                var targetPointId = GetIntProp(item, "pointId") ?? (targetMapId is not null ? 0 : null);
                var itemEventOnly = GetBoolProp(item, "isEventOnly") ?? false;
                var events = new List<EventEntryDto>();
                ExtractEventsFromArray(item["EventList"] as JArray, events);
                ExtractEventsFromArray(item["eventList2"] as JArray, events);
                var afterEvents = new List<EventEntryDto>();
                ExtractEventsFromArray(item["afterTransitionEventList"] as JArray, afterEvents);

                targets.Add(new TransitionTargetDto
                {
                    Condition = NormalizeCondition(GetStringProp(item, "FlagCondition")),
                    TargetMapId = targetMapId,
                    TargetPointId = targetPointId,
                    IsEventOnly = itemEventOnly,
                    Events = events,
                    AfterTransitionEvents = afterEvents,
                });
            }

            return targets;
        }

        var directMapId = GetIntProp(props, "m_MapId");
        var resolvedMapId = directMapId is >= 0 ? directMapId : (isSameMap ? levelMapId : null);
        var directPointId = GetIntProp(props, "m_PointID");
        var resolvedPointId = directPointId ?? (resolvedMapId is not null ? 0 : null);
        var directEvents = new List<EventEntryDto>();
        ExtractEventsFromArray(props?["m_EventList"] as JArray, directEvents);
        ExtractEventsFromArray(props?["m_EventList2"] as JArray, directEvents);
        var directAfterEvents = new List<EventEntryDto>();
        ExtractEventsFromArray(props?["m_AfterTransitionEventList"] as JArray, directAfterEvents);

        if (resolvedMapId is not null || resolvedPointId is not null || defaultIsEventOnly || directEvents.Count > 0)
        {
            targets.Add(new TransitionTargetDto
            {
                Condition = NormalizeCondition(defaultCondition),
                TargetMapId = resolvedMapId,
                TargetPointId = resolvedPointId,
                IsEventOnly = defaultIsEventOnly,
                Events = directEvents,
                AfterTransitionEvents = directAfterEvents,
            });
        }

        return targets;
    }

    private static List<EventEntryDto> ExtractActorEvents(JObject? props)
    {
        var events = new List<EventEntryDto>();
        if (props is null)
        {
            return events;
        }

        ExtractEventsFromArray(props["m_EventList"] as JArray, events);
        ExtractEventsFromArray(props["m_EventList2"] as JArray, events);

        if (props["m_EventSettings"] is JObject settings && settings["EventList"] is JArray groups)
        {
            foreach (var group in groups.OfType<JObject>())
            {
                var groupCond = NormalizeCondition(GetStringProp(group, "Condition"));
                if (group["EventList"] is JArray innerList)
                {
                    foreach (var entry in innerList.OfType<JObject>())
                    {
                        var eventId = GetStringProp(entry, "EventId");
                        if (!string.IsNullOrEmpty(eventId) && eventId != "None")
                        {
                            events.Add(new EventEntryDto(groupCond, eventId));
                        }
                    }
                }
            }
        }

        return events;
    }

    private static void ExtractEventsFromArray(JArray? array, List<EventEntryDto> target)
    {
        if (array is null)
        {
            return;
        }

        foreach (var item in array.OfType<JObject>())
        {
            var eventId = GetStringProp(item, "EventId");
            if (!string.IsNullOrEmpty(eventId) && eventId != "None")
            {
                var cond = NormalizeCondition(GetStringProp(item, "Condition"));
                target.Add(new EventEntryDto(cond, eventId));
            }
        }
    }

    private static string? ExtractInteractEventId(JObject? props)
    {
        if (props is null)
        {
            return null;
        }

        var token = props["m_InteractEventId"];
        if (token is null)
        {
            return null;
        }

        string? value = token switch
        {
            JValue jv when jv.Type == JTokenType.String => jv.Value<string>(),
            JObject jo => GetStringProp(jo, "EventId"),
            _ => null,
        };
        return string.IsNullOrEmpty(value) || value == "None" ? null : value;
    }

    private static Vec3Dto? FindActorBoxExtent(ActorDto actor)
    {
        if (actor.Properties?["m_BoxComponentBoxExtent"] is JObject extentObj)
        {
            var vec = ReadVec3FromJObject(extentObj);
            if (vec is not null)
            {
                return vec;
            }
        }

        if (actor.Subobjects is not null)
        {
            foreach (var sub in actor.Subobjects)
            {
                if (sub.BoxExtent is not null)
                {
                    return sub.BoxExtent;
                }
            }
        }

        return null;
    }

    private static double? FindActorSphereRadius(ActorDto actor)
    {
        if (actor.Subobjects is not null)
        {
            foreach (var sub in actor.Subobjects)
            {
                if (sub.SphereRadius is not null)
                {
                    return sub.SphereRadius;
                }
            }
        }

        return null;
    }

    private static Vec3Dto? ReadVec3FromJObject(JObject obj)
    {
        var x = GetDoubleProp(obj, "X") ?? GetDoubleProp(obj, "x");
        var y = GetDoubleProp(obj, "Y") ?? GetDoubleProp(obj, "y");
        var z = GetDoubleProp(obj, "Z") ?? GetDoubleProp(obj, "z");
        if (x is not null && y is not null && z is not null)
        {
            return new Vec3Dto(RoundCoord(x.Value), RoundCoord(y.Value), RoundCoord(z.Value));
        }

        return null;
    }

    private static string? ExtractObjectRefPath(JToken? token)
    {
        if (token is not JObject obj)
        {
            return null;
        }

        var path = GetStringProp(obj, "ObjectPath") ?? GetStringProp(obj, "AssetPathName");
        if (string.IsNullOrEmpty(path))
        {
            return null;
        }

        var dot = path.LastIndexOf('.');
        var slash = path.LastIndexOf('/');
        return dot > slash ? path[..dot] : path;
    }

    private static string? ExtractFTextString(JToken? token)
    {
        if (token is null)
        {
            return null;
        }

        if (token is JValue jv && jv.Type == JTokenType.String)
        {
            var str = jv.Value<string>();
            return string.IsNullOrEmpty(str) ? null : str;
        }

        if (token is JObject obj)
        {
            return GetStringProp(obj, "LocalizedString")
                ?? GetStringProp(obj, "SourceString")
                ?? GetStringProp(obj, "CultureInvariantString");
        }

        return null;
    }

    private static string? NormalizeCondition(string? raw)
    {
        return string.IsNullOrWhiteSpace(raw) ? null : raw;
    }

    private static int? GetIntProp(JObject? obj, string key)
    {
        var token = obj?[key];
        if (token is null || token.Type == JTokenType.Null)
        {
            return null;
        }

        if (token.Type == JTokenType.Integer)
        {
            return token.Value<int>();
        }

        if (token.Type == JTokenType.Float)
        {
            return (int)token.Value<double>();
        }

        return null;
    }

    private static int? GetNestedIntProp(JObject? obj, string parentKey, string childKey)
    {
        return obj?[parentKey] is JObject nested ? GetIntProp(nested, childKey) : null;
    }

    private static double? GetDoubleProp(JObject? obj, string key)
    {
        var token = obj?[key];
        if (token is null || token.Type == JTokenType.Null)
        {
            return null;
        }

        if (token.Type is JTokenType.Float or JTokenType.Integer)
        {
            return token.Value<double>();
        }

        return null;
    }

    private static bool? GetBoolProp(JObject? obj, string key)
    {
        var token = obj?[key];
        if (token is null || token.Type != JTokenType.Boolean)
        {
            return null;
        }

        return token.Value<bool>();
    }

    private static string? GetStringProp(JObject? obj, string key)
    {
        var token = obj?[key];
        if (token is null || token.Type != JTokenType.String)
        {
            return null;
        }

        var value = token.Value<string>();
        return string.IsNullOrEmpty(value) ? null : value;
    }
}
