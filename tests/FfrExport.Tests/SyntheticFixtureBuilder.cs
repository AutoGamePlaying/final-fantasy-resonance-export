using System.Buffers.Binary;
using System.Security.Cryptography;
using System.Text;

namespace FfrExport.Tests;

internal sealed record FixtureAsset(
    string PackageName,
    string PackagePath,
    string AssetName,
    string AssetClass,
    IReadOnlyDictionary<string, string>? Tags = null);

internal sealed record PakFileEntry(string Path, byte[] Data);

/// <summary>
/// Builds minimal in-memory Unreal binary fixtures (.pak v3, AssetRegistry.bin v7, .usmap v0,
/// and .locres v1) parseable by CUE4Parse with zero dependency on real game files.
/// </summary>
internal static class SyntheticFixtureBuilder
{
    public static byte[] EncodeFString(string text)
    {
        if (text.Length == 0)
        {
            return new byte[4];
        }

        var isAscii = text.All(ch => ch <= 0x7F);
        if (isAscii)
        {
            var bytes = Encoding.ASCII.GetBytes(text);
            var buffer = new byte[4 + bytes.Length + 1];
            BinaryPrimitives.WriteInt32LittleEndian(buffer, bytes.Length + 1);
            Buffer.BlockCopy(bytes, 0, buffer, 4, bytes.Length);
            buffer[4 + bytes.Length] = 0;
            return buffer;
        }
        else
        {
            var bytes = Encoding.Unicode.GetBytes(text);
            var charCountWithNull = text.Length + 1;
            var buffer = new byte[4 + bytes.Length + 2];
            BinaryPrimitives.WriteInt32LittleEndian(buffer, -charCountWithNull);
            Buffer.BlockCopy(bytes, 0, buffer, 4, bytes.Length);
            return buffer;
        }
    }

    public static byte[] BuildAssetRegistry(IReadOnlyList<FixtureAsset> assets)
    {
        var nameToIndex = new Dictionary<string, int>(StringComparer.Ordinal);
        var names = new List<string>();

        int Intern(string name)
        {
            if (nameToIndex.TryGetValue(name, out var existing))
            {
                return existing;
            }

            var idx = names.Count;
            names.Add(name);
            nameToIndex[name] = idx;
            return idx;
        }

        void WriteFName(BinaryWriter writer, string name)
        {
            writer.Write(Intern(name));
            writer.Write(0);
        }

        byte[] body;
        using (var bodyStream = new MemoryStream())
        using (var writer = new BinaryWriter(bodyStream, Encoding.UTF8, leaveOpen: true))
        {
            writer.Write(assets.Count);
            foreach (var asset in assets)
            {
                var objectPath = $"{asset.PackageName}.{asset.AssetName}";
                WriteFName(writer, objectPath);
                WriteFName(writer, asset.PackagePath);
                WriteFName(writer, asset.AssetClass);
                WriteFName(writer, asset.PackageName);
                WriteFName(writer, asset.AssetName);

                var tags = asset.Tags ?? new Dictionary<string, string>();
                writer.Write(tags.Count);
                foreach (var (key, value) in tags)
                {
                    WriteFName(writer, key);
                    writer.Write(EncodeFString(value));
                }

                writer.Write(0); // ChunkIDs count
                writer.Write(0u); // PackageFlags
            }

            // DependsNodes (0 count, 0 section size) + PackageData (0 count)
            writer.Write(0);
            writer.Write(0L);
            writer.Write(0);
            writer.Flush();
            body = bodyStream.ToArray();
        }

        const int headerSize = 16 + 4 + 8; // GUID + Version + NameOffset
        var nameTableOffset = (long)(headerSize + body.Length);

        using var fullStream = new MemoryStream();
        using var fullWriter = new BinaryWriter(fullStream, Encoding.UTF8, leaveOpen: true);

        // FAssetRegistryVersion.GUID = 717F9EE7-E9B0493A-88B39132-1B388107
        fullWriter.Write(0x717f9ee7u);
        fullWriter.Write(0xe9b0493au);
        fullWriter.Write(0x88b39132u);
        fullWriter.Write(0x1b388107u);
        fullWriter.Write(7); // AddedDependencyFlags
        fullWriter.Write(nameTableOffset);
        fullWriter.Write(body);

        fullWriter.Write(names.Count);
        foreach (var name in names)
        {
            fullWriter.Write(EncodeFString(name));
            fullWriter.Write(0u); // NonCasePreservingHash + CasePreservingHash
        }

        fullWriter.Flush();
        return fullStream.ToArray();
    }

    public static byte[] BuildPak(IReadOnlyList<PakFileEntry> entries)
    {
        using var payloadStream = new MemoryStream();
        var offsets = new long[entries.Count];

        for (var i = 0; i < entries.Count; i++)
        {
            offsets[i] = payloadStream.Position;
            var localHeader = EncodePakEntryRecord(0, entries[i].Data);
            payloadStream.Write(localHeader);
            payloadStream.Write(entries[i].Data);
        }

        var indexOffset = payloadStream.Position;

        byte[] indexBytes;
        using (var indexStream = new MemoryStream())
        using (var indexWriter = new BinaryWriter(indexStream, Encoding.UTF8, leaveOpen: true))
        {
            indexWriter.Write(EncodeFString("../../../"));
            indexWriter.Write(entries.Count);
            for (var i = 0; i < entries.Count; i++)
            {
                indexWriter.Write(EncodeFString(entries[i].Path));
                indexWriter.Write(EncodePakEntryRecord(offsets[i], entries[i].Data));
            }

            indexWriter.Flush();
            indexBytes = indexStream.ToArray();
        }

        var indexHash = SHA1.HashData(indexBytes);
        var footer = new byte[44];
        BinaryPrimitives.WriteUInt32LittleEndian(footer.AsSpan(0, 4), 0x5a6f12e1u); // FPakInfo.PAK_FILE_MAGIC
        BinaryPrimitives.WriteInt32LittleEndian(footer.AsSpan(4, 4), 3); // PakFile_Version_CompressionEncryption
        BinaryPrimitives.WriteInt64LittleEndian(footer.AsSpan(8, 8), indexOffset);
        BinaryPrimitives.WriteInt64LittleEndian(footer.AsSpan(16, 8), indexBytes.Length);
        Buffer.BlockCopy(indexHash, 0, footer, 24, 20);

        payloadStream.Write(indexBytes);
        payloadStream.Write(footer);
        return payloadStream.ToArray();
    }

    public static byte[] BuildUsmap()
    {
        var buffer = new byte[24];
        BinaryPrimitives.WriteUInt16LittleEndian(buffer.AsSpan(0, 2), 0x30c4); // Magic
        buffer[2] = 0; // Version = Initial
        buffer[3] = 0; // CompressionMethod = None
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(4, 4), 12u); // CompressedSize
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(8, 4), 12u); // DecompressedSize
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(12, 4), 0u); // NameCount
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(16, 4), 0u); // EnumCount
        BinaryPrimitives.WriteUInt32LittleEndian(buffer.AsSpan(20, 4), 0u); // StructCount
        return buffer;
    }

    internal sealed record UsmapPropSpec(
        ushort SchemaIndex,
        byte ArraySize,
        string Name,
        byte PropertyType,
        string? StructName = null,
        string? EnumName = null,
        byte? EnumInnerType = null,
        byte? InnerType = null,
        byte? ValueType = null);

    internal sealed record UsmapStructSpec(
        string Name,
        string? SuperName,
        ushort PropertyCount,
        IReadOnlyList<UsmapPropSpec> Properties);

    internal sealed record UsmapEnumSpec(
        string Name,
        IReadOnlyList<string> Members);

    /// <summary>
    /// Builds a valid v0 uncompressed .usmap binary from structured enum and struct definitions.
    /// Allows caller-specified name table ordering to test order-independent content hashing.
    /// </summary>
    public static byte[] BuildUsmapV0(
        IReadOnlyList<string> explicitNameOrder,
        IReadOnlyList<UsmapEnumSpec> enums,
        IReadOnlyList<UsmapStructSpec> structs)
    {
        var nameIndex = new Dictionary<string, int>(StringComparer.Ordinal);
        var names = new List<string>();

        int Intern(string name)
        {
            if (nameIndex.TryGetValue(name, out var idx))
            {
                return idx;
            }

            var next = names.Count;
            names.Add(name);
            nameIndex[name] = next;
            return next;
        }

        foreach (var pre in explicitNameOrder)
        {
            Intern(pre);
        }

        foreach (var e in enums)
        {
            Intern(e.Name);
            foreach (var m in e.Members)
            {
                Intern(m);
            }
        }

        foreach (var s in structs)
        {
            Intern(s.Name);
            if (s.SuperName is not null)
            {
                Intern(s.SuperName);
            }

            foreach (var p in s.Properties)
            {
                Intern(p.Name);
                if (p.StructName is not null)
                {
                    Intern(p.StructName);
                }

                if (p.EnumName is not null)
                {
                    Intern(p.EnumName);
                }
            }
        }

        using var payload = new MemoryStream();
        using (var w = new BinaryWriter(payload, Encoding.UTF8, leaveOpen: true))
        {
            w.Write((uint)names.Count);
            foreach (var n in names)
            {
                var bytes = Encoding.UTF8.GetBytes(n);
                w.Write((byte)bytes.Length);
                w.Write(bytes);
            }

            w.Write((uint)enums.Count);
            foreach (var e in enums)
            {
                w.Write(Intern(e.Name));
                w.Write((byte)e.Members.Count);
                foreach (var m in e.Members)
                {
                    w.Write(Intern(m));
                }
            }

            w.Write((uint)structs.Count);
            foreach (var s in structs)
            {
                w.Write(Intern(s.Name));
                w.Write(s.SuperName is null ? -1 : Intern(s.SuperName));
                w.Write(s.PropertyCount);
                w.Write((ushort)s.Properties.Count);
                foreach (var p in s.Properties)
                {
                    w.Write(p.SchemaIndex);
                    w.Write(p.ArraySize);
                    w.Write(Intern(p.Name));
                    w.Write(p.PropertyType);
                    // StructProperty = 9, ArrayProperty = 8, SetProperty = 25, MapProperty = 24, EnumProperty = 26
                    if (p.PropertyType == 9)
                    {
                        w.Write(Intern(p.StructName!));
                    }
                    else if (p.PropertyType == 26)
                    {
                        w.Write(p.EnumInnerType ?? (byte)0); // ByteProperty
                        w.Write(Intern(p.EnumName!));
                    }
                    else if (p.PropertyType is 8 or 25)
                    {
                        w.Write(p.InnerType ?? (byte)2); // IntProperty
                    }
                    else if (p.PropertyType == 24)
                    {
                        w.Write(p.InnerType ?? (byte)10); // StrProperty
                        w.Write(p.ValueType ?? (byte)2); // IntProperty
                    }
                }
            }

            w.Flush();
        }

        var data = payload.ToArray();
        var result = new byte[12 + data.Length];
        BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(0, 2), 0x30c4); // Magic
        result[2] = 0; // Version = Initial
        result[3] = 0; // CompressionMethod = None
        BinaryPrimitives.WriteUInt32LittleEndian(result.AsSpan(4, 4), (uint)data.Length);
        BinaryPrimitives.WriteUInt32LittleEndian(result.AsSpan(8, 4), (uint)data.Length);
        Buffer.BlockCopy(data, 0, result, 12, data.Length);
        return result;
    }

    /// <summary>
    /// Builds a minimal valid PE (PE32 or PE32+) binary containing an RT_VERSION resource
    /// in a caller-chosen section name (e.g. ".rsrc" or ".link") with VS_FIXEDFILEINFO
    /// and optional StringFileInfo FileVersion/ProductVersion entries.
    /// </summary>
    public static byte[] BuildMinimalPeWithVersionResource(
        (ushort Major, ushort Minor, ushort Build, ushort Revision) fileVersion,
        (ushort Major, ushort Minor, ushort Build, ushort Revision) productVersion,
        string? stringFileVersion = null,
        string? stringProductVersion = null,
        string sectionName = ".rsrc",
        bool pe32Plus = true)
    {
        var versionBlob = BuildVersionResourceBlob(
            fileVersion,
            productVersion,
            stringFileVersion,
            stringProductVersion);

        const uint rsrcRva = 0x2000;
        const uint rsrcRawOffset = 0x200;

        // 3 resource directory levels (16 + 8 = 24 bytes each) + 1 data entry (16 bytes) = 88 bytes
        const uint dir0Offset = 0;
        const uint dir1Offset = 24;
        const uint dir2Offset = 48;
        const uint dataEntryOffset = 72;
        const uint blobOffsetInSection = 88;

        var rsrcSection = new byte[AlignUp((int)blobOffsetInSection + versionBlob.Length, 0x200)];

        // Level 1: RT_VERSION (ID 16) -> subdir dir1Offset
        BinaryPrimitives.WriteUInt16LittleEndian(rsrcSection.AsSpan((int)dir0Offset + 14, 2), 1); // IdEntries
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir0Offset + 16, 4), 16u);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir0Offset + 20, 4), 0x80000000u | dir1Offset);

        // Level 2: Resource ID 1 -> subdir dir2Offset
        BinaryPrimitives.WriteUInt16LittleEndian(rsrcSection.AsSpan((int)dir1Offset + 14, 2), 1);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir1Offset + 16, 4), 1u);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir1Offset + 20, 4), 0x80000000u | dir2Offset);

        // Level 3: Language 0x0409 -> leaf dataEntryOffset
        BinaryPrimitives.WriteUInt16LittleEndian(rsrcSection.AsSpan((int)dir2Offset + 14, 2), 1);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir2Offset + 16, 4), 0x0409u);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dir2Offset + 20, 4), dataEntryOffset);

        // IMAGE_RESOURCE_DATA_ENTRY
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dataEntryOffset + 0, 4), rsrcRva + blobOffsetInSection);
        BinaryPrimitives.WriteUInt32LittleEndian(rsrcSection.AsSpan((int)dataEntryOffset + 4, 4), (uint)versionBlob.Length);
        Buffer.BlockCopy(versionBlob, 0, rsrcSection, (int)blobOffsetInSection, versionBlob.Length);

        var pe = new byte[rsrcRawOffset + rsrcSection.Length];

        // DOS header
        pe[0] = (byte)'M';
        pe[1] = (byte)'Z';
        const int peOff = 0x80;
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(0x3C, 4), peOff);

        // PE signature + COFF header
        pe[peOff + 0] = (byte)'P';
        pe[peOff + 1] = (byte)'E';
        pe[peOff + 2] = 0;
        pe[peOff + 3] = 0;
        BinaryPrimitives.WriteUInt16LittleEndian(pe.AsSpan(peOff + 4, 2), pe32Plus ? (ushort)0x8664 : (ushort)0x014C);
        BinaryPrimitives.WriteUInt16LittleEndian(pe.AsSpan(peOff + 6, 2), 1); // NumberOfSections

        ushort optSize = pe32Plus ? (ushort)240 : (ushort)224;
        BinaryPrimitives.WriteUInt16LittleEndian(pe.AsSpan(peOff + 20, 2), optSize);

        var optOff = peOff + 24;
        BinaryPrimitives.WriteUInt16LittleEndian(pe.AsSpan(optOff, 2), pe32Plus ? (ushort)0x020B : (ushort)0x010B);
        var numRvaOff = optOff + (pe32Plus ? 108 : 92);
        var ddOff = optOff + (pe32Plus ? 112 : 96);
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(numRvaOff, 4), 16u);
        // DataDirectory[2] = Resource Table
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(ddOff + 16, 4), rsrcRva);
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(ddOff + 20, 4), (uint)rsrcSection.Length);

        // Section Header[0]
        var secOff = optOff + optSize;
        var secNameBytes = Encoding.ASCII.GetBytes(sectionName);
        Buffer.BlockCopy(secNameBytes, 0, pe, secOff, Math.Min(8, secNameBytes.Length));
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(secOff + 8, 4), (uint)rsrcSection.Length);
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(secOff + 12, 4), rsrcRva);
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(secOff + 16, 4), (uint)rsrcSection.Length);
        BinaryPrimitives.WriteUInt32LittleEndian(pe.AsSpan(secOff + 20, 4), rsrcRawOffset);

        Buffer.BlockCopy(rsrcSection, 0, pe, (int)rsrcRawOffset, rsrcSection.Length);
        return pe;
    }

    private static byte[] BuildVersionResourceBlob(
        (ushort Major, ushort Minor, ushort Build, ushort Revision) fileVersion,
        (ushort Major, ushort Minor, ushort Build, ushort Revision) productVersion,
        string? stringFileVersion,
        string? stringProductVersion)
    {
        var fixedInfo = new byte[52];
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(0, 4), 0xFEEF04BDu);
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(4, 4), 0x00010000u);
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(8, 4), ((uint)fileVersion.Major << 16) | fileVersion.Minor);
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(12, 4), ((uint)fileVersion.Build << 16) | fileVersion.Revision);
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(16, 4), ((uint)productVersion.Major << 16) | productVersion.Minor);
        BinaryPrimitives.WriteUInt32LittleEndian(fixedInfo.AsSpan(20, 4), ((uint)productVersion.Build << 16) | productVersion.Revision);

        var children = new List<byte[]>();
        if (stringFileVersion is not null || stringProductVersion is not null)
        {
            var stringNodes = new List<byte[]>();
            if (stringFileVersion is not null)
            {
                stringNodes.Add(BuildVersionStringNode("FileVersion", stringFileVersion));
            }

            if (stringProductVersion is not null)
            {
                stringNodes.Add(BuildVersionStringNode("ProductVersion", stringProductVersion));
            }

            var tableNode = BuildVersionContainerNode("040904b0", 1, [], stringNodes);
            var sfiNode = BuildVersionContainerNode("StringFileInfo", 1, [], [tableNode]);
            children.Add(sfiNode);
        }

        return BuildVersionContainerNode("VS_VERSION_INFO", 0, fixedInfo, children);
    }

    private static byte[] BuildVersionStringNode(string key, string value)
    {
        var valueBytes = Encoding.Unicode.GetBytes(value + "\0");
        var charLenWithNull = (ushort)(value.Length + 1);
        return BuildVersionNodeRaw(key, wType: 1, wValueLength: charLenWithNull, valueBytes, []);
    }

    private static byte[] BuildVersionContainerNode(
        string key,
        ushort wType,
        byte[] valueBytes,
        IReadOnlyList<byte[]> children)
    {
        return BuildVersionNodeRaw(key, wType, (ushort)valueBytes.Length, valueBytes, children);
    }

    private static byte[] BuildVersionNodeRaw(
        string key,
        ushort wType,
        ushort wValueLength,
        byte[] valueBytes,
        IReadOnlyList<byte[]> children)
    {
        using var ms = new MemoryStream();
        ms.Write(new byte[6]); // Placeholder for wLength, wValueLength, wType
        var keyBytes = Encoding.Unicode.GetBytes(key + "\0");
        ms.Write(keyBytes);
        PadTo4(ms);
        if (valueBytes.Length > 0)
        {
            ms.Write(valueBytes);
        }

        foreach (var child in children)
        {
            PadTo4(ms);
            ms.Write(child);
        }

        var result = ms.ToArray();
        BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(0, 2), (ushort)result.Length);
        BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(2, 2), wValueLength);
        BinaryPrimitives.WriteUInt16LittleEndian(result.AsSpan(4, 2), wType);
        return result;
    }

    private static void PadTo4(MemoryStream ms)
    {
        while (ms.Length % 4 != 0)
        {
            ms.WriteByte(0);
        }
    }

    private static int AlignUp(int value, int alignment) => (value + alignment - 1) & ~(alignment - 1);

    /// <summary>
    /// Builds a valid UE FTextLocalizationResource binary (Version 1 = Compact)
    /// supporting ASCII and Unicode (UTF-16LE) strings.
    /// </summary>
    public static byte[] BuildLocres(
        IReadOnlyDictionary<string, IReadOnlyDictionary<string, string>> namespaces)
    {
        var strings = new List<string>();
        byte[] entriesSection;
        using (var entryStream = new MemoryStream())
        using (var writer = new BinaryWriter(entryStream, Encoding.UTF8, leaveOpen: true))
        {
            writer.Write((uint)namespaces.Count);
            foreach (var (ns, entries) in namespaces)
            {
                writer.Write(EncodeFString(ns));
                writer.Write((uint)entries.Count);
                foreach (var (key, localizedValue) in entries)
                {
                    writer.Write(EncodeFString(key));
                    writer.Write(0u); // SourceStringHash
                    var stringIndex = strings.Count;
                    strings.Add(localizedValue);
                    writer.Write(stringIndex);
                }
            }

            writer.Flush();
            entriesSection = entryStream.ToArray();
        }

        const long headerSize = 16 + 1 + 8; // Magic + Version + LocalizedStringArrayOffset
        var stringArrayOffset = headerSize + entriesSection.Length;

        using var stream = new MemoryStream();
        using var fullWriter = new BinaryWriter(stream, Encoding.UTF8, leaveOpen: true);

        // FTextLocalizationResource MagicNumber
        fullWriter.Write(0x7574140Eu);
        fullWriter.Write(0xFC034A67u);
        fullWriter.Write(0x9D90154Au);
        fullWriter.Write(0x1B7F37C3u);
        fullWriter.Write((byte)1); // Compact
        fullWriter.Write(stringArrayOffset);
        fullWriter.Write(entriesSection);

        fullWriter.Write(strings.Count);
        foreach (var text in strings)
        {
            fullWriter.Write(EncodeFString(text));
        }

        fullWriter.Flush();
        return stream.ToArray();
    }

    private static byte[] EncodePakEntryRecord(long offset, byte[] data)
    {
        var hash = SHA1.HashData(data);
        var buf = new byte[8 + 8 + 8 + 4 + 20 + 1 + 4];
        BinaryPrimitives.WriteInt64LittleEndian(buf.AsSpan(0, 8), offset);
        BinaryPrimitives.WriteInt64LittleEndian(buf.AsSpan(8, 8), data.Length);
        BinaryPrimitives.WriteInt64LittleEndian(buf.AsSpan(16, 8), data.Length);
        BinaryPrimitives.WriteInt32LittleEndian(buf.AsSpan(24, 4), 0); // CompressionMethod = None
        Buffer.BlockCopy(hash, 0, buf, 28, 20);
        buf[48] = 0; // Flags
        BinaryPrimitives.WriteInt32LittleEndian(buf.AsSpan(49, 4), 0); // CompressionBlockSize
        return buf;
    }
}
