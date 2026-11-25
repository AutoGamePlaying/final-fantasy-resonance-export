using System.Diagnostics;
using System.Globalization;
using System.Text.Json;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

internal static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        if (args.Any(arg => string.Equals(arg, "--self-test", StringComparison.OrdinalIgnoreCase)))
        {
            return RunSelfTest();
        }

        Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
        Application.EnableVisualStyles();
        Application.SetCompatibleTextRenderingDefault(false);
        Application.Run(new MainForm());
        return 0;
    }

    /// <summary>
    /// Headless dependency check for Windows CI: verifies WinForms assemblies, shared SteamLocator,
    /// JSON serialization, and GUI core helpers load cleanly without displaying any window.
    /// </summary>
    private static int RunSelfTest()
    {
        try
        {
            var winFormsAssembly = typeof(Form).Assembly.GetName().Name;
            var listViewType = typeof(ListView).FullName;
            var progressBarType = typeof(ProgressBar).FullName;
            var highDpiMode = Application.HighDpiMode;
            if (string.IsNullOrEmpty(winFormsAssembly)
                || string.IsNullOrEmpty(listViewType)
                || string.IsNullOrEmpty(progressBarType))
            {
                throw new InvalidOperationException("WinForms 程序集元数据加载异常。");
            }

            if (SteamLocator.ResonanceGame.AppId != 3259780)
            {
                throw new InvalidOperationException("共享 SteamLocator 配置异常。");
            }

            if (!BuildCliCommand.TryParseStepProgress("==> 步骤 2/5：导出游戏数据为 JSON", out var step)
                || step.CurrentStep != 2
                || step.TotalSteps != 5)
            {
                throw new InvalidOperationException("进度解析自检失败。");
            }

            var sampleOutput = GuiPaths.BuildDefaultOutputDirectory(
                Path.GetTempPath(),
                SteamLocator.ResonanceGame.Code,
                "24250053");
            if (string.IsNullOrWhiteSpace(sampleOutput))
            {
                throw new InvalidOperationException("默认输出路径计算自检失败。");
            }

            var serialized = JsonSerializer.Serialize(new GuiSettings(Path.GetTempPath(), true));
            var deserialized = JsonSerializer.Deserialize<GuiSettings>(serialized);
            if (deserialized is null || !deserialized.IncludeJmap)
            {
                throw new InvalidOperationException("配置 JSON 序列化自检失败。");
            }

            Debug.WriteLine(string.Create(
                CultureInfo.InvariantCulture,
                $"self-test ok: {winFormsAssembly}, dpi={highDpiMode}, appid={SteamLocator.ResonanceGame.AppId}"));
            return 0;
        }
        catch (Exception exception)
        {
            Console.Error.WriteLine($"窗口程序自检失败：{exception.Message}");
            return 1;
        }
    }
}
