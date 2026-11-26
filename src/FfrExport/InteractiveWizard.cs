using System.Diagnostics;
using System.Globalization;
using System.Text;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;

namespace FfrExport;

internal delegate Task<int> BuildDbRunner(string[] buildDbArgs);

/// <summary>
/// Interactive Chinese console wizard launched when ffr-export is started without arguments
/// (for example, by double-clicking ffr-export.exe in Windows Explorer).
/// Delegates the actual database build to the exact same build-db entry point.
/// </summary>
internal static class InteractiveWizard
{
    private const string BuildMetadataFileName = "ffr.sqlite.build.json";
    private static readonly UTF8Encoding Utf8WithoutBom = new(false);

    /// <summary>
    /// Trims surrounding whitespace and matched single or double quotes from user-pasted paths
    /// (such as Windows Explorer's "Copy as path" output).
    /// </summary>
    internal static string CleanUserPathInput(string? rawInput)
    {
        if (string.IsNullOrWhiteSpace(rawInput))
        {
            return string.Empty;
        }

        var cleaned = rawInput.Trim();
        while (cleaned.Length >= 2
            && ((cleaned[0] == '"' && cleaned[^1] == '"')
                || (cleaned[0] == '\'' && cleaned[^1] == '\'')))
        {
            cleaned = cleaned[1..^1].Trim();
        }

        return cleaned;
    }

    /// <summary>
    /// Computes the default output directory under the unified agp layout:
    /// %LOCALAPPDATA%\agp\ffr-export\output\&lt;gameCode&gt;-&lt;steamBuildId&gt;.
    /// </summary>
    internal static string BuildDefaultOutputDirectory(
        string gameCode,
        string? steamBuildId,
        string? agpHomeOverride = null)
    {
        return AgpPaths.GetDefaultOutputDirectory(
            gameCode,
            steamBuildId,
            agpHomeEnvOverride: agpHomeOverride);
    }

    /// <summary>
    /// Auto-discovers the newest .usmap file across %LOCALAPPDATA%\agp\ffr-export\usmap\,
    /// FFRS\Binaries\Win64\ue4ss\, and FFRS\Binaries\Win64\.
    /// </summary>
    internal static (string FullPath, string SourceLabel)? TryDiscoverLatestUsmap(
        string gameDirectory,
        string? agpHomeOverride = null)
    {
        var agpUsmapDir = AgpPaths.GetUsmapDirectory(agpHomeEnvOverride: agpHomeOverride);
        var ue4ssDir = Ue4ssRuntimeSupport.GetUe4ssDirectory(gameDirectory);
        var win64Dir = Ue4ssRuntimeSupport.GetWin64Directory(gameDirectory);

        var primaryCandidates = new List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)>();
        CollectUsmapCandidates(agpUsmapDir, priority: 0, "agp/usmap", primaryCandidates);
        CollectUsmapCandidates(ue4ssDir, priority: 1, "ue4ss", primaryCandidates);
        if (primaryCandidates.Count > 0)
        {
            var chosen = primaryCandidates
                .OrderByDescending(item => item.LastWriteUtc)
                .ThenBy(item => item.Priority)
                .ThenByDescending(item => Path.GetFileName(item.FullPath), StringComparer.OrdinalIgnoreCase)
                .First();
            return (chosen.FullPath, chosen.SourceLabel);
        }

        var fallbackCandidates = new List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)>();
        CollectUsmapCandidates(win64Dir, priority: 2, "Win64", fallbackCandidates);
        if (fallbackCandidates.Count > 0)
        {
            var chosen = fallbackCandidates
                .OrderByDescending(item => item.LastWriteUtc)
                .ThenByDescending(item => Path.GetFileName(item.FullPath), StringComparer.OrdinalIgnoreCase)
                .First();
            return (chosen.FullPath, chosen.SourceLabel);
        }

        return null;
    }

    internal static async Task<int> RunAsync(
        TextReader input,
        TextWriter output,
        TextWriter error,
        BuildDbRunner buildDbRunner,
        Func<IReadOnlyList<DiscoveredSteamGame>> discoverGames,
        Func<string> resolveFfrDb,
        string executableDirectory,
        bool? isWindowsOverride = null,
        string? agpHomeOverride = null)
    {
        var isWindows = isWindowsOverride ?? OperatingSystem.IsWindows();
        var exeDir = string.IsNullOrWhiteSpace(executableDirectory)
            ? Directory.GetCurrentDirectory()
            : PathSupport.TrimTrailingSeparators(Path.GetFullPath(executableDirectory));

        var effectiveAgpHome = agpHomeOverride;
        if (string.IsNullOrWhiteSpace(effectiveAgpHome))
        {
            var agpResolution = AgpPaths.ResolveWritableAgpRootOrThrow(
                noticeOutput: output,
                isWindowsOverride: isWindows,
                executableDirectoryOverride: exeDir);
            effectiveAgpHome = agpResolution.AgpRoot;
        }

        output.WriteLine("============================================================");
        output.WriteLine("《FINAL FANTASY RESONANCE》资源导出与建库向导");
        output.WriteLine("============================================================");
        output.WriteLine("说明：本工具仅以只读方式读取游戏文件，不会修改游戏安装目录。");
        output.WriteLine("提示：粘贴路径时可直接保留两端的引号，也支持包含空格的路径。");

        // Pre-check ffr-db binary so double-click users get an immediate Chinese message if it is missing.
        string resolvedFfrDb;
        try
        {
            resolvedFfrDb = resolveFfrDb();
        }
        catch (Exception exception)
        {
            output.WriteLine();
            PrintMissingFfrDbError(error, exception, exeDir, isWindows);
            WaitForExit(input, output);
            return 2;
        }

        // Step 1: Locate Steam games or prompt for manual game directory.
        output.WriteLine();
        output.WriteLine("【步骤 1/4】定位游戏目录");
        var gameSelection = SelectGameDirectory(input, output, discoverGames);
        if (gameSelection is null)
        {
            output.WriteLine("输入流已结束，向导已取消。");
            return 2;
        }

        // Step 2: Auto-discover or prompt for .usmap (required) and .jmap (optional, multiple).
        output.WriteLine();
        output.WriteLine("【步骤 2/4】指定映射文件（.usmap / .jmap）");
        output.WriteLine("说明：.usmap 为必填项；可用「ffr-export fetch-usmap --game <目录>」一键获取，或用 UE4SS 在游戏运行时转储。");
        var discoveredUsmap = TryDiscoverLatestUsmap(gameSelection.GameDirectory, effectiveAgpHome);
        var usmapPath = PromptUsmapFile(input, output, discoveredUsmap);
        if (usmapPath is null)
        {
            output.WriteLine("输入流已结束，向导已取消。");
            return 2;
        }

        output.WriteLine();
        output.WriteLine("可选：.jmap 反射转储文件（可添加多个，不提供请直接回车跳过）。");
        var jmapPaths = PromptOptionalFiles(input, output, isWindows);
        if (jmapPaths is null)
        {
            output.WriteLine("输入流已结束，向导已取消。");
            return 2;
        }

        // Step 3: Output directory (defaults to <agp>/ffr-export/output/<gameCode>-<buildId>).
        output.WriteLine();
        output.WriteLine("【步骤 3/4】选择输出目录");
        var defaultOutputDir = BuildDefaultOutputDirectory(
            gameSelection.GameCode,
            gameSelection.SteamBuildId,
            effectiveAgpHome);
        var outputSelection = PromptOutputDirectory(input, output, gameSelection.GameDirectory, defaultOutputDir);
        if (outputSelection is null)
        {
            output.WriteLine("输入流已结束，向导已取消。");
            return 2;
        }

        // Step 4: Summary, confirmation, and invoke the exact build-db entry point.
        output.WriteLine();
        output.WriteLine("【步骤 4/4】确认信息并开始建库");
        output.WriteLine($"  游戏名称：{gameSelection.DisplayName}");
        output.WriteLine($"  游戏目录：{gameSelection.GameDirectory}");
        output.WriteLine($"  Steam 构建号：{gameSelection.SteamBuildId ?? "未检测到"}");
        output.WriteLine($"  .usmap 文件：{usmapPath}");
        if (jmapPaths.Count == 0)
        {
            output.WriteLine("  .jmap 文件：无（已跳过）");
        }
        else
        {
            output.WriteLine($"  .jmap 文件：共 {jmapPaths.Count} 个");
            for (var i = 0; i < jmapPaths.Count; i++)
            {
                output.WriteLine($"    [{i + 1}] {jmapPaths[i]}");
            }
        }

        output.WriteLine(outputSelection.Force
            ? $"  输出目录：{outputSelection.OutputDirectory}（将清空并重新建库）"
            : $"  输出目录：{outputSelection.OutputDirectory}");

        var confirmed = PromptConfirmation(input, output);
        if (confirmed is null)
        {
            output.WriteLine("输入流已结束，向导已取消。");
            return 2;
        }

        if (!confirmed.Value)
        {
            output.WriteLine("已取消建库，未修改任何文件。");
            WaitForExit(input, output);
            return 0;
        }

        var buildDbArgs = BuildCliArguments(
            gameSelection.GameDirectory,
            usmapPath,
            jmapPaths,
            outputSelection.OutputDirectory,
            gameSelection.SteamBuildId,
            resolvedFfrDb,
            outputSelection.Force);

        output.WriteLine();
        output.WriteLine("开始执行建库流程（与命令行 build-db 相同），请耐心等待……");
        output.Flush();

        var stopwatch = Stopwatch.StartNew();
        try
        {
            var exitCode = await buildDbRunner(buildDbArgs);
            stopwatch.Stop();

            if (exitCode is 0 or 1)
            {
                PrintCompletionSummary(output, outputSelection.OutputDirectory, exitCode, stopwatch.Elapsed);
            }
            else
            {
                error.WriteLine();
                error.WriteLine($"错误：建库流程未成功完成（退出码：{exitCode}）。");
                PrintFailureAdvice(error, null, exeDir, isWindows);
            }

            WaitForExit(input, output);
            return exitCode;
        }
        catch (Exception exception)
        {
            stopwatch.Stop();
            error.WriteLine();
            error.WriteLine($"错误：建库失败：{exception.Message}");
            PrintFailureAdvice(error, exception, exeDir, isWindows);
            WaitForExit(input, output);
            return 2;
        }
    }

    internal static string[] BuildCliArguments(
        string gameDirectory,
        string usmapPath,
        IReadOnlyList<string> jmapPaths,
        string outputDirectory,
        string? steamBuildId,
        string? resolvedFfrDbPath,
        bool force)
    {
        var args = new List<string>
        {
            "--game",
            gameDirectory,
            "--usmap",
            usmapPath,
        };

        foreach (var jmap in jmapPaths)
        {
            args.Add("--jmap");
            args.Add(jmap);
        }

        args.Add("--out");
        args.Add(outputDirectory);

        if (!string.IsNullOrWhiteSpace(steamBuildId))
        {
            args.Add("--steam-buildid");
            args.Add(steamBuildId.Trim());
        }

        if (!string.IsNullOrWhiteSpace(resolvedFfrDbPath))
        {
            args.Add("--ffr-db");
            args.Add(resolvedFfrDbPath);
        }

        if (force)
        {
            args.Add("--force");
        }

        return [.. args];
    }

    private sealed record GameSelection(
        string GameCode,
        string DisplayName,
        string GameDirectory,
        string? SteamBuildId);

    private sealed record OutputSelection(
        string OutputDirectory,
        bool Force);

    private static GameSelection? SelectGameDirectory(
        TextReader input,
        TextWriter output,
        Func<IReadOnlyList<DiscoveredSteamGame>> discoverGames)
    {
        IReadOnlyList<DiscoveredSteamGame> discovered;
        try
        {
            discovered = discoverGames();
        }
        catch (Exception exception)
        {
            output.WriteLine($"提示：自动扫描 Steam 游戏库时出错（{exception.Message}），改为手动输入目录。");
            discovered = [];
        }

        if (discovered.Count > 0)
        {
            output.WriteLine("已在 Steam 游戏库中找到以下游戏：");
            for (var i = 0; i < discovered.Count; i++)
            {
                var game = discovered[i];
                output.WriteLine($"  [{i + 1}] {game.DisplayName}");
                output.WriteLine($"      目录：{game.GameDirectory}");
                output.WriteLine($"      Steam 构建号：{game.SteamBuildId ?? "未知"}");
            }

            output.WriteLine("  [0] 手动粘贴其他游戏目录");

            while (true)
            {
                output.Write($"请输入编号（1-{discovered.Count}，直接回车默认选 1，输入 0 手动输入）：");
                output.Flush();
                var raw = input.ReadLine();
                if (raw is null)
                {
                    return null;
                }

                var trimmed = raw.Trim();
                if (string.IsNullOrEmpty(trimmed))
                {
                    var chosen = discovered[0];
                    return new GameSelection(chosen.Spec.Code, chosen.DisplayName, chosen.GameDirectory, chosen.SteamBuildId);
                }

                if (int.TryParse(trimmed, NumberStyles.Integer, CultureInfo.InvariantCulture, out var number))
                {
                    if (number == 0)
                    {
                        break;
                    }

                    if (number >= 1 && number <= discovered.Count)
                    {
                        var chosen = discovered[number - 1];
                        return new GameSelection(chosen.Spec.Code, chosen.DisplayName, chosen.GameDirectory, chosen.SteamBuildId);
                    }
                }

                output.WriteLine($"输入无效，请输入 0 到 {discovered.Count} 之间的整数。");
            }
        }
        else
        {
            output.WriteLine(
                $"未在 Steam 游戏库中自动找到《{SteamLocator.ResonanceGame.DefaultDisplayName}》" +
                $"（AppID: {SteamLocator.ResonanceGame.AppId}）。");
        }

        while (true)
        {
            output.Write($"请粘贴游戏安装根目录（须包含 {GameVersionDetector.RelativePaksSubpath}）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                output.WriteLine("游戏目录不能为空，请重新输入。");
                continue;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullDir, out var pathError))
            {
                output.WriteLine($"路径格式无效：{pathError}，请重新输入。");
                continue;
            }

            if (!Directory.Exists(PathSupport.ToIoPath(fullDir)))
            {
                output.WriteLine($"找不到目录：{cleaned}，请检查路径后重新输入。");
                continue;
            }

            var paksDir = SteamLocator.CombineRelativeSubpath(fullDir, GameVersionDetector.RelativePaksSubpath);
            if (!Directory.Exists(PathSupport.ToIoPath(paksDir)))
            {
                output.WriteLine(
                    $"该目录下缺少 {GameVersionDetector.RelativePaksSubpath}：{fullDir}。" +
                    "请确认粘贴的是游戏安装根目录后重新输入。");
                continue;
            }

            var steamInfo = GameVersionDetector.ReadSteamBuildInfo(fullDir, null, TextWriter.Null);
            var displayName = string.IsNullOrWhiteSpace(steamInfo.Name)
                ? SteamLocator.ResonanceGame.DefaultDisplayName
                : steamInfo.Name;
            return new GameSelection(
                SteamLocator.ResonanceGame.Code,
                displayName,
                fullDir,
                steamInfo.BuildId);
        }
    }

    private static void CollectUsmapCandidates(
        string directoryPath,
        int priority,
        string sourceLabel,
        List<(string FullPath, DateTime LastWriteUtc, int Priority, string SourceLabel)> target)
    {
        try
        {
            var ioDir = PathSupport.ToIoPath(directoryPath);
            if (!Directory.Exists(ioDir))
            {
                return;
            }

            foreach (var rawFile in Directory.EnumerateFiles(ioDir, "*", SearchOption.TopDirectoryOnly))
            {
                if (!string.Equals(Path.GetExtension(rawFile), ".usmap", StringComparison.OrdinalIgnoreCase))
                {
                    continue;
                }

                var fullPath = Path.GetFullPath(rawFile);
                var writeUtc = DateTime.MinValue;
                try
                {
                    writeUtc = File.GetLastWriteTimeUtc(PathSupport.ToIoPath(fullPath));
                }
                catch
                {
                    // Treat unreadable mtime as oldest.
                }

                target.Add((fullPath, writeUtc, priority, sourceLabel));
            }
        }
        catch
        {
            // Ignore unreadable directories during best-effort wizard auto-discovery.
        }
    }

    private static string? PromptUsmapFile(
        TextReader input,
        TextWriter output,
        (string FullPath, string SourceLabel)? discoveredUsmap)
    {
        if (discoveredUsmap is null)
        {
            return PromptRequiredFile(input, output, ".usmap 文件路径", ".usmap");
        }

        output.WriteLine($"已自动找到 .usmap（{discoveredUsmap.Value.SourceLabel}）：{discoveredUsmap.Value.FullPath}");
        while (true)
        {
            output.Write(".usmap 文件路径（直接回车使用自动找到的文件，或粘贴其他 .usmap 路径）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                return discoveredUsmap.Value.FullPath;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine($"路径格式无效：{pathError}，请重新输入。");
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine($"输入的是目录而不是文件：{cleaned}，请粘贴具体的 .usmap 文件路径。");
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine($"找不到文件：{cleaned}，请检查路径后重新输入。");
                continue;
            }

            return fullPath;
        }
    }

    private static string? PromptRequiredFile(
        TextReader input,
        TextWriter output,
        string promptLabel,
        string expectedExtension)
    {
        while (true)
        {
            output.Write($"请粘贴 {promptLabel}：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                output.WriteLine($"{promptLabel} 为必填项，不能为空，请重新输入。");
                continue;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine($"路径格式无效：{pathError}，请重新输入。");
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine($"输入的是目录而不是文件：{cleaned}，请粘贴具体的 {expectedExtension} 文件路径。");
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine($"找不到文件：{cleaned}，请检查路径后重新输入。");
                continue;
            }

            return fullPath;
        }
    }

    private static List<string>? PromptOptionalFiles(
        TextReader input,
        TextWriter output,
        bool isWindows)
    {
        var files = new List<string>();
        var comparer = isWindows ? StringComparer.OrdinalIgnoreCase : StringComparer.Ordinal;
        var seen = new HashSet<string>(comparer);

        while (true)
        {
            var prompt = files.Count == 0
                ? "第 1 个 .jmap 文件路径（直接回车跳过）："
                : $"第 {files.Count + 1} 个 .jmap 文件路径（继续粘贴或直接回车结束）：";
            output.Write(prompt);
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            if (string.IsNullOrEmpty(cleaned))
            {
                if (files.Count == 0)
                {
                    output.WriteLine("已跳过 .jmap 文件。");
                }
                else
                {
                    output.WriteLine($"共添加 {files.Count} 个 .jmap 文件。");
                }

                return files;
            }

            if (!TryNormalizeFullPath(cleaned, out var fullPath, out var pathError))
            {
                output.WriteLine($"路径格式无效：{pathError}，请重新输入或直接回车跳过。");
                continue;
            }

            var ioPath = PathSupport.ToIoPath(fullPath);
            if (Directory.Exists(ioPath))
            {
                output.WriteLine($"输入的是目录而不是文件：{cleaned}，请粘贴具体的 .jmap 文件路径或直接回车跳过。");
                continue;
            }

            if (!File.Exists(ioPath))
            {
                output.WriteLine($"找不到文件：{cleaned}，请检查路径后重新输入或直接回车跳过。");
                continue;
            }

            if (!seen.Add(fullPath))
            {
                output.WriteLine($"提示：该 .jmap 文件已添加，已跳过重复项：{fullPath}");
                continue;
            }

            files.Add(fullPath);
            output.WriteLine($"  已添加：{fullPath}");
        }
    }

    private static OutputSelection? PromptOutputDirectory(
        TextReader input,
        TextWriter output,
        string gameDirectory,
        string defaultOutputDir)
    {
        output.WriteLine($"默认输出目录：{defaultOutputDir}");

        while (true)
        {
            output.Write("输出目录（直接回车使用默认目录，或粘贴其他目录路径）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var cleaned = CleanUserPathInput(raw);
            string candidateDir;
            if (string.IsNullOrEmpty(cleaned))
            {
                candidateDir = defaultOutputDir;
            }
            else if (!TryNormalizeFullPath(cleaned, out candidateDir, out var pathError))
            {
                output.WriteLine($"路径格式无效：{pathError}，请重新输入。");
                continue;
            }

            if (PathSupport.IsSameOrDescendantPath(gameDirectory, candidateDir)
                || PathSupport.IsSameOrDescendantPath(candidateDir, gameDirectory))
            {
                output.WriteLine($"输出目录不能与游戏目录重叠：{candidateDir}，请换一个目录。");
                continue;
            }

            var ioCandidate = PathSupport.ToIoPath(candidateDir);
            if (File.Exists(ioCandidate))
            {
                output.WriteLine($"目标路径是一个已存在的文件而非目录：{candidateDir}，请换一个路径。");
                continue;
            }

            if (Directory.Exists(ioCandidate) && Directory.EnumerateFileSystemEntries(ioCandidate).Any())
            {
                if (!HasOwnBuildDbMetadata(ioCandidate))
                {
                    output.WriteLine(
                        $"目标目录非空且不是 ffr-export build-db 的输出目录：{candidateDir}。" +
                        "为避免误删文件，请换一个空目录。");
                    continue;
                }

                output.WriteLine($"提示：目标目录已存在之前的建库结果：{candidateDir}");
                output.Write("是否清空该目录并重新建库？（输入 y 覆盖重建，直接回车或输入 n 重新选择目录）：");
                output.Flush();
                var overwriteRaw = input.ReadLine();
                if (overwriteRaw is null)
                {
                    return null;
                }

                var overwriteTrimmed = overwriteRaw.Trim();
                if (IsYes(overwriteTrimmed))
                {
                    return new OutputSelection(candidateDir, Force: true);
                }

                continue;
            }

            return new OutputSelection(candidateDir, Force: false);
        }
    }

    private static bool? PromptConfirmation(TextReader input, TextWriter output)
    {
        while (true)
        {
            output.Write("确认开始建库吗？（直接回车开始，输入 n 取消）：");
            output.Flush();
            var raw = input.ReadLine();
            if (raw is null)
            {
                return null;
            }

            var trimmed = raw.Trim();
            if (string.IsNullOrEmpty(trimmed) || IsYes(trimmed))
            {
                return true;
            }

            if (IsNo(trimmed))
            {
                return false;
            }

            output.WriteLine("输入无效，请直接回车开始，或输入 n 取消。");
        }
    }

    private static void PrintCompletionSummary(
        TextWriter output,
        string outputDirectory,
        int exitCode,
        TimeSpan wallElapsed)
    {
        var sqlitePath = Path.Combine(outputDirectory, "ffr.sqlite");
        var metadataPath = Path.Combine(outputDirectory, BuildMetadataFileName);

        output.WriteLine();
        output.WriteLine("============================================================");
        output.WriteLine("建库结果汇总");
        output.WriteLine("============================================================");
        output.WriteLine($"数据库路径：{sqlitePath}");

        var ioMetaPath = PathSupport.ToIoPath(metadataPath);
        if (File.Exists(ioMetaPath))
        {
            try
            {
                var report = JObject.Parse(File.ReadAllText(ioMetaPath, Utf8WithoutBom));
                var db = report["database"] as JObject;
                var comparison = report["known_build_check"] as JObject;
                var timings = report["timings_ms"] as JObject;

                var fileSha = db?["file_sha256"]?.Value<string>() ?? "无";
                var contentSha = db?["content_sha256"]?.Value<string>() ?? "无";
                var cmpMessage = comparison?["message"]?.Value<string>() ?? "无";
                var cmpSource = comparison?["source"]?.Value<string>();
                var cmpStatus = comparison?["status"]?.Value<string>();
                var matchedDate = comparison?["matched_registered_at"]?.Value<string>();
                var fileShaOnly = comparison?["file_sha256_mismatch_only"]?.Value<bool>() ?? false;
                var tableDiffs = comparison?["table_diffs"] as JArray;

                output.WriteLine($"文件哈希（file_sha256）：{fileSha}");
                output.WriteLine($"内容哈希（content_sha256）：{contentSha}");
                output.WriteLine(string.IsNullOrWhiteSpace(cmpSource)
                    ? $"known-builds.json 比对结论：{cmpMessage}"
                    : $"known-builds.json 比对结论：{cmpMessage}（来源：{cmpSource}）");

                if (string.Equals(cmpStatus, "matched", StringComparison.Ordinal)
                    && !string.IsNullOrWhiteSpace(matchedDate))
                {
                    output.WriteLine($"  匹配登记日期：{matchedDate}");
                }
                else if (string.Equals(cmpStatus, "hash_mismatch", StringComparison.Ordinal))
                {
                    if (fileShaOnly)
                    {
                        output.WriteLine("  说明：所有表内容一致（content_sha256 相同），仅文件哈希不同。");
                    }
                    else if (tableDiffs is not null && tableDiffs.Count > 0)
                    {
                        output.WriteLine($"  差异表数：{tableDiffs.Count} 张（详情见上方输出与构建报告）。");
                    }
                }

                if (timings is not null)
                {
                    var totalMs = timings["total"]?.Value<long>();
                    var exportMs = (timings["export_datatable"]?.Value<long>() ?? 0)
                        + (timings["export_locres"]?.Value<long>() ?? 0)
                        + (timings["export_inventory"]?.Value<long>() ?? 0)
                        + (timings["export_assets"]?.Value<long>() ?? 0)
                        + (timings["export_maps"]?.Value<long>() ?? 0);
                    var dbBuildMs = timings["ffr_db_build"]?.Value<long>() ?? 0;
                    var dbHashMs = timings["ffr_db_hash"]?.Value<long>() ?? 0;
                    var displayTotalSec = totalMs.HasValue
                        ? totalMs.Value / 1000.0
                        : wallElapsed.TotalSeconds;
                    output.WriteLine(string.Create(
                        CultureInfo.InvariantCulture,
                        $"耗时统计：导出 JSON {exportMs / 1000.0:F2}s | 建库 {dbBuildMs / 1000.0:F2}s | 计算哈希 {dbHashMs / 1000.0:F2}s | 总计 {displayTotalSec:F2}s"));
                }
                else
                {
                    output.WriteLine(string.Create(CultureInfo.InvariantCulture, $"总耗时：{wallElapsed.TotalSeconds:F2}s"));
                }

                output.WriteLine($"构建报告：{metadataPath}");
            }
            catch (Exception exception)
            {
                output.WriteLine($"提示：读取构建报告 {metadataPath} 失败（{exception.Message}）。");
                output.WriteLine(string.Create(CultureInfo.InvariantCulture, $"总耗时：{wallElapsed.TotalSeconds:F2}s"));
            }
        }
        else
        {
            output.WriteLine(string.Create(CultureInfo.InvariantCulture, $"总耗时：{wallElapsed.TotalSeconds:F2}s"));
        }

        if (exitCode == 1)
        {
            output.WriteLine("提示：数据库已生成，但有少量资源包导出失败，详情请查看构建报告或日志输出。");
        }
    }

    private static void PrintMissingFfrDbError(
        TextWriter error,
        Exception exception,
        string executableDirectory,
        bool isWindows)
    {
        var dbExeName = isWindows ? "ffr-db.exe" : "ffr-db";
        var exportExeName = isWindows ? "ffr-export.exe" : "ffr-export";

        error.WriteLine($"错误：找不到 {dbExeName}，无法执行建库。");
        error.WriteLine($"详情：{exception.Message}");
        error.WriteLine("下一步建议：");
        error.WriteLine($"  1. 请将下载的压缩包完整解压到同一目录，确保 {dbExeName} 与 {exportExeName} 位于同一文件夹（当前目录：{executableDirectory}）；");
        error.WriteLine($"  2. 不要单独移动 {exportExeName} 到桌面或其他目录运行；");
        error.WriteLine($"  3. 如使用自编译版本，可将 {dbExeName} 复制到程序同目录或加入环境变量 PATH 后重试。");
    }

    private static void PrintFailureAdvice(
        TextWriter error,
        Exception? exception,
        string executableDirectory,
        bool isWindows)
    {
        var dbExeName = isWindows ? "ffr-db.exe" : "ffr-db";
        var message = exception?.Message ?? string.Empty;

        error.WriteLine("下一步建议：");
        if (message.Contains("ffr-db", StringComparison.OrdinalIgnoreCase))
        {
            error.WriteLine($"  1. 确认 {dbExeName} 与本程序在同一目录（{executableDirectory}）且未被安全软件拦截；");
            error.WriteLine("  2. 确认输出磁盘有足够可用空间，然后重新运行本向导。");
            return;
        }

        if (message.Contains(".usmap", StringComparison.OrdinalIgnoreCase)
            || message.Contains("映射文件", StringComparison.Ordinal))
        {
            error.WriteLine("  1. 确认所选 .usmap 文件是从当前版本运行中的游戏通过 UE4SS 转储得到的完整文件；");
            error.WriteLine("  2. 如果游戏刚更新过版本，请重新转储一份最新的 .usmap 后再运行。");
            return;
        }

        if (message.Contains("游戏目录", StringComparison.Ordinal)
            || message.Contains("FFRS/", StringComparison.Ordinal)
            || message.Contains("FFRS-Win64-Shipping.exe", StringComparison.OrdinalIgnoreCase))
        {
            error.WriteLine("  1. 确认所选目录是《FINAL FANTASY RESONANCE》的游戏安装根目录；");
            error.WriteLine("  2. 如文件缺失，请在 Steam 中右键游戏 -> 属性 -> 已安装文件 -> 验证游戏文件的完整性。");
            return;
        }

        error.WriteLine("  1. 根据上方错误信息检查游戏目录、.usmap/.jmap 文件以及输出目录权限；");
        error.WriteLine("  2. 确认输出磁盘留有充足可用空间（建库过程需要数 GB 临时空间）；");
        error.WriteLine("  3. 排查完成后重新双击运行本程序。");
    }

    private static void WaitForExit(TextReader input, TextWriter output)
    {
        output.WriteLine();
        output.Write("按回车退出...");
        output.Flush();
        _ = input.ReadLine();
    }

    private static bool TryNormalizeFullPath(string cleanedPath, out string fullPath, out string errorMessage)
    {
        try
        {
            fullPath = PathSupport.TrimTrailingSeparators(Path.GetFullPath(cleanedPath));
            errorMessage = string.Empty;
            return true;
        }
        catch (Exception exception)
        {
            fullPath = string.Empty;
            errorMessage = exception.Message;
            return false;
        }
    }

    private static bool HasOwnBuildDbMetadata(string ioOutputDir)
    {
        var metaPath = Path.Combine(ioOutputDir, BuildMetadataFileName);
        if (!File.Exists(metaPath))
        {
            return false;
        }

        try
        {
            var doc = JObject.Parse(File.ReadAllText(metaPath, Utf8WithoutBom));
            var command = doc.Property("command")?.Value.Value<string>();
            return string.Equals(command, "build-db", StringComparison.Ordinal)
                && doc.Property("cue4ParseVersion") is not null;
        }
        catch (JsonException)
        {
            return false;
        }
    }

    private static bool IsYes(string value)
    {
        return value.Equals("y", StringComparison.OrdinalIgnoreCase)
            || value.Equals("yes", StringComparison.OrdinalIgnoreCase)
            || value.Equals("是", StringComparison.Ordinal);
    }

    private static bool IsNo(string value)
    {
        return value.Equals("n", StringComparison.OrdinalIgnoreCase)
            || value.Equals("no", StringComparison.OrdinalIgnoreCase)
            || value.Equals("q", StringComparison.OrdinalIgnoreCase)
            || value.Equals("否", StringComparison.Ordinal);
    }
}

internal static partial class Program
{
    internal static BuildDbRunner DefaultBuildDbRunner => RunBuildDbAsync;

    private static Task<int> RunInteractiveWizardAsync()
    {
        TryEnableUtf8Console();
        return InteractiveWizard.RunAsync(
            Console.In,
            Console.Out,
            Console.Error,
            DefaultBuildDbRunner,
            () => SteamLocator.DiscoverGames(),
            () => FfrDbLocator.Resolve(null),
            AppContext.BaseDirectory);
    }

    private static void TryEnableUtf8Console()
    {
        try
        {
            if (!Console.IsOutputRedirected)
            {
                Console.OutputEncoding = new UTF8Encoding(false);
            }

            if (!Console.IsInputRedirected)
            {
                Console.InputEncoding = new UTF8Encoding(false);
            }
        }
        catch
        {
            // Ignore console encoding errors in headless or restricted terminal environments.
        }
    }
}
