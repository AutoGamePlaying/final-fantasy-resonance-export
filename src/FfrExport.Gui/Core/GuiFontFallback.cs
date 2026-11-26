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
            ? L.Format("GuiCore_FontMissingOrUnsupported", preferredFamily)
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
                    WarningMessage: L.Format("GuiCore_UiFontFallbackMsgBox", preferredFamily, reason));
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
                        WarningMessage: L.Format("GuiCore_UiFontFallbackMsgBoxRegular", preferredFamily, reason));
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
            WarningMessage: L.Format("GuiCore_UiFontFallbackControlDefault", preferredFamily, reason));
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
            ? L.Format("GuiCore_MonoFontNotInstalled", preferredFamily)
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
                    WarningMessage: L.Format("GuiCore_MonoFontFallbackGeneric", preferredFamily, reason));
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
                    WarningMessage: L.Format("GuiCore_MonoFontFallbackMsgBox", preferredFamily, reason));
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
            WarningMessage: L.Format("GuiCore_MonoFontFallbackControlDefault", preferredFamily, reason));
    }
}
