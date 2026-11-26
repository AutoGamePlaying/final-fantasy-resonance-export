namespace FfrExport.Gui.Core;

internal sealed record FontFallbackResult<TFont>(
    TFont Font,
    bool IsOwned,
    bool UsedFallback,
    string? WarningMessage);

/// <summary>
/// Platform-independent font fallback strategy so GUI font creation never crashes when preferred
/// Windows fonts or styles (e.g. Microsoft YaHei UI / Bold / Consolas) are missing or broken.
/// </summary>
internal static class GuiFontFallback
{
    internal static FontFallbackResult<TFont> ResolveUiFont<TFont>(
        string preferredFamily,
        float size,
        bool isBold,
        Func<string, float, bool, TFont?> tryCreatePreferred,
        Func<float, bool, TFont?> tryCreateMessageBoxFont,
        Func<TFont> getFallbackDefaultFont)
    {
        string? preferredFailure = null;
        try
        {
            var preferred = tryCreatePreferred(preferredFamily, size, isBold);
            if (preferred is not null)
            {
                return new FontFallbackResult<TFont>(preferred, IsOwned: true, UsedFallback: false, WarningMessage: null);
            }
        }
        catch (Exception exception)
        {
            preferredFailure = exception.Message;
        }

        var reason = string.IsNullOrWhiteSpace(preferredFailure)
            ? $"系统未安装字体 {preferredFamily} 或不支持当前样式"
            : preferredFailure;

        try
        {
            var msgBoxFont = tryCreateMessageBoxFont(size, isBold);
            if (msgBoxFont is not null)
            {
                return new FontFallbackResult<TFont>(
                    msgBoxFont,
                    IsOwned: true,
                    UsedFallback: true,
                    WarningMessage: $"界面字体 {preferredFamily} 不可用（{reason}），已退回系统默认字体（MessageBoxFont）。");
            }
        }
        catch
        {
            // Fall through to regular MessageBoxFont or default font.
        }

        if (isBold)
        {
            try
            {
                var regularMsgBoxFont = tryCreateMessageBoxFont(size, false);
                if (regularMsgBoxFont is not null)
                {
                    return new FontFallbackResult<TFont>(
                        regularMsgBoxFont,
                        IsOwned: true,
                        UsedFallback: true,
                        WarningMessage: $"界面字体 {preferredFamily} 不可用（{reason}），已退回系统默认常规字体（MessageBoxFont）。");
                }
            }
            catch
            {
                // Fall through to default font.
            }
        }

        var defaultFont = getFallbackDefaultFont();
        return new FontFallbackResult<TFont>(
            defaultFont,
            IsOwned: false,
            UsedFallback: true,
            WarningMessage: $"界面字体 {preferredFamily} 与 MessageBoxFont 均不可用（{reason}），已退回控件默认字体。");
    }

    internal static FontFallbackResult<TFont> ResolveMonoFont<TFont>(
        string preferredFamily,
        float size,
        Func<string, float, TFont?> tryCreatePreferred,
        Func<float, TFont?> tryCreateGenericMonospace,
        Func<float, bool, TFont?> tryCreateMessageBoxFont,
        Func<TFont> getFallbackDefaultFont)
    {
        string? preferredFailure = null;
        try
        {
            var preferred = tryCreatePreferred(preferredFamily, size);
            if (preferred is not null)
            {
                return new FontFallbackResult<TFont>(preferred, IsOwned: true, UsedFallback: false, WarningMessage: null);
            }
        }
        catch (Exception exception)
        {
            preferredFailure = exception.Message;
        }

        var reason = string.IsNullOrWhiteSpace(preferredFailure)
            ? $"系统未安装字体 {preferredFamily}"
            : preferredFailure;

        try
        {
            var genericMono = tryCreateGenericMonospace(size);
            if (genericMono is not null)
            {
                return new FontFallbackResult<TFont>(
                    genericMono,
                    IsOwned: true,
                    UsedFallback: true,
                    WarningMessage: $"等宽字体 {preferredFamily} 不可用（{reason}），已退回系统等宽字体。");
            }
        }
        catch
        {
            // Fall through to MessageBoxFont.
        }

        try
        {
            var msgBoxFont = tryCreateMessageBoxFont(size, false);
            if (msgBoxFont is not null)
            {
                return new FontFallbackResult<TFont>(
                    msgBoxFont,
                    IsOwned: true,
                    UsedFallback: true,
                    WarningMessage: $"等宽字体 {preferredFamily} 不可用（{reason}），已退回系统默认字体（MessageBoxFont）。");
            }
        }
        catch
        {
            // Fall through to default font.
        }

        var defaultFont = getFallbackDefaultFont();
        return new FontFallbackResult<TFont>(
            defaultFont,
            IsOwned: false,
            UsedFallback: true,
            WarningMessage: $"等宽字体 {preferredFamily} 不可用（{reason}），已退回控件默认字体。");
    }
}
