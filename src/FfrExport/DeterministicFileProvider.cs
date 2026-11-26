using CUE4Parse.FileProvider;
using CUE4Parse.FileProvider.Objects;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.Assets.Objects;
using CUE4Parse.UE4.IO;
using CUE4Parse.UE4.IO.Objects;
using CUE4Parse.UE4.Readers;
using CUE4Parse.UE4.Versions;
using CUE4Parse.UE4.VirtualFileSystem;

namespace FfrExport;

/// <summary>
/// Wraps CUE4Parse's <see cref="DefaultFileProvider"/> to guarantee deterministic VFS mount order,
/// eager serial initialization of all IoStore container headers before parallel export, headerless
/// patch container resolution for every <see cref="LoadPackage(GameFile)"/> entry point (including
/// internal <c>FPackageId</c> import lookups), and synchronized package loading with isolated
/// per-package <see cref="VersionContainer"/> state.
/// </summary>
internal sealed class DeterministicFileProvider : DefaultFileProvider
{
    private readonly object _loadSyncRoot = new();
    private readonly Dictionary<IoStoreReader, FIoContainerHeader?> _readerContainerHeaders = new();
    private readonly Dictionary<FPackageId, FIoContainerHeader> _containerHeaderByPackageId = new();
    private readonly List<string> _mountedVfsOrder = [];
    private FIoContainerHeader? _fallbackContainerHeader;
    private bool _fallbackContainerHeaderIsPatch;
    private bool _isMountedDeterministically;
    private int _headerlessIoStoreCount;

    public DeterministicFileProvider(
        string directory,
        SearchOption searchOption,
        VersionContainer? versions = null,
        StringComparer? pathComparer = null)
        : base(directory, searchOption, versions, pathComparer)
    {
    }

    public DeterministicFileProvider(
        DirectoryInfo directory,
        SearchOption searchOption,
        VersionContainer? versions = null,
        StringComparer? pathComparer = null)
        : base(directory, searchOption, versions, pathComparer)
    {
    }

    internal object LoadSyncRoot => _loadSyncRoot;

    internal bool IsMountedDeterministically => _isMountedDeterministically;

    internal IReadOnlyList<string> MountedVfsOrder => _mountedVfsOrder;

    internal int HeaderlessIoStoreCount => _headerlessIoStoreCount;

    internal FIoContainerHeader? FallbackContainerHeader => _fallbackContainerHeader;

    public new int Mount() => MountDeterministically();

    /// <summary>
    /// Mounts all registered VFS readers in a strictly deterministic order (global container first,
    /// then ascending <see cref="IVfsReader.ReadOrder"/> so higher-priority patch containers always
    /// win in both path-based and <see cref="FPackageId"/>-based indices) and eagerly loads every
    /// IoStore <see cref="FIoContainerHeader"/> serially before any parallel export begins.
    /// </summary>
    internal int MountDeterministically()
    {
        lock (_loadSyncRoot)
        {
            var candidates = _unloadedVfs.Keys.ToArray();
            Array.Sort(candidates, CompareVfsReadersForMount);

            _unloadedVfs.Clear();
            var unmounted = new List<IAesVfsReader>();
            var totalMounted = 0;

            foreach (var reader in candidates)
            {
                _unloadedVfs[reader] = null;
                totalMounted += base.Mount();

                if (_unloadedVfs.TryRemove(reader, out _))
                {
                    // Readers without a directory index (e.g. global.utoc, whose IoGlobalData has
                    // already been initialized by base.Mount()) or encrypted archives awaiting keys
                    // remain unloaded; restore them after the loop so Dispose() still closes them.
                    unmounted.Add(reader);
                }
                else
                {
                    _mountedVfsOrder.Add(reader.Name);
                }
            }

            foreach (var reader in unmounted)
            {
                _unloadedVfs[reader] = null;
            }

            PreloadIoStoreContainerHeadersLocked();
            _isMountedDeterministically = true;
            return totalMounted;
        }
    }

    /// <summary>
    /// Verifies that the provider completed deterministic mounting and that every mounted
    /// <see cref="IoStoreReader"/> has its <see cref="FIoContainerHeader"/> preloaded and pinned.
    /// </summary>
    internal void EnsureReadyForDeterministicLoads()
    {
        if (!_isMountedDeterministically)
        {
            throw new InvalidOperationException(
                "DeterministicFileProvider must be mounted via MountDeterministically() before loading packages.");
        }

        foreach (var vfs in MountedVfs)
        {
            if (vfs is IoStoreReader ioReader && !_readerContainerHeaders.ContainsKey(ioReader))
            {
                throw new InvalidOperationException(
                    $"IoStoreReader '{ioReader.Name}' was mounted outside MountDeterministically() without container header preloading.");
            }
        }
    }

    public override IPackage LoadPackage(GameFile file)
    {
        EnsureReadyForDeterministicLoads();

        lock (_loadSyncRoot)
        {
            if (file is FIoStoreEntry ioEntry)
            {
                return LoadIoStorePackageLocked(ioEntry);
            }

            return base.LoadPackage(file);
        }
    }

    public override Task<IPackage> LoadPackageAsync(GameFile file)
    {
        EnsureReadyForDeterministicLoads();
        return Task.FromResult(LoadPackage(file));
    }

    private void PreloadIoStoreContainerHeadersLocked()
    {
        _readerContainerHeaders.Clear();
        _containerHeaderByPackageId.Clear();
        _fallbackContainerHeader = null;
        _fallbackContainerHeaderIsPatch = false;
        _headerlessIoStoreCount = 0;

        var ioReaders = MountedVfs.OfType<IoStoreReader>().ToArray();
        Array.Sort(ioReaders, CompareVfsReadersForMount);

        foreach (var reader in ioReaders)
        {
            var headerChunkType = reader.Game >= EGame.GAME_UE5_0
                ? (byte)EIoChunkType5.ContainerHeader
                : (byte)EIoChunkType.ContainerHeader;
            var headerChunkId = new FIoChunkId(reader.TocResource.Header.ContainerId.Id, 0, headerChunkType);

            if (!reader.DoesChunkExist(headerChunkId))
            {
                _readerContainerHeaders[reader] = null;
                _headerlessIoStoreCount++;
                continue;
            }

            // Force lazy IoStoreReader.ContainerHeader initialization serially while mounting.
            var header = reader.ContainerHeader;
            _readerContainerHeaders[reader] = header;

            if (header is null)
            {
                continue;
            }

            if (header.PackageIds is not null)
            {
                foreach (var packageId in header.PackageIds)
                {
                    _containerHeaderByPackageId[packageId] = header;
                }
            }

            if (header.OptionalSegmentPackageIds is not null)
            {
                foreach (var packageId in header.OptionalSegmentPackageIds)
                {
                    _containerHeaderByPackageId[packageId] = header;
                }
            }

            var isPatch = IsPatchContainerName(reader.Name);
            if (_fallbackContainerHeader is null || (_fallbackContainerHeaderIsPatch && !isPatch))
            {
                _fallbackContainerHeader = header;
                _fallbackContainerHeaderIsPatch = isPatch;
            }
        }
    }

    private IoPackage LoadIoStorePackageLocked(FIoStoreEntry ioEntry)
    {
        if (!ioEntry.IsUePackage)
        {
            throw new ArgumentException("cannot load non-UE package", nameof(ioEntry));
        }

        Files.FindPayloads(ioEntry, out _, out var ubulks, out var uptnls);
        var uasset = ioEntry.CreateReader();
        // Isolate per-package VersionContainer so packages with bHasVersioningInfo cannot mutate
        // shared Vfs/provider version state across threads or packages.
        uasset.Versions = (VersionContainer)uasset.Versions.Clone();

        var lazyUbulk = CreateLazyPayloadReader(ubulks);
        var lazyUptnl = CreateLazyPayloadReader(uptnls);
        var containerHeader = ResolveContainerHeader(ioEntry.IoStoreReader, ioEntry.ChunkId.AsPackageId());
        var optionalSegment = FindOptionalSegmentLocked(ioEntry);

        return new IoPackage(
            uasset,
            containerHeader,
            lazyUbulk,
            lazyUptnl,
            this,
            optionalSegment,
            ioEntry.Vfs.Name);
    }

    private IoOptionalSegment? FindOptionalSegmentLocked(FIoStoreEntry entry)
    {
        if (entry.IsOptionalSegmentPackage || !entry.IsPackageData)
        {
            return null;
        }

        var packageId = entry.ChunkId.AsPackageId();
        if (!Files.TryGetOptionalSegmentPackage(packageId, out var optionalFile)
            || optionalFile is not FIoStoreEntry optionalEntry
            || ReferenceEquals(optionalEntry, entry)
            || !optionalEntry.IsUePackage)
        {
            return null;
        }

        try
        {
            Files.FindPayloads(optionalEntry, out _, out var ubulks, out var uptnls);
            var optionalArchive = optionalEntry.CreateReader();
            optionalArchive.Versions = (VersionContainer)optionalArchive.Versions.Clone();
            var containerHeader = ResolveContainerHeader(optionalEntry.IoStoreReader, optionalEntry.ChunkId.AsPackageId());

            return new IoOptionalSegment(
                optionalArchive,
                CreateLazyPayloadReader(ubulks),
                CreateLazyPayloadReader(uptnls),
                containerHeader);
        }
        catch
        {
            return null;
        }
    }

    private FIoContainerHeader? ResolveContainerHeader(IoStoreReader reader, FPackageId packageId)
    {
        if (_readerContainerHeaders.TryGetValue(reader, out var ownHeader) && ownHeader is not null)
        {
            return ownHeader;
        }

        if (_containerHeaderByPackageId.TryGetValue(packageId, out var packageHeader))
        {
            return packageHeader;
        }

        return _fallbackContainerHeader;
    }

    private static Func<FByteBulkDataHeader?, FArchive?>? CreateLazyPayloadReader(IReadOnlyList<GameFile> payloads)
    {
        if (payloads.Count == 0)
        {
            return null;
        }

        var first = payloads[0];
        return header => first.SafeCreateReader(header);
    }

    internal static int CompareVfsReadersForMount(IAesVfsReader left, IAesVfsReader right)
    {
        var globalCmp = GetGlobalPriority(left).CompareTo(GetGlobalPriority(right));
        if (globalCmp != 0)
        {
            return globalCmp;
        }

        var readOrderCmp = left.ReadOrder.CompareTo(right.ReadOrder);
        if (readOrderCmp != 0)
        {
            return readOrderCmp;
        }

        var patchCmp = (IsPatchContainerName(left.Name) ? 1 : 0).CompareTo(IsPatchContainerName(right.Name) ? 1 : 0);
        if (patchCmp != 0)
        {
            return patchCmp;
        }

        var nameCmp = StringComparer.Ordinal.Compare(left.Name, right.Name);
        if (nameCmp != 0)
        {
            return nameCmp;
        }

        return StringComparer.Ordinal.Compare(left.Path, right.Path);
    }

    private static int GetGlobalPriority(IAesVfsReader reader)
    {
        if (reader is IoStoreReader
            && (string.Equals(reader.Name, "global.utoc", StringComparison.OrdinalIgnoreCase)
                || string.Equals(reader.Name, "global_console_win.utoc", StringComparison.OrdinalIgnoreCase)))
        {
            return 0;
        }

        return 1;
    }

    private static bool IsPatchContainerName(string name)
    {
        return name.Contains("_P.", StringComparison.OrdinalIgnoreCase);
    }
}
