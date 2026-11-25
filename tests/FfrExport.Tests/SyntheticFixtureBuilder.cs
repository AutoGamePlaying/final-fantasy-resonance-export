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
