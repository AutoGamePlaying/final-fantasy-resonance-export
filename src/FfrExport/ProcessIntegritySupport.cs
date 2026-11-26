using System.Globalization;
using System.Runtime.InteropServices;

namespace FfrExport;

internal enum ProcessIntegrityLevel
{
    Unknown = 0,
    Untrusted = 1,
    Low = 2,
    Medium = 3,
    High = 4,
    System = 5,
}

internal sealed record ProcessIntegrityInfo(
    ProcessIntegrityLevel Level,
    uint? RawRid,
    string Label,
    string? DiagnosticError = null)
{
    internal bool IsLowOrUntrusted => Level is ProcessIntegrityLevel.Low or ProcessIntegrityLevel.Untrusted;

    internal string DisplayText => L.Get(Level switch
    {
        ProcessIntegrityLevel.Untrusted => "Integrity_Untrusted",
        ProcessIntegrityLevel.Low => "Integrity_Low",
        ProcessIntegrityLevel.Medium => "Integrity_Medium",
        ProcessIntegrityLevel.High => "Integrity_High",
        ProcessIntegrityLevel.System => "Integrity_System",
        _ => "Integrity_Unknown",
    });

    internal string FormatForLog()
    {
        if (RawRid.HasValue)
        {
            return string.Create(CultureInfo.InvariantCulture, $"{Label} (0x{RawRid.Value:X4})");
        }

        if (!string.IsNullOrWhiteSpace(DiagnosticError))
        {
            return $"{Label} ({DiagnosticError})";
        }

        return Label;
    }
}

/// <summary>
/// Queries the current Windows process token mandatory integrity level via
/// GetTokenInformation(TokenIntegrityLevel) so Low IL launches can be diagnosed and handled cleanly.
/// </summary>
internal static class ProcessIntegritySupport
{
    internal const uint SecurityMandatoryUntrustedRid = 0x0000;
    internal const uint SecurityMandatoryLowRid = 0x1000;
    internal const uint SecurityMandatoryMediumRid = 0x2000;
    internal const uint SecurityMandatoryMediumPlusRid = 0x2100;
    internal const uint SecurityMandatoryHighRid = 0x3000;
    internal const uint SecurityMandatorySystemRid = 0x4000;

    private const uint TokenQuery = 0x0008;
    private const int TokenIntegrityLevelClass = 25;

    [StructLayout(LayoutKind.Sequential)]
    private struct SidAndAttributes
    {
        public IntPtr Sid;
        public uint Attributes;
    }

    [StructLayout(LayoutKind.Sequential)]
    private struct TokenMandatoryLabel
    {
        public SidAndAttributes Label;
    }

    internal static ProcessIntegrityInfo DetectCurrent(Func<ProcessIntegrityInfo>? detectorOverride = null)
    {
        if (detectorOverride is not null)
        {
            return detectorOverride();
        }

        if (!OperatingSystem.IsWindows())
        {
            return FromLevel(ProcessIntegrityLevel.Unknown);
        }

        return QueryWindowsIntegrityLevel();
    }

    internal static ProcessIntegrityInfo ClassifyRid(uint rid)
    {
        if (rid < SecurityMandatoryLowRid)
        {
            return FromLevel(ProcessIntegrityLevel.Untrusted, rid);
        }

        if (rid < SecurityMandatoryMediumRid)
        {
            return FromLevel(ProcessIntegrityLevel.Low, rid);
        }

        if (rid < SecurityMandatoryHighRid)
        {
            return FromLevel(ProcessIntegrityLevel.Medium, rid);
        }

        if (rid < SecurityMandatorySystemRid)
        {
            return FromLevel(ProcessIntegrityLevel.High, rid);
        }

        return FromLevel(ProcessIntegrityLevel.System, rid);
    }

    internal static ProcessIntegrityInfo FromLevel(
        ProcessIntegrityLevel level,
        uint? rawRid = null,
        string? diagnosticError = null)
    {
        return level switch
        {
            ProcessIntegrityLevel.Untrusted => new ProcessIntegrityInfo(
                level,
                rawRid ?? SecurityMandatoryUntrustedRid,
                "Untrusted",
                diagnosticError),
            ProcessIntegrityLevel.Low => new ProcessIntegrityInfo(
                level,
                rawRid ?? SecurityMandatoryLowRid,
                "Low",
                diagnosticError),
            ProcessIntegrityLevel.Medium => new ProcessIntegrityInfo(
                level,
                rawRid ?? SecurityMandatoryMediumRid,
                "Medium",
                diagnosticError),
            ProcessIntegrityLevel.High => new ProcessIntegrityInfo(
                level,
                rawRid ?? SecurityMandatoryHighRid,
                "High",
                diagnosticError),
            ProcessIntegrityLevel.System => new ProcessIntegrityInfo(
                level,
                rawRid ?? SecurityMandatorySystemRid,
                "System",
                diagnosticError),
            _ => new ProcessIntegrityInfo(
                ProcessIntegrityLevel.Unknown,
                rawRid,
                "Unknown",
                diagnosticError),
        };
    }

    private static ProcessIntegrityInfo QueryWindowsIntegrityLevel()
    {
        var tokenHandle = IntPtr.Zero;
        try
        {
            if (!OpenProcessToken(GetCurrentProcess(), TokenQuery, out tokenHandle)
                || tokenHandle == IntPtr.Zero)
            {
                var err = Marshal.GetLastWin32Error();
                return FromLevel(
                    ProcessIntegrityLevel.Unknown,
                    diagnosticError: string.Create(CultureInfo.InvariantCulture, $"OpenProcessToken failed (Win32={err})"));
            }

            _ = GetTokenInformation(tokenHandle, TokenIntegrityLevelClass, IntPtr.Zero, 0, out var requiredLength);
            if (requiredLength <= 0)
            {
                var err = Marshal.GetLastWin32Error();
                return FromLevel(
                    ProcessIntegrityLevel.Unknown,
                    diagnosticError: string.Create(CultureInfo.InvariantCulture, $"GetTokenInformation length query failed (Win32={err})"));
            }

            var buffer = Marshal.AllocHGlobal(requiredLength);
            try
            {
                if (!GetTokenInformation(tokenHandle, TokenIntegrityLevelClass, buffer, requiredLength, out _))
                {
                    var err = Marshal.GetLastWin32Error();
                    return FromLevel(
                        ProcessIntegrityLevel.Unknown,
                        diagnosticError: string.Create(CultureInfo.InvariantCulture, $"GetTokenInformation read failed (Win32={err})"));
                }

                var mandatoryLabel = Marshal.PtrToStructure<TokenMandatoryLabel>(buffer);
                var sid = mandatoryLabel.Label.Sid;
                if (sid == IntPtr.Zero)
                {
                    return FromLevel(ProcessIntegrityLevel.Unknown, diagnosticError: "TokenIntegrityLevel SID is null");
                }

                var countPtr = GetSidSubAuthorityCount(sid);
                if (countPtr == IntPtr.Zero)
                {
                    var err = Marshal.GetLastWin32Error();
                    return FromLevel(
                        ProcessIntegrityLevel.Unknown,
                        diagnosticError: string.Create(CultureInfo.InvariantCulture, $"GetSidSubAuthorityCount failed (Win32={err})"));
                }

                var subAuthorityCount = Marshal.ReadByte(countPtr);
                if (subAuthorityCount == 0)
                {
                    return FromLevel(ProcessIntegrityLevel.Unknown, diagnosticError: "SID sub-authority count is 0");
                }

                var ridPtr = GetSidSubAuthority(sid, (uint)(subAuthorityCount - 1));
                if (ridPtr == IntPtr.Zero)
                {
                    var err = Marshal.GetLastWin32Error();
                    return FromLevel(
                        ProcessIntegrityLevel.Unknown,
                        diagnosticError: string.Create(CultureInfo.InvariantCulture, $"GetSidSubAuthority failed (Win32={err})"));
                }

                var rid = unchecked((uint)Marshal.ReadInt32(ridPtr));
                return ClassifyRid(rid);
            }
            finally
            {
                Marshal.FreeHGlobal(buffer);
            }
        }
        catch (Exception exception)
        {
            return FromLevel(
                ProcessIntegrityLevel.Unknown,
                diagnosticError: $"{exception.GetType().Name}: {exception.Message}");
        }
        finally
        {
            if (tokenHandle != IntPtr.Zero)
            {
                _ = CloseHandle(tokenHandle);
            }
        }
    }

    [DllImport("kernel32.dll")]
    private static extern IntPtr GetCurrentProcess();

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool CloseHandle(IntPtr hObject);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool OpenProcessToken(
        IntPtr processHandle,
        uint desiredAccess,
        out IntPtr tokenHandle);

    [DllImport("advapi32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool GetTokenInformation(
        IntPtr tokenHandle,
        int tokenInformationClass,
        IntPtr tokenInformation,
        int tokenInformationLength,
        out int returnLength);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern IntPtr GetSidSubAuthorityCount(IntPtr pSid);

    [DllImport("advapi32.dll", SetLastError = true)]
    private static extern IntPtr GetSidSubAuthority(IntPtr pSid, uint nSubAuthority);
}
