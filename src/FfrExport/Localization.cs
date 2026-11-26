using System.Globalization;
using System.Resources;
using System.Text;
using System.Text.Json;

namespace FfrExport;

internal sealed record LanguageOption(string Code, string NativeName);

internal sealed record CliLanguageExtraction(string? CliLanguage, string[] RemainingArgs);

/// <summary>
/// Centralized localization runtime backed by embedded .resx resources (Strings.resx, Strings.zh-Hans.resx, Strings.ja.resx).
/// Language resolution priority: CLI --lang -> AGP_LANG -> saved GUI settings.json -> default "en".
/// Never follows OS locale automatically; defaults to English.
/// </summary>
internal static class Localization
{
    internal const string DefaultLanguage = "en";
    internal const string ChineseSimplifiedLanguage = "zh-Hans";
    internal const string JapaneseLanguage = "ja";
    internal const string LangEnvVar = "AGP_LANG";

    internal static readonly IReadOnlyList<string> SupportedLanguages =
    [
        DefaultLanguage,
        ChineseSimplifiedLanguage,
        JapaneseLanguage,
    ];

    internal static readonly IReadOnlyList<LanguageOption> LanguageOptions =
    [
        new(DefaultLanguage, "English"),
        new(ChineseSimplifiedLanguage, "简体中文"),
        new(JapaneseLanguage, "日本語"),
    ];

    private const string ResourceBaseName = "FfrExport.Resources.Strings";
    private static readonly ResourceManager Manager = new(ResourceBaseName, typeof(Localization).Assembly);
    private static readonly AsyncLocal<string?> ScopedLanguage = new();
    private static volatile string _processLanguage = DefaultLanguage;

    internal static string CurrentLanguage => ScopedLanguage.Value ?? _processLanguage;

    internal static bool TryNormalizeLanguageCode(string? raw, out string canonical)
    {
        canonical = DefaultLanguage;
        if (string.IsNullOrWhiteSpace(raw))
        {
            return false;
        }

        var trimmed = raw.Trim();
        if (string.Equals(trimmed, DefaultLanguage, StringComparison.OrdinalIgnoreCase))
        {
            canonical = DefaultLanguage;
            return true;
        }

        if (string.Equals(trimmed, ChineseSimplifiedLanguage, StringComparison.OrdinalIgnoreCase))
        {
            canonical = ChineseSimplifiedLanguage;
            return true;
        }

        if (string.Equals(trimmed, JapaneseLanguage, StringComparison.OrdinalIgnoreCase))
        {
            canonical = JapaneseLanguage;
            return true;
        }

        return false;
    }

    /// <summary>
    /// Resolves the active language in strict priority order:
    /// 1. Explicit CLI --lang value (must already be validated or valid);
    /// 2. AGP_LANG environment variable (or envLanguageOverride);
    /// 3. Saved GUI settings.json language;
    /// 4. Default "en" (never auto-switches based on OS locale).
    /// </summary>
    internal static string ResolveLanguage(
        string? cliLanguage,
        string? envLanguageOverride = null,
        string? savedLanguage = null)
    {
        if (!string.IsNullOrWhiteSpace(cliLanguage)
            && TryNormalizeLanguageCode(cliLanguage, out var normalizedCli))
        {
            return normalizedCli;
        }

        var envRaw = envLanguageOverride is not null
            ? envLanguageOverride
            : SafeGetEnvironmentVariable(LangEnvVar);
        if (!string.IsNullOrWhiteSpace(envRaw)
            && TryNormalizeLanguageCode(envRaw, out var normalizedEnv))
        {
            return normalizedEnv;
        }

        if (!string.IsNullOrWhiteSpace(savedLanguage)
            && TryNormalizeLanguageCode(savedLanguage, out var normalizedSaved))
        {
            return normalizedSaved;
        }

        return DefaultLanguage;
    }

    internal static void SetLanguage(string languageCode)
    {
        if (!TryNormalizeLanguageCode(languageCode, out var canonical))
        {
            throw new ArgumentException(Format("Common_UnsupportedLanguage", languageCode));
        }

        _processLanguage = canonical;
        var culture = GetCultureForLanguage(canonical);
        CultureInfo.CurrentUICulture = culture;
        CultureInfo.DefaultThreadCurrentUICulture = culture;
    }

    /// <summary>
    /// Temporarily overrides the active language for the current async execution flow (ideal for parallel tests).
    /// </summary>
    internal static IDisposable UseLanguageScope(string languageCode)
    {
        if (!TryNormalizeLanguageCode(languageCode, out var canonical))
        {
            throw new ArgumentException(Format("Common_UnsupportedLanguage", languageCode));
        }

        var previous = ScopedLanguage.Value;
        ScopedLanguage.Value = canonical;
        return new LanguageScope(previous);
    }

    internal static string Get(string key)
    {
        return GetForLanguage(key, CurrentLanguage);
    }

    internal static string Format(string key, params object?[] args)
    {
        return string.Format(CultureInfo.InvariantCulture, Get(key), args);
    }

    internal static string GetForLanguage(string key, string languageCode)
    {
        ArgumentException.ThrowIfNullOrEmpty(key);
        var culture = GetCultureForLanguage(languageCode);
        var value = Manager.GetString(key, culture);
        if (value is not null)
        {
            return value;
        }

        return Manager.GetString(key, CultureInfo.InvariantCulture) ?? key;
    }

    /// <summary>
    /// Extracts global --lang &lt;code&gt; / --lang=&lt;code&gt; arguments from a CLI argument vector.
    /// When stopAfterLocresSubcommand is true, any --lang appearing after the 'locres' subcommand
    /// is left untouched because 'locres --lang' selects game localization languages to export.
    /// </summary>
    internal static CliLanguageExtraction ExtractCliLanguageArgs(
        string[] args,
        bool stopAfterLocresSubcommand = true)
    {
        ArgumentNullException.ThrowIfNull(args);

        string? cliLanguage = null;
        var remaining = new List<string>(args.Length);
        var sawSubcommand = false;
        var inLocresSubcommand = false;

        for (var i = 0; i < args.Length; i++)
        {
            var token = args[i];
            if (inLocresSubcommand)
            {
                remaining.Add(token);
                continue;
            }

            if (string.Equals(token, "--lang", StringComparison.Ordinal))
            {
                if (i + 1 >= args.Length)
                {
                    throw new ArgumentException(Format("Common_OptionRequiresValue", "--lang"));
                }

                cliLanguage = args[++i];
                continue;
            }

            if (token.StartsWith("--lang=", StringComparison.Ordinal))
            {
                cliLanguage = token["--lang=".Length..];
                continue;
            }

            if (!sawSubcommand && !token.StartsWith("-", StringComparison.Ordinal))
            {
                sawSubcommand = true;
                if (stopAfterLocresSubcommand && string.Equals(token, "locres", StringComparison.Ordinal))
                {
                    inLocresSubcommand = true;
                }
            }

            remaining.Add(token);
        }

        return new CliLanguageExtraction(cliLanguage, [.. remaining]);
    }

    /// <summary>
    /// Initializes process language from CLI arguments, AGP_LANG, and saved settings.json (read-only best-effort).
    /// Returns the remaining arguments with global --lang removed.
    /// </summary>
    internal static string[] InitializeFromCli(
        string[] args,
        bool stopAfterLocresSubcommand = true,
        string? envLanguageOverride = null,
        string? agpHomeOverride = null)
    {
        // Pre-apply fallback language (env -> saved -> en) so errors during --lang extraction are localized.
        var savedLanguage = TryReadSavedLanguageFromSettings(agpHomeOverride);
        var fallbackLanguage = ResolveLanguage(null, envLanguageOverride, savedLanguage);
        SetLanguage(fallbackLanguage);

        var extracted = ExtractCliLanguageArgs(args, stopAfterLocresSubcommand);
        if (extracted.CliLanguage is not null)
        {
            if (!TryNormalizeLanguageCode(extracted.CliLanguage, out var canonicalCli))
            {
                throw new ArgumentException(Format("Common_UnsupportedLanguage", extracted.CliLanguage));
            }

            SetLanguage(canonicalCli);
        }

        return extracted.RemainingArgs;
    }

    /// <summary>
    /// Best-effort, strictly read-only lookup of the saved "language" field in settings.json.
    /// Never creates directories, writes probe files, or throws.
    /// </summary>
    internal static string? TryReadSavedLanguageFromSettings(
        string? agpHomeOverride = null,
        bool? isWindowsOverride = null,
        string? executableDirectoryOverride = null)
    {
        try
        {
            foreach (var agpRoot in EnumerateCandidateAgpRootsReadOnly(
                agpHomeOverride,
                isWindowsOverride,
                executableDirectoryOverride))
            {
                var settingsPath = AgpPaths.GetSettingsFilePathForAgpRoot(agpRoot);
                var ioPath = PathSupport.ToIoPath(settingsPath);
                if (!File.Exists(ioPath))
                {
                    continue;
                }

                var json = File.ReadAllText(ioPath, new UTF8Encoding(false));
                using var doc = JsonDocument.Parse(json);
                if (doc.RootElement.ValueKind != JsonValueKind.Object)
                {
                    return null;
                }

                if (doc.RootElement.TryGetProperty("language", out var langProp)
                    && langProp.ValueKind == JsonValueKind.String
                    && TryNormalizeLanguageCode(langProp.GetString(), out var canonical))
                {
                    return canonical;
                }

                return null;
            }
        }
        catch
        {
            // Ignore unreadable or malformed settings when probing saved language.
        }

        return null;
    }

    private static IEnumerable<string> EnumerateCandidateAgpRootsReadOnly(
        string? agpHomeOverride,
        bool? isWindowsOverride,
        string? executableDirectoryOverride)
    {
        var rawEnv = agpHomeOverride is not null
            ? agpHomeOverride
            : SafeGetEnvironmentVariable(AgpPaths.AgpHomeEnvVar);
        if (!string.IsNullOrWhiteSpace(rawEnv))
        {
            if (AgpPaths.TryNormalizePath(rawEnv, out var normalizedEnv))
            {
                yield return normalizedEnv;
            }

            yield break;
        }

        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var candidates = new List<string>();
        var seen = new HashSet<string>(isWindows ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal);

        void TryAdd(string? raw)
        {
            if (!string.IsNullOrWhiteSpace(raw)
                && AgpPaths.TryNormalizePath(raw, out var normalized)
                && seen.Add(normalized))
            {
                candidates.Add(normalized);
            }
        }

        var userProfile = SafeGetFolderPath(Environment.SpecialFolder.UserProfile)
            ?? SafeGetEnvironmentVariable(isWindows ? "USERPROFILE" : "HOME");

        if (isWindows)
        {
            var localAppData = SafeGetFolderPath(Environment.SpecialFolder.LocalApplicationData)
                ?? SafeGetEnvironmentVariable("LOCALAPPDATA");
            if (!string.IsNullOrWhiteSpace(localAppData))
            {
                TryAdd(Path.Combine(localAppData.Trim(), AgpPaths.AgpFolderName));
            }

            if (!string.IsNullOrWhiteSpace(userProfile))
            {
                TryAdd(Path.Combine(userProfile.Trim(), "AppData", "LocalLow", AgpPaths.AgpFolderName));
            }
        }
        else
        {
            var xdgDataHome = SafeGetEnvironmentVariable("XDG_DATA_HOME");
            if (!string.IsNullOrWhiteSpace(xdgDataHome))
            {
                TryAdd(Path.Combine(xdgDataHome.Trim(), AgpPaths.AgpFolderName));
            }

            if (!string.IsNullOrWhiteSpace(userProfile))
            {
                TryAdd(Path.Combine(userProfile.Trim(), ".local", "share", AgpPaths.AgpFolderName));
            }
        }

        var exeDir = !string.IsNullOrWhiteSpace(executableDirectoryOverride)
            ? executableDirectoryOverride.Trim()
            : AppContext.BaseDirectory;
        if (!string.IsNullOrWhiteSpace(exeDir))
        {
            TryAdd(Path.Combine(exeDir, AgpPaths.PortableAgpFolderName));
        }

        if (!string.IsNullOrWhiteSpace(userProfile))
        {
            TryAdd(Path.Combine(userProfile.Trim(), AgpPaths.AgpFolderName));
        }

        var savedLocation = AgpPaths.TryReadLocationFile(exeDir);
        if (!string.IsNullOrWhiteSpace(savedLocation))
        {
            TryAdd(savedLocation);
        }

        foreach (var candidate in candidates)
        {
            yield return candidate;
        }
    }

    private static CultureInfo GetCultureForLanguage(string languageCode)
    {
        if (string.Equals(languageCode, ChineseSimplifiedLanguage, StringComparison.OrdinalIgnoreCase))
        {
            return CultureInfo.GetCultureInfo(ChineseSimplifiedLanguage);
        }

        if (string.Equals(languageCode, JapaneseLanguage, StringComparison.OrdinalIgnoreCase))
        {
            return CultureInfo.GetCultureInfo(JapaneseLanguage);
        }

        // InvariantCulture maps directly to Strings.resx (neutral English) without ever probing OS locale.
        return CultureInfo.InvariantCulture;
    }

    private static string? SafeGetEnvironmentVariable(string name)
    {
        try
        {
            return Environment.GetEnvironmentVariable(name);
        }
        catch
        {
            return null;
        }
    }

    private static string? SafeGetFolderPath(Environment.SpecialFolder folder)
    {
        try
        {
            return Environment.GetFolderPath(folder);
        }
        catch
        {
            return null;
        }
    }

    private sealed class LanguageScope(string? previous) : IDisposable
    {
        public void Dispose()
        {
            ScopedLanguage.Value = previous;
        }
    }
}

/// <summary>
/// Concise shorthand facade for localized string lookups.
/// </summary>
internal static class L
{
    internal static string Get(string key) => Localization.Get(key);

    internal static string Format(string key, params object?[] args) => Localization.Format(key, args);
}
