using System.Buffers.Binary;
using System.Text;
using CUE4Parse.FileProvider;
using CUE4Parse.FileProvider.Objects;
using CUE4Parse.FileProvider.Vfs;
using CUE4Parse.MappingsProvider.Usmap;
using CUE4Parse.UE4.Assets;
using CUE4Parse.UE4.IO.Objects;
using CUE4Parse.UE4.Versions;
using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class DeterministicProviderTests
{
    private const ulong SynthSubPublicExportHash = 0x1122334455667788UL;
    private const string MainVfsPath = "FFRS/Content/Sequencer/SynthMain.uasset";
    private const string SubVfsPath = "FFRS/Content/Sequencer/SynthSub.uasset";
    private const string MainPackageName = "/Game/Sequencer/SynthMain";
    private const string SubPackageName = "/Game/Sequencer/SynthSub";

    /// <summary>
    /// Minimal stock DefaultFileProvider subclass that forces base-before-patch mount order
    /// without DeterministicFileProvider's headerless patch override, deterministically triggering
    /// the exact FilesById -> headerless *_P.utoc state observed in build-db.
    /// </summary>
    private sealed class BaseThenPatchRawDefaultFileProvider(
        string directory,
        SearchOption searchOption,
        VersionContainer versions)
        : DefaultFileProvider(directory, searchOption, versions)
    {
        public void MountBaseThenPatchWithoutFix()
        {
            var readers = _unloadedVfs.Keys.ToArray();
            Array.Sort(readers, DeterministicFileProvider.CompareVfsReadersForMount);
            _unloadedVfs.Clear();

            var unmounted = new List<CUE4Parse.UE4.VirtualFileSystem.IAesVfsReader>();
            foreach (var reader in readers)
            {
                _unloadedVfs[reader] = null;
                base.Mount();
                if (_unloadedVfs.TryRemove(reader, out _))
                {
                    unmounted.Add(reader);
                }
            }

            foreach (var reader in unmounted)
            {
                _unloadedVfs[reader] = null;
            }
        }
    }

    [Fact]
    public void StockDefaultFileProvider_ReproducesNullExternalReference_WhenHeaderlessPatchWinsFilesById()
    {
        using var fixture = SyntheticIoStoreFixture.Create();
        using var provider = new BaseThenPatchRawDefaultFileProvider(
            fixture.PaksDirectory,
            SearchOption.TopDirectoryOnly,
            new VersionContainer(EGame.GAME_UE5_6));
        provider.MappingsContainer = new FileUsmapTypeMappingsProvider(fixture.UsmapPath);
        provider.Initialize();
        provider.MountBaseThenPatchWithoutFix();
        Assert.NotNull(provider.GlobalData);

        var subId = FPackageId.FromName(SubPackageName);
        Assert.True(provider.FilesById.TryGetValue(subId, out var subFile));
        var subEntry = Assert.IsType<FIoStoreEntry>(subFile);
        Assert.Equal("Synth-Windows_0_P.utoc", subEntry.IoStoreReader.Name);

        // Main package lives in base container and holds an external FPackageIndex import to SynthSub.
        // Because FilesById[subId] points to headerless Synth-Windows_0_P.utoc, stock DefaultFileProvider
        // throws on ContainerHeader inside TryLoadPackage(FPackageId), swallows it, and emits SubSequence: null.
        var mainPkg = provider.LoadPackage(MainVfsPath);
        var json = TextOutput.SerializeJson(mainPkg.GetExports().ToArray());
        var exports = JArray.Parse(json);
        var subSequenceToken = exports[0]["Properties"]?["SubSequence"];
        Assert.NotNull(subSequenceToken);
        Assert.Equal(JTokenType.Null, subSequenceToken!.Type);
    }

    [Fact]
    public void DeterministicFileProvider_ResolvesExternalReferencesDeterministicallyInParallel_AcrossThreadsAndJobCounts()
    {
        using var fixture = SyntheticIoStoreFixture.Create();
        var subId = FPackageId.FromName(SubPackageName);
        var baselineJson = (string?)null;

        // Spawn providers on distinct dedicated threads so ConcurrentDictionary object identity hashes
        // vary across threads just like async continuations in CLI subcommands.
        for (var threadRun = 0; threadRun < 6; threadRun++)
        {
            string? runJson = null;
            Exception? threadError = null;
            var thread = new Thread(() =>
            {
                try
                {
                    using var provider = Program.CreateProvider(fixture.PaksDirectory, fixture.UsmapPath);

                    Assert.True(provider.IsMountedDeterministically);
                    Assert.Equal(["Synth-Windows.utoc", "Synth-Windows_0_P.utoc"], provider.MountedVfsOrder);
                    Assert.Equal(1, provider.HeaderlessIoStoreCount);
                    Assert.NotNull(provider.FallbackContainerHeader);

                    Assert.True(provider.FilesById.TryGetValue(subId, out var subFile));
                    var subEntry = Assert.IsType<FIoStoreEntry>(subFile);
                    Assert.Equal("Synth-Windows_0_P.utoc", subEntry.IoStoreReader.Name);

                    foreach (var jobs in new[] { 1, 4, 16 })
                    {
                        var outputs = new string[32];
                        Parallel.For(
                            0,
                            outputs.Length,
                            new ParallelOptions { MaxDegreeOfParallelism = jobs },
                            i =>
                            {
                                CUE4Parse.UE4.Assets.Exports.UObject[] exports;
                                lock (provider.LoadSyncRoot)
                                {
                                    Assert.True(provider.TryLoadPackage(MainVfsPath, out IPackage? pkg) && pkg is not null);
                                    exports = pkg.GetExports().ToArray();
                                }

                                // Serialize outside LoadSyncRoot just like AssetExport.cs so lazy
                                // FPackageIndex -> ImportedPackages -> TryLoadPackage(FPackageId)
                                // executes concurrently across worker threads and synchronizes on LoadSyncRoot.
                                outputs[i] = TextOutput.SerializeJson(exports);
                            });

                        var first = outputs[0];
                        var parsed = JArray.Parse(first);
                        var subSeq = parsed[0]["Properties"]?["SubSequence"] as JObject;
                        Assert.NotNull(subSeq);
                        Assert.Equal("SynthSequence'SynthSubPatched'", subSeq!["ObjectName"]?.Value<string>());
                        Assert.Equal("/Game/Sequencer/SynthSub.0", subSeq["ObjectPath"]?.Value<string>());

                        foreach (var output in outputs)
                        {
                            Assert.Equal(first, output);
                        }

                        runJson ??= first;
                        Assert.Equal(runJson, first);
                    }
                }
                catch (Exception ex)
                {
                    threadError = ex;
                }
            });
            thread.Start();
            thread.Join();

            if (threadError is not null)
            {
                throw new InvalidOperationException("Parallel synthetic provider test failed on worker thread.", threadError);
            }

            Assert.NotNull(runJson);
            baselineJson ??= runJson;
            Assert.Equal(baselineJson, runJson);
        }
    }

    [Fact]
    public void DeterministicFileProvider_RuntimeGuard_RejectsUnmountedOrBypassedMount()
    {
        using var fixture = SyntheticIoStoreFixture.Create();

        // Case 1: not mounted at all.
        using (var unmounted = new DeterministicFileProvider(
            fixture.PaksDirectory,
            SearchOption.TopDirectoryOnly,
            new VersionContainer(EGame.GAME_UE5_6)))
        {
            unmounted.Initialize();
            Assert.False(unmounted.IsMountedDeterministically);
            Assert.Throws<InvalidOperationException>(() => unmounted.EnsureReadyForDeterministicLoads());
        }

        // Case 2: bypassed MountDeterministically() by calling AbstractVfsFileProvider.Mount() directly.
        using (var bypassed = new DeterministicFileProvider(
            fixture.PaksDirectory,
            SearchOption.TopDirectoryOnly,
            new VersionContainer(EGame.GAME_UE5_6)))
        {
            bypassed.MappingsContainer = new FileUsmapTypeMappingsProvider(fixture.UsmapPath);
            bypassed.Initialize();
            ((AbstractVfsFileProvider)bypassed).Mount();

            Assert.False(bypassed.IsMountedDeterministically);
            Assert.Throws<InvalidOperationException>(() => bypassed.EnsureReadyForDeterministicLoads());
            Assert.True(bypassed.TryGetGameFile(MainVfsPath, out var mainFile) && mainFile is not null);
            Assert.Throws<InvalidOperationException>(() => bypassed.LoadPackage(mainFile!));
        }
    }

    private sealed class SyntheticIoStoreFixture : IDisposable
    {
        private readonly string _root;

        private SyntheticIoStoreFixture(string root, string paksDirectory, string usmapPath)
        {
            _root = root;
            PaksDirectory = paksDirectory;
            UsmapPath = usmapPath;
        }

        public string PaksDirectory { get; }

        public string UsmapPath { get; }

        public static SyntheticIoStoreFixture Create()
        {
            var root = Path.Combine(
                AppContext.BaseDirectory,
                "synth-iostore-" + Guid.NewGuid().ToString("N"));
            var paksDir = Path.Combine(root, "Paks");
            Directory.CreateDirectory(paksDir);

            // 1. Minimal .usmap defining SynthSequence { ObjectProperty SubSequence }.
            var usmapPath = Path.Combine(root, "synth.usmap");
            var usmapBytes = SyntheticFixtureBuilder.BuildUsmapV0(
                explicitNameOrder: ["SynthSequence", "SubSequence"],
                enums: [],
                structs:
                [
                    new SyntheticFixtureBuilder.UsmapStructSpec(
                        Name: "SynthSequence",
                        SuperName: null,
                        PropertyCount: 1,
                        Properties:
                        [
                            new SyntheticFixtureBuilder.UsmapPropSpec(
                                SchemaIndex: 0,
                                ArraySize: 1,
                                Name: "SubSequence",
                                PropertyType: (byte)EPropertyType.ObjectProperty),
                        ]),
                ]);
            File.WriteAllBytes(usmapPath, usmapBytes);

            // 2. global.utoc / global.ucas with ScriptObjects chunk (type 5).
            var globalScriptObjects = BuildGlobalScriptObjectsPayload();
            WriteIoStoreContainer(
                Path.Combine(paksDir, "global"),
                containerId: 0x0000000000000001UL,
                indexed: false,
                chunks:
                [
                    (new FIoChunkId(0UL, 0, (byte)EIoChunkType5.ScriptObjects), null, globalScriptObjects),
                ]);

            // 3. Base container Synth-Windows.utoc / .ucas (WITH ContainerHeader chunk type 6).
            var subPackageId = FPackageId.FromName(SubPackageName);
            var mainPackageId = FPackageId.FromName(MainPackageName);

            var subBasePayload = BuildZenPackagePayload(
                packageName: SubPackageName,
                exportObjectName: "SynthSubBase",
                publicExportHash: SynthSubPublicExportHash,
                importedPublicExportHashes: [],
                importMap: [],
                exportData: BuildUnversionedZeroPropertiesPayload());

            var mainPayload = BuildZenPackagePayload(
                packageName: MainPackageName,
                exportObjectName: "SynthMain",
                publicExportHash: 0x2233445566778899UL,
                importedPublicExportHashes: [SynthSubPublicExportHash],
                importMap: [MakePackageImport(importedPackageIndex: 0, importedPublicExportHashIndex: 0)],
                exportData: BuildUnversionedSingleObjectPropertyPayload(packageIndex: -1));

            const ulong baseContainerId = 0x1000000000000001UL;
            var baseContainerHeader = BuildContainerHeaderPayload(
                containerId: baseContainerId,
                entries:
                [
                    (subPackageId, ExportCount: 1, ExportBundleCount: 1, ImportedPackages: []),
                    (mainPackageId, ExportCount: 1, ExportBundleCount: 1, ImportedPackages: [subPackageId]),
                ]);

            WriteIoStoreContainer(
                Path.Combine(paksDir, "Synth-Windows"),
                containerId: baseContainerId,
                indexed: true,
                chunks:
                [
                    (new FIoChunkId(subPackageId.id, 0, (byte)EIoChunkType5.ExportBundleData), SubVfsPath, subBasePayload),
                    (new FIoChunkId(mainPackageId.id, 0, (byte)EIoChunkType5.ExportBundleData), MainVfsPath, mainPayload),
                    (new FIoChunkId(baseContainerId, 0, (byte)EIoChunkType5.ContainerHeader), null, baseContainerHeader),
                ]);

            // 4. Headerless patch container Synth-Windows_0_P.utoc / .ucas (NO ContainerHeader chunk 6!).
            var subPatchedPayload = BuildZenPackagePayload(
                packageName: SubPackageName,
                exportObjectName: "SynthSubPatched",
                publicExportHash: SynthSubPublicExportHash,
                importedPublicExportHashes: [],
                importMap: [],
                exportData: BuildUnversionedZeroPropertiesPayload());

            WriteIoStoreContainer(
                Path.Combine(paksDir, "Synth-Windows_0_P"),
                containerId: 0x2000000000000002UL,
                indexed: true,
                chunks:
                [
                    (new FIoChunkId(subPackageId.id, 0, (byte)EIoChunkType5.ExportBundleData), SubVfsPath, subPatchedPayload),
                ]);

            return new SyntheticIoStoreFixture(root, paksDir, usmapPath);
        }

        public void Dispose()
        {
            if (Directory.Exists(_root))
            {
                Directory.Delete(_root, recursive: true);
            }
        }

        private static ulong MakeScriptImport(ulong id) => (1UL << 62) | id;

        private static ulong MakePackageImport(uint importedPackageIndex, uint importedPublicExportHashIndex)
            => (2UL << 62) | ((ulong)importedPackageIndex << 32) | importedPublicExportHashIndex;

        private static (uint NameIndex, uint ExtraIndex) MakeGlobalMappedName(uint index)
            => ((2u << 30) | index, 0u);

        private static (uint NameIndex, uint ExtraIndex) MakePackageMappedName(uint index)
            => (index, 0u);

        private static byte[] BuildGlobalScriptObjectsPayload()
        {
            using var ms = new MemoryStream();
            using var w = new BinaryWriter(ms, Encoding.UTF8, leaveOpen: true);

            WriteNameBatch(w, ["None", "/Script/Synth", "SynthSequence"]);

            // Two script objects: package /Script/Synth and class SynthSequence.
            w.Write(2);

            // Entry 0: /Script/Synth
            WriteMappedName(w, MakeGlobalMappedName(1));
            w.Write(MakeScriptImport(1)); // GlobalIndex
            w.Write(ulong.MaxValue);      // OuterIndex = Invalid
            w.Write(ulong.MaxValue);      // CDOClassIndex = Invalid

            // Entry 1: SynthSequence (class under /Script/Synth)
            WriteMappedName(w, MakeGlobalMappedName(2));
            w.Write(MakeScriptImport(2)); // GlobalIndex
            w.Write(MakeScriptImport(1)); // OuterIndex = /Script/Synth
            w.Write(ulong.MaxValue);      // CDOClassIndex = Invalid

            w.Flush();
            return ms.ToArray();
        }

        private static byte[] BuildUnversionedZeroPropertiesPayload()
        {
            var buf = new byte[6];
            // FFragment: SkipNum=0, HasAnyZeroes=false, IsLast=true (0x0100), ValueNum=0.
            BinaryPrimitives.WriteUInt16LittleEndian(buf.AsSpan(0, 2), 0x0100);
            // UObject trailing bHasGuid = 0 (false).
            BinaryPrimitives.WriteInt32LittleEndian(buf.AsSpan(2, 4), 0);
            return buf;
        }

        private static byte[] BuildUnversionedSingleObjectPropertyPayload(int packageIndex)
        {
            var buf = new byte[10];
            // FFragment: SkipNum=0, HasAnyZeroes=false, IsLast=true (0x0100), ValueNum=1 (1 << 9 = 0x0200).
            BinaryPrimitives.WriteUInt16LittleEndian(buf.AsSpan(0, 2), 0x0300);
            // ObjectProperty (FPackageIndex) value.
            BinaryPrimitives.WriteInt32LittleEndian(buf.AsSpan(2, 4), packageIndex);
            // UObject trailing bHasGuid = 0 (false).
            BinaryPrimitives.WriteInt32LittleEndian(buf.AsSpan(6, 4), 0);
            return buf;
        }

        private static byte[] BuildZenPackagePayload(
            string packageName,
            string exportObjectName,
            ulong publicExportHash,
            ulong[] importedPublicExportHashes,
            ulong[] importMap,
            byte[] exportData)
        {
            // Build NameMap + BulkDataMap + ImportedPublicExportHashes + ImportMap + ExportMap + ExportBundleEntries.
            byte[] nameAndBulkBytes;
            using (var ms = new MemoryStream())
            using (var w = new BinaryWriter(ms, Encoding.UTF8, leaveOpen: true))
            {
                WriteNameBatch(w, [packageName, exportObjectName]);
                w.Write(0UL); // UE 5.4+ pad size
                w.Write(0UL); // BulkDataMap size = 0
                w.Flush();
                nameAndBulkBytes = ms.ToArray();
            }

            const int summarySize = 52; // FZenPackageSummary for UE 5.3+
            const int cellOffsetsSize = 8; // FZenPackageCellOffsets for Ver >= 1015 (UE 5.6)

            var importedPublicExportHashesOffset = summarySize + cellOffsetsSize + nameAndBulkBytes.Length;
            var importMapOffset = importedPublicExportHashesOffset + (importedPublicExportHashes.Length * 8);
            var exportMapOffset = importMapOffset + (importMap.Length * 8);
            const int exportMapEntrySize = 72;
            var exportBundleEntriesOffset = exportMapOffset + exportMapEntrySize;
            const int exportBundleEntriesSize = 2 * 8; // Create + Serialize for 1 export
            var headerSize = exportBundleEntriesOffset + exportBundleEntriesSize;

            using var pkgStream = new MemoryStream();
            using var pw = new BinaryWriter(pkgStream, Encoding.UTF8, leaveOpen: true);

            // FZenPackageSummary (UE 5.3+ / 5.6)
            pw.Write(0u); // bHasVersioningInfo
            pw.Write((uint)headerSize); // HeaderSize
            WriteMappedName(pw, MakePackageMappedName(0)); // Name = packageName
            pw.Write(0x00002000u); // PackageFlags = PKG_UnversionedProperties
            pw.Write((uint)headerSize); // CookedHeaderSize
            pw.Write(importedPublicExportHashesOffset);
            pw.Write(importMapOffset);
            pw.Write(exportMapOffset);
            pw.Write(exportBundleEntriesOffset);
            pw.Write(headerSize); // DependencyBundleHeadersOffset
            pw.Write(headerSize); // DependencyBundleEntriesOffset
            pw.Write(headerSize); // ImportedPackageNamesOffset

            // FZenPackageCellOffsets (UE 5.6)
            pw.Write(exportBundleEntriesOffset); // CellImportMapOffset
            pw.Write(exportBundleEntriesOffset); // CellExportMapOffset

            // NameMap + empty BulkDataMap
            pw.Write(nameAndBulkBytes);

            // ImportedPublicExportHashes
            foreach (var h in importedPublicExportHashes)
            {
                pw.Write(h);
            }

            // ImportMap
            foreach (var imp in importMap)
            {
                pw.Write(imp);
            }

            // ExportMap (1 entry, 72 bytes)
            var exportEntryStart = pkgStream.Position;
            pw.Write(0UL); // CookedSerialOffset (relative to HeaderSize)
            pw.Write((ulong)exportData.Length); // CookedSerialSize
            WriteMappedName(pw, MakePackageMappedName(1)); // ObjectName = exportObjectName
            pw.Write(ulong.MaxValue); // OuterIndex = package
            pw.Write(MakeScriptImport(2)); // ClassIndex = /Script/Synth.SynthSequence
            pw.Write(ulong.MaxValue); // SuperIndex = Invalid
            pw.Write(ulong.MaxValue); // TemplateIndex = Invalid
            pw.Write(publicExportHash); // PublicExportHash
            pw.Write(0x00000003u); // ObjectFlags = RF_Public | RF_Standalone
            pw.Write((byte)0); // FilterFlags
            while (pkgStream.Position - exportEntryStart < exportMapEntrySize)
            {
                pw.Write((byte)0);
            }

            // ExportBundleEntries: Create (0), Serialize (1)
            pw.Write(0u); // LocalExportIndex = 0
            pw.Write(0u); // EExportCommandType.ExportCommandType_Create
            pw.Write(0u); // LocalExportIndex = 0
            pw.Write(1u); // EExportCommandType.ExportCommandType_Serialize

            // Export 0 serialized data starting at HeaderSize
            pw.Write(exportData);
            pw.Flush();
            return pkgStream.ToArray();
        }

        private static byte[] BuildContainerHeaderPayload(
            ulong containerId,
            IReadOnlyList<(FPackageId PackageId, int ExportCount, int ExportBundleCount, FPackageId[] ImportedPackages)> entries)
        {
            // Build StoreEntries buffer (Version 0 = Initial: 24 bytes per FFilePackageStoreEntry + trailing FPackageId[]).
            byte[] storeEntriesBuffer;
            using (var seStream = new MemoryStream())
            using (var sew = new BinaryWriter(seStream, Encoding.UTF8, leaveOpen: true))
            {
                var fixedTableSize = entries.Count * 24;
                var trailingOffset = fixedTableSize;

                for (var i = 0; i < entries.Count; i++)
                {
                    var entry = entries[i];
                    var entryStart = i * 24;
                    var importedPackagesFieldPos = entryStart + 8;

                    sew.Write(entry.ExportCount);
                    sew.Write(entry.ExportBundleCount);
                    sew.Write(entry.ImportedPackages.Length);
                    sew.Write(entry.ImportedPackages.Length > 0 ? trailingOffset - importedPackagesFieldPos : 0);
                    sew.Write(0); // ShaderMapHashes count
                    sew.Write(0); // ShaderMapHashes offset

                    trailingOffset += entry.ImportedPackages.Length * 8;
                }

                foreach (var entry in entries)
                {
                    foreach (var importedId in entry.ImportedPackages)
                    {
                        sew.Write(importedId.id);
                    }
                }

                sew.Flush();
                storeEntriesBuffer = seStream.ToArray();
            }

            using var ms = new MemoryStream();
            using var w = new BinaryWriter(ms, Encoding.UTF8, leaveOpen: true);

            w.Write(0x496F436Eu); // FIoContainerHeader.Signature ("nCoI")
            w.Write(0); // EIoContainerHeaderVersion.Initial
            w.Write(containerId);
            w.Write((uint)entries.Count); // PackageCount (present when Version < 2)

            // PackageIds array
            w.Write(entries.Count);
            foreach (var entry in entries)
            {
                w.Write(entry.PackageId.id);
            }

            // StoreEntries byte blob
            w.Write(storeEntriesBuffer.Length);
            w.Write(storeEntriesBuffer);

            // ContainerNameMap (empty name batch)
            w.Write(0);
            // PackageRedirects (empty array)
            w.Write(0);

            w.Flush();
            return ms.ToArray();
        }

        private static void WriteIoStoreContainer(
            string basePathWithoutExtension,
            ulong containerId,
            bool indexed,
            IReadOnlyList<(FIoChunkId ChunkId, string? FilePath, byte[] Data)> chunks)
        {
            const uint compressionBlockSize = 65536u;

            // Write .ucas with each chunk as 1 uncompressed block.
            var blockRawOffsets = new ulong[chunks.Count];
            using (var ucas = File.Create(basePathWithoutExtension + ".ucas"))
            {
                for (var i = 0; i < chunks.Count; i++)
                {
                    blockRawOffsets[i] = (ulong)ucas.Position;
                    ucas.Write(chunks[i].Data, 0, chunks[i].Data.Length);
                }
            }

            // Build directory index if requested.
            byte[] directoryIndex = indexed ? BuildDirectoryIndex(chunks) : [];

            using var toc = File.Create(basePathWithoutExtension + ".utoc");
            using var w = new BinaryWriter(toc, Encoding.UTF8, leaveOpen: true);

            // FIoStoreTocHeader (144 bytes, Version 3 = PartitionSize -> no perfect hash table needed).
            w.Write(FIoStoreTocHeader.TOC_MAGIC); // 16B
            w.Write((byte)3); // EIoStoreTocVersion.PartitionSize
            w.Write((byte)0); // _reserved0
            w.Write((ushort)0); // _reserved1
            w.Write(144u); // TocHeaderSize
            w.Write((uint)chunks.Count); // TocEntryCount
            w.Write((uint)chunks.Count); // TocCompressedBlockEntryCount
            w.Write(12u); // TocCompressedBlockEntrySize
            w.Write(1u); // CompressionMethodNameCount (1 empty entry avoids EOF zero-length Serialize in FByteArchive)
            w.Write(32u); // CompressionMethodNameLength
            w.Write(compressionBlockSize); // CompressionBlockSize
            w.Write((uint)directoryIndex.Length); // DirectoryIndexSize
            w.Write(1u); // PartitionCount
            w.Write(containerId); // ContainerId
            w.Write(new byte[16]); // EncryptionKeyGuid
            w.Write(indexed ? (byte)8 : (byte)0); // EIoContainerFlags (Indexed = 8)
            w.Write((byte)0); // EncryptionMethod = None
            w.Write((ushort)0); // _reserved4
            w.Write(0u); // TocChunkPerfectHashSeedsCount
            w.Write(ulong.MaxValue); // PartitionSize
            w.Write(0u); // TocChunksWithoutPerfectHashCount
            w.Write(0u); // reserved / skipped for UE < 6.0
            for (var i = 0; i < 5; i++)
            {
                w.Write(0UL); // CustomGameData[5]
            }

            // ChunkIds[N] (12 bytes each: uint64 ChunkId LE + uint16 chunkIndex BE + byte padding + byte ChunkType).
            foreach (var (chunkId, _, _) in chunks)
            {
                w.Write(chunkId.ChunkId);
                w.Write((ushort)0); // chunkIndex = 0
                w.Write((byte)0);   // padding
                w.Write(chunkId.ChunkType);
            }

            // ChunkOffsetLengths[N] (10 bytes each: 5B BE logical offset + 5B BE length).
            // Place chunk i at logical offset i * compressionBlockSize so block index == i.
            for (var i = 0; i < chunks.Count; i++)
            {
                var logicalOffset = (ulong)i * compressionBlockSize;
                var length = (ulong)chunks[i].Data.Length;
                WriteUInt40BigEndian(w, logicalOffset);
                WriteUInt40BigEndian(w, length);
            }

            // CompressionBlocks[N] (12 bytes each).
            for (var i = 0; i < chunks.Count; i++)
            {
                var len = (uint)chunks[i].Data.Length;
                var offsetAndCompSize = (blockRawOffsets[i] & 0xFFFFFFFFFFUL) | ((ulong)len << 40);
                var uncompSizeAndMethod = len & 0xFFFFFFu; // method 0 = None
                w.Write(offsetAndCompSize);
                w.Write(uncompSizeAndMethod);
            }

            // CompressionMethodNames (1 x 32-byte zeroed entry).
            w.Write(new byte[32]);

            // DirectoryIndex buffer (if indexed).
            if (directoryIndex.Length > 0)
            {
                w.Write(directoryIndex);
            }

            w.Flush();
        }

        private static byte[] BuildDirectoryIndex(IReadOnlyList<(FIoChunkId ChunkId, string? FilePath, byte[] Data)> chunks)
        {
            var files = new List<(string Path, uint ChunkIndex)>();
            for (var i = 0; i < chunks.Count; i++)
            {
                if (chunks[i].FilePath is { } path)
                {
                    files.Add((path, (uint)i));
                }
            }

            using var ms = new MemoryStream();
            using var w = new BinaryWriter(ms, Encoding.UTF8, leaveOpen: true);

            w.Write(SyntheticFixtureBuilder.EncodeFString("../../../"));

            // 1 root directory entry.
            w.Write(1);
            w.Write(uint.MaxValue); // Name = none
            w.Write(uint.MaxValue); // FirstChildEntry = none
            w.Write(uint.MaxValue); // NextSiblingEntry = none
            w.Write(files.Count > 0 ? 0u : uint.MaxValue); // FirstFileEntry

            // File entries linked sequentially under root.
            w.Write(files.Count);
            for (var i = 0; i < files.Count; i++)
            {
                w.Write((uint)i); // String table index
                w.Write(i + 1 < files.Count ? (uint)(i + 1) : uint.MaxValue); // NextFileEntry
                w.Write(files[i].ChunkIndex); // UserData = TOC chunk index
            }

            // String table.
            w.Write(files.Count);
            foreach (var (path, _) in files)
            {
                w.Write(SyntheticFixtureBuilder.EncodeFString(path));
            }

            w.Flush();
            return ms.ToArray();
        }

        private static void WriteNameBatch(BinaryWriter w, IReadOnlyList<string> names)
        {
            w.Write(names.Count);
            if (names.Count == 0)
            {
                return;
            }

            var encoded = names.Select(Encoding.UTF8.GetBytes).ToArray();
            w.Write(encoded.Sum(b => b.Length)); // numStringBytes
            w.Write(0UL); // hashVersion
            for (var i = 0; i < encoded.Length; i++)
            {
                w.Write(0UL); // hash[i]
            }

            foreach (var bytes in encoded)
            {
                // FSerializedNameHeader: bit 7 of byte0 = IsUtf16 (0), bits 0..6 + byte1 = Length.
                w.Write((byte)((bytes.Length >> 8) & 0x7F));
                w.Write((byte)(bytes.Length & 0xFF));
            }

            foreach (var bytes in encoded)
            {
                w.Write(bytes);
            }
        }

        private static void WriteMappedName(BinaryWriter w, (uint NameIndex, uint ExtraIndex) mappedName)
        {
            w.Write(mappedName.NameIndex);
            w.Write(mappedName.ExtraIndex);
        }

        private static void WriteUInt40BigEndian(BinaryWriter w, ulong value)
        {
            w.Write((byte)((value >> 32) & 0xFF));
            w.Write((byte)((value >> 24) & 0xFF));
            w.Write((byte)((value >> 16) & 0xFF));
            w.Write((byte)((value >> 8) & 0xFF));
            w.Write((byte)(value & 0xFF));
        }
    }
}
