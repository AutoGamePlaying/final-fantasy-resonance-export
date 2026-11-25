using System.Text;

namespace FfrExport;

/// <summary>
/// Lightweight host shim supplying the game constants and Valve VDF manifest parser
/// referenced by the linked SteamLocator.cs without pulling CUE4Parse into the GUI binary.
/// </summary>
internal static class GameVersionDetector
{
    internal const int SteamAppId = 3259780;
    internal const string AppManifestFileName = "appmanifest_3259780.acf";
    internal const string RelativePaksSubpath = "FFRS/Content/Paks";
    internal const string RelativeExeSubpath = "FFRS/Binaries/Win64/FFRS-Win64-Shipping.exe";

    /// <summary>
    /// Parses a Valve KeyValues (VDF) appmanifest file and returns the top-level key/value pairs
    /// inside the root AppState block (brace depth 1).
    /// </summary>
    internal static Dictionary<string, string> ParseAppManifestVdf(string content)
    {
        var result = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
        var depth = 0;
        var index = 0;
        string? pendingKey = null;

        while (index < content.Length)
        {
            SkipWhitespaceAndComments(content, ref index);
            if (index >= content.Length)
            {
                break;
            }

            var ch = content[index];
            if (ch == '{')
            {
                depth++;
                pendingKey = null;
                index++;
                continue;
            }

            if (ch == '}')
            {
                if (depth > 0)
                {
                    depth--;
                }

                pendingKey = null;
                index++;
                continue;
            }

            var token = ReadVdfToken(content, ref index);
            if (depth == 1)
            {
                if (pendingKey is null)
                {
                    pendingKey = token;
                }
                else
                {
                    result[pendingKey] = token;
                    pendingKey = null;
                }
            }
            else
            {
                pendingKey = null;
            }
        }

        return result;
    }

    private static void SkipWhitespaceAndComments(string content, ref int index)
    {
        while (index < content.Length)
        {
            var ch = content[index];
            if (char.IsWhiteSpace(ch))
            {
                index++;
                continue;
            }

            if (ch == '/' && index + 1 < content.Length && content[index + 1] == '/')
            {
                index += 2;
                while (index < content.Length && content[index] != '\n')
                {
                    index++;
                }

                continue;
            }

            break;
        }
    }

    private static string ReadVdfToken(string content, ref int index)
    {
        if (content[index] == '"')
        {
            index++;
            var sb = new StringBuilder();
            while (index < content.Length)
            {
                var ch = content[index++];
                if (ch == '"')
                {
                    break;
                }

                if (ch == '\\' && index < content.Length)
                {
                    var escaped = content[index++];
                    sb.Append(escaped switch
                    {
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        _ => escaped,
                    });
                }
                else
                {
                    sb.Append(ch);
                }
            }

            return sb.ToString();
        }

        var start = index;
        while (index < content.Length
            && !char.IsWhiteSpace(content[index])
            && content[index] != '{'
            && content[index] != '}'
            && content[index] != '"')
        {
            index++;
        }

        return content[start..index];
    }
}
