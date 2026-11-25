namespace FfrExport;

/// <summary>
/// Cross-platform path helpers supporting spaces, cross-drive paths, and Windows paths over 260 characters.
/// </summary>
internal static class PathSupport
{
    private const int ExtendedPathThreshold = 240;

    /// <summary>
    /// Converts a path into a safe I/O path, adding the Windows extended-length prefix when needed
    /// so deep export trees exceed MAX_PATH even if OS-wide LongPathsEnabled is unset.
    /// </summary>
    internal static string ToIoPath(string path)
    {
        if (!OperatingSystem.IsWindows())
        {
            return path;
        }

        var fullPath = Path.GetFullPath(path);
        return ApplyWindowsLongPathPrefix(fullPath);
    }

    /// <summary>
    /// Pure formatting helper (testable on Linux) that adds the Win32 extended-length prefix
    /// for normalized Windows full paths approaching or exceeding MAX_PATH.
    /// </summary>
    internal static string ApplyWindowsLongPathPrefix(string normalizedWindowsFullPath)
    {
        if (string.IsNullOrEmpty(normalizedWindowsFullPath)
            || normalizedWindowsFullPath.Length < ExtendedPathThreshold)
        {
            return normalizedWindowsFullPath;
        }

        if (normalizedWindowsFullPath.StartsWith(@"\\?\", StringComparison.Ordinal)
            || normalizedWindowsFullPath.StartsWith(@"\\.\", StringComparison.Ordinal))
        {
            return normalizedWindowsFullPath;
        }

        if (normalizedWindowsFullPath.StartsWith(@"\\", StringComparison.Ordinal))
        {
            return @"\\?\UNC\" + normalizedWindowsFullPath[2..];
        }

        return @"\\?\" + normalizedWindowsFullPath;
    }

    internal static string TrimTrailingSeparators(string path)
    {
        if (string.IsNullOrEmpty(path))
        {
            return path;
        }

        var root = Path.GetPathRoot(path);
        var trimmed = path.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
        if (string.IsNullOrEmpty(trimmed) && !string.IsNullOrEmpty(root))
        {
            return root;
        }

        if (!string.IsNullOrEmpty(root)
            && trimmed.Length < root.Length
            && root.StartsWith(trimmed, StringComparison.Ordinal))
        {
            return root;
        }

        return trimmed;
    }

    internal static bool IsSameOrDescendantPath(string parentFullPath, string candidateFullPath)
    {
        var relativePath = Path.GetRelativePath(parentFullPath, candidateFullPath);
        return relativePath == "."
            || (relativePath != ".."
                && !relativePath.StartsWith($"..{Path.DirectorySeparatorChar}", StringComparison.Ordinal)
                && !relativePath.StartsWith($"..{Path.AltDirectorySeparatorChar}", StringComparison.Ordinal)
                && !Path.IsPathRooted(relativePath));
    }

    internal static string NormalizeRelativePath(string rootDirectory, string fullPath)
    {
        return Path.GetRelativePath(rootDirectory, fullPath).Replace('\\', '/');
    }
}
