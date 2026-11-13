using System.Buffers.Binary;

namespace Retrograde.Core;

/// <summary>
/// Minimal read-only LevelDB reader (log + table files, Snappy blocks), enough to read the
/// Electron build's localStorage. Returns the newest live value of every key.
/// </summary>
public static class LevelDbReader
{
    public static Dictionary<string, byte[]> ReadAll(string dir)
    {
        var newest = new Dictionary<string, (ulong Seq, byte[]? Value)>(StringComparer.Ordinal);
        void Put(byte[] key, ulong seq, byte[]? value)
        {
            var k = Convert.ToBase64String(key);
            if (!newest.TryGetValue(k, out var cur) || seq >= cur.Seq) newest[k] = (seq, value);
        }

        foreach (var f in Directory.EnumerateFiles(dir))
        {
            var ext = Path.GetExtension(f).ToLowerInvariant();
            try
            {
                var bytes = ReadShared(f);
                if (ext is ".ldb" or ".sst") ReadTable(bytes, Put);
                else if (ext == ".log") ReadLog(bytes, Put);
            }
            catch { /* a damaged file only loses its own entries */ }
        }

        var result = new Dictionary<string, byte[]>(StringComparer.Ordinal);
        foreach (var (k, v) in newest)
            if (v.Value is not null) result[k] = v.Value;
        return result;
    }

    /// <summary>The live database may be open in Electron, so read with sharing.</summary>
    private static byte[] ReadShared(string path)
    {
        using var fs = new FileStream(path, FileMode.Open, FileAccess.Read, FileShare.ReadWrite | FileShare.Delete);
        var buf = new byte[fs.Length];
        fs.ReadExactly(buf);
        return buf;
    }

    // ---- log files: 32KB blocks of records carrying WriteBatches ----
    private static void ReadLog(byte[] data, Action<byte[], ulong, byte[]?> put)
    {
        const int block = 32768;
        var pending = new List<byte>();
        int pos = 0;
        while (pos + 7 <= data.Length)
        {
            int left = block - pos % block;
            if (left < 7) { pos += left; continue; }
            int len = data[pos + 4] | data[pos + 5] << 8;
            byte type = data[pos + 6];
            int start = pos + 7;
            if (type == 0 && len == 0) { pos += left; continue; } // zero padding
            if (start + len > data.Length) break;
            var payload = new ReadOnlySpan<byte>(data, start, len);
            switch (type)
            {
                case 1: ApplyBatch(payload, put); pending.Clear(); break;
                case 2: pending.Clear(); pending.AddRange(payload.ToArray()); break;
                case 3: pending.AddRange(payload.ToArray()); break;
                case 4: pending.AddRange(payload.ToArray()); ApplyBatch(pending.ToArray(), put); pending.Clear(); break;
            }
            pos = start + len;
        }
    }

    private static void ApplyBatch(ReadOnlySpan<byte> b, Action<byte[], ulong, byte[]?> put)
    {
        if (b.Length < 12) return;
        ulong seq = BinaryPrimitives.ReadUInt64LittleEndian(b);
        int count = (int)BinaryPrimitives.ReadUInt32LittleEndian(b[8..]);
        int p = 12;
        for (int i = 0; i < count && p < b.Length; i++, seq++)
        {
            byte kind = b[p++];
            var key = ReadSlice(b, ref p);
            if (kind == 1) put(key, seq, ReadSlice(b, ref p));
            else put(key, seq, null);
        }
    }

    // ---- table files ----
    private static void ReadTable(byte[] data, Action<byte[], ulong, byte[]?> put)
    {
        if (data.Length < 48) return;
        var footer = new ReadOnlySpan<byte>(data, data.Length - 48, 48);
        int p = 0;
        ReadVarint64(footer, ref p); ReadVarint64(footer, ref p); // metaindex handle
        long idxOff = (long)ReadVarint64(footer, ref p);
        long idxSize = (long)ReadVarint64(footer, ref p);
        var index = ReadBlock(data, idxOff, idxSize);
        foreach (var (_, handle) in BlockEntries(index))
        {
            int hp = 0;
            long off = (long)ReadVarint64(handle, ref hp);
            long size = (long)ReadVarint64(handle, ref hp);
            foreach (var (ikey, value) in BlockEntries(ReadBlock(data, off, size)))
            {
                if (ikey.Length < 8) continue;
                ulong trailer = BinaryPrimitives.ReadUInt64LittleEndian(ikey.AsSpan(ikey.Length - 8));
                var userKey = ikey[..^8];
                put(userKey, trailer >> 8, (trailer & 0xFF) == 1 ? value : null);
            }
        }
    }

    private static byte[] ReadBlock(byte[] data, long off, long size)
    {
        var raw = new ReadOnlySpan<byte>(data, (int)off, (int)size);
        byte type = data[off + size];
        return type == 1 ? Snappy.Decompress(raw) : raw.ToArray();
    }

    private static IEnumerable<(byte[] Key, byte[] Value)> BlockEntries(byte[] block)
    {
        if (block.Length < 4) yield break;
        int restarts = (int)BinaryPrimitives.ReadUInt32LittleEndian(block.AsSpan(block.Length - 4));
        int end = block.Length - 4 - restarts * 4;
        int p = 0;
        byte[] last = [];
        while (p < end)
        {
            int shared = (int)ReadVarint32(block, ref p);
            int nonShared = (int)ReadVarint32(block, ref p);
            int valueLen = (int)ReadVarint32(block, ref p);
            var key = new byte[shared + nonShared];
            Array.Copy(last, key, shared);
            Array.Copy(block, p, key, shared, nonShared);
            p += nonShared;
            var value = block.AsSpan(p, valueLen).ToArray();
            p += valueLen;
            last = key;
            yield return (key, value);
        }
    }

    private static byte[] ReadSlice(ReadOnlySpan<byte> b, ref int p)
    {
        int len = (int)ReadVarint32(b, ref p);
        var s = b.Slice(p, len).ToArray();
        p += len;
        return s;
    }

    private static uint ReadVarint32(ReadOnlySpan<byte> b, ref int p) => (uint)ReadVarint64(b, ref p);

    private static ulong ReadVarint64(ReadOnlySpan<byte> b, ref int p)
    {
        ulong result = 0;
        for (int shift = 0; shift < 64; shift += 7)
        {
            byte x = b[p++];
            result |= (ulong)(x & 0x7F) << shift;
            if ((x & 0x80) == 0) break;
        }
        return result;
    }

    private static class Snappy
    {
        public static byte[] Decompress(ReadOnlySpan<byte> src)
        {
            int p = 0;
            int len = (int)ReadVarint32(src, ref p);
            var dst = new byte[len];
            int d = 0;
            while (p < src.Length && d < len)
            {
                byte tag = src[p++];
                switch (tag & 3)
                {
                    case 0:
                    {
                        int n = tag >> 2;
                        if (n >= 60)
                        {
                            int extra = n - 59;
                            n = 0;
                            for (int i = 0; i < extra; i++) n |= src[p++] << (8 * i);
                        }
                        n += 1;
                        src.Slice(p, n).CopyTo(dst.AsSpan(d));
                        p += n; d += n;
                        break;
                    }
                    case 1:
                    {
                        int n = ((tag >> 2) & 7) + 4;
                        int off = ((tag >> 5) << 8) | src[p++];
                        Copy(dst, ref d, off, n);
                        break;
                    }
                    case 2:
                    {
                        int n = (tag >> 2) + 1;
                        int off = src[p] | src[p + 1] << 8;
                        p += 2;
                        Copy(dst, ref d, off, n);
                        break;
                    }
                    default:
                    {
                        int n = (tag >> 2) + 1;
                        int off = (int)BinaryPrimitives.ReadUInt32LittleEndian(src[p..]);
                        p += 4;
                        Copy(dst, ref d, off, n);
                        break;
                    }
                }
            }
            return dst;
        }

        // Overlapping copies are allowed (run-length style), so copy byte by byte.
        private static void Copy(byte[] dst, ref int d, int off, int n)
        {
            int s = d - off;
            for (int i = 0; i < n; i++) dst[d++] = dst[s + i];
        }
    }
}
