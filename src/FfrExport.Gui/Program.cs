using System.Diagnostics;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Runtime.InteropServices;
using System.Text;
using System.Text.Json;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

internal static class Program
{
    private const uint MbOk = 0x00000000;
    private const uint MbYesNo = 0x00000004;
    private const uint MbIconError = 0x00000010;
    private const uint MbIconWarning = 0x00000030;
    private const uint MbDefButton1 = 0x00000000;
    private const int IdYes = 6;

    [STAThread]
    public static int Main(string[] args)
    {
        var isSelfTest = args.Any(arg => string.Equals(arg, "--self-test", StringComparison.OrdinalIgnoreCase));
        var mode = isSelfTest ? "self-test" : "gui";

        // 1. Detect process integrity level and resolve a writable data root BEFORE initializing
        //    any WinForms API (including SetHighDpiMode / EnableVisualStyles).
        var integrity = ProcessIntegritySupport.DetectCurrent();
        var agpResolution = AgpPaths.ResolveWritableAgpRoot(integrityOverride: integrity);
        if (!agpResolution.Success)
        {
            if (isSelfTest)
            {
                TryConfigureUtf8Console();
                Console.Error.WriteLine($"窗口程序自检失败：{agpResolution.ErrorMessage}");
                Environment.ExitCode = 1;
                return 1;
            }

            if (agpResolution.Source == AgpRootSource.AgpHomeEnv)
            {
                // Explicit AGP_HOME must never silently fall back; use native Win32 MessageBoxW
                // which requires neither WinForms visual styles nor a writable %TEMP%.
                ShowNativeMessageBoxSafely(
                    agpResolution.ErrorMessage ?? "环境变量 AGP_HOME 指定的目录不可写。",
                    "数据目录不可写",
                    MbOk | MbIconError);
                Environment.ExitCode = 1;
                return 1;
            }

            var selectedResolution = PromptUserForWritableAgpRoot(agpResolution);
            if (selectedResolution is null || !selectedResolution.Success)
            {
                Environment.ExitCode = 1;
                return 1;
            }

            agpResolution = selectedResolution;
        }

        // 2. Redirect process-local TEMP/TMP into <agpRoot>\ffr-export\tmp on Windows BEFORE
        //    calling SetHighDpiMode / EnableVisualStyles or any WinForms control creation, so
        //    single-file WinForms visual-styles activation context never touches system %TEMP%.
        var tempConfigured = AgpPaths.TryConfigureProcessTempDirectory(
            agpResolution,
            out var configuredTempDir,
            out var tempConfigError);

        // 3. Configure WinForms DPI awareness and visual styles for both normal GUI and --self-test.
        var dpiSetting = ConfigureWinFormsSafely(
            out var dpiError,
            out var visualStylesException);

        var startupInfo = GuiStartupLog.CollectStartupInfo(
            typeof(Program).Assembly,
            dpiSetting,
            mode,
            integrity,
            agpResolution.ToolRoot);
        var startupLog = GuiStartupLog.StartSession(startupInfo, agpRootResolution: agpResolution);

        if (tempConfigured && !string.IsNullOrWhiteSpace(configuredTempDir))
        {
            startupLog.WriteInfo($"进程临时目录已指向数据根目录子目录：{configuredTempDir}");
        }
        else if (!string.IsNullOrWhiteSpace(tempConfigError))
        {
            startupLog.WriteWarning($"配置数据根目录 tmp 子目录失败：{tempConfigError}");
        }

        if (!string.IsNullOrWhiteSpace(dpiError))
        {
            startupLog.WriteWarning($"高 DPI 初始化失败：{dpiError}");
        }

        if (visualStylesException is not null)
        {
            startupLog.WriteWarning(
                $"视觉样式初始化失败（将沿用应用清单中的视觉样式）：{visualStylesException.GetType().FullName}: {visualStylesException.Message}");
        }

        if (isSelfTest)
        {
            return RunSelfTest(startupLog, agpResolution, visualStylesException);
        }

        return RunGui(startupLog, agpResolution);
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static string ConfigureWinFormsSafely(
        out string? dpiError,
        out Exception? visualStylesException)
    {
        dpiError = null;
        visualStylesException = null;

        string dpiSetting;
        try
        {
            var applied = Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
            var actual = Application.HighDpiMode;
            dpiSetting = $"{actual} (SetPerMonitorV2={applied})";
        }
        catch (Exception exception)
        {
            dpiError = $"{exception.GetType().FullName}: {exception.Message}";
            dpiSetting = "unavailable";
        }

        try
        {
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        }
        catch
        {
            // Continue even if unhandled exception mode was already set.
        }

        try
        {
            Application.SetCompatibleTextRenderingDefault(false);
        }
        catch
        {
            // Continue if text rendering default was already locked by an earlier dialog.
        }

        // Single-file WinForms EnableVisualStyles creates a temporary activation-context manifest
        // in TEMP; TEMP/TMP have already been redirected into <agpRoot>\ffr-export\tmp above.
        try
        {
            Application.EnableVisualStyles();
        }
        catch (Exception exception)
        {
            visualStylesException = exception;
        }

        return dpiSetting;
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static AgpRootResolution? PromptUserForWritableAgpRoot(AgpRootResolution initialFailure)
    {
        var currentFailure = initialFailure;
        while (true)
        {
            // Show the initial guidance via native Win32 MessageBoxW so the message is guaranteed
            // to appear even when no writable directory exists and visual styles are not initialized.
            var promptText = AgpPaths.FormatAllCandidatesFailedGuiMessage(currentFailure);
            var answer = ShowNativeMessageBoxSafely(
                promptText,
                "请选择可写的数据目录",
                MbYesNo | MbIconWarning | MbDefButton1);
            if (answer != IdYes)
            {
                return null;
            }

            string? selectedPath;
            try
            {
                using var dialog = new FolderBrowserDialog
                {
                    Description = "选择一个可写文件夹作为本工具的数据根目录（用于保存日志、设置和导出文件）",
                    UseDescriptionForTitle = true,
                    ShowNewFolderButton = true,
                };
                if (dialog.ShowDialog() != DialogResult.OK || string.IsNullOrWhiteSpace(dialog.SelectedPath))
                {
                    continue;
                }

                selectedPath = dialog.SelectedPath;
            }
            catch (Exception exception)
            {
                ShowNativeMessageBoxSafely(
                    $"无法打开「选择文件夹」对话框（{exception.GetType().Name}: {exception.Message}）。\n\n"
                    + $"请设置环境变量 {AgpPaths.AgpHomeEnvVar} 指向一个可写文件夹后重新启动本工具，"
                    + "或右键压缩包 -> 属性 -> 解除锁定后重新解压。",
                    "无法打开文件夹选择器",
                    MbOk | MbIconError);
                return null;
            }

            var accepted = AgpPaths.TryAcceptUserSelectedAgpRoot(
                selectedPath,
                currentFailure.Integrity,
                currentFailure.Failures);
            if (accepted.Success)
            {
                return accepted;
            }

            ShowNativeMessageBoxSafely(
                accepted.ErrorMessage ?? "所选文件夹不可写，请重新选择其他目录。",
                "所选目录不可写",
                MbOk | MbIconError);
            currentFailure = accepted;
        }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int RunGui(GuiStartupLogSession startupLog, AgpRootResolution agpResolution)
    {
        var fatalDialogShown = 0;
        var exitCode = 0;
        var mainFormShown = false;

        void ReportFatal(string phase, Exception exception)
        {
            Volatile.Write(ref exitCode, 1);
            startupLog.WriteException(phase, exception);

            if (Interlocked.Exchange(ref fatalDialogShown, 1) != 0)
            {
                return;
            }

            var headline = mainFormShown ? "窗口程序运行异常" : "窗口程序启动失败";
            var message = GuiStartupLog.FormatCrashDialogMessage(
                exception,
                startupLog.LogFilePath,
                startupLog.LastWriteError,
                headline);
            ShowCrashMessageBoxSafely(message, headline, startupLog);
        }

        Application.ThreadException += (_, eventArgs) =>
        {
            ReportFatal("Application.ThreadException", eventArgs.Exception);
            try
            {
                Environment.ExitCode = 1;
                Application.Exit();
            }
            catch
            {
                // Best-effort exit after logging and displaying the fatal error dialog.
            }
        };

        AppDomain.CurrentDomain.UnhandledException += (_, eventArgs) =>
        {
            if (eventArgs.ExceptionObject is Exception exception)
            {
                ReportFatal("AppDomain.UnhandledException", exception);
                return;
            }

            Volatile.Write(ref exitCode, 1);
            startupLog.WriteFatalObject("AppDomain.UnhandledException", eventArgs.ExceptionObject);
            if (Interlocked.Exchange(ref fatalDialogShown, 1) == 0)
            {
                var synthetic = new InvalidOperationException(
                    $"发生未处理的非异常对象：{eventArgs.ExceptionObject?.ToString() ?? "(null)"}");
                var headline = mainFormShown ? "窗口程序运行异常" : "窗口程序启动失败";
                var message = GuiStartupLog.FormatCrashDialogMessage(
                    synthetic,
                    startupLog.LogFilePath,
                    startupLog.LastWriteError,
                    headline);
                ShowCrashMessageBoxSafely(message, headline, startupLog);
            }
        };

        TaskScheduler.UnobservedTaskException += (_, eventArgs) =>
        {
            startupLog.WriteException("TaskScheduler.UnobservedTaskException", eventArgs.Exception);
        };

        try
        {
            startupLog.WriteInfo("正在构造主窗口 MainForm...");
            using var mainForm = new MainForm(startupLog, agpResolution);
            mainForm.Shown += (_, _) =>
            {
                mainFormShown = true;
                startupLog.WriteInfo("主窗口已显示。");
            };

            startupLog.WriteInfo("正在启动 WinForms 消息循环...");
            Application.Run(mainForm);

            var finalExitCode = Volatile.Read(ref exitCode);
            startupLog.WriteInfo(string.Create(CultureInfo.InvariantCulture, $"窗口程序退出，退出码={finalExitCode}。"));
            if (finalExitCode != 0)
            {
                Environment.ExitCode = finalExitCode;
            }

            return finalExitCode;
        }
        catch (Exception exception)
        {
            ReportFatal(mainFormShown ? "Application.Run" : "MainForm 构造或启动", exception);
            Environment.ExitCode = 1;
            return 1;
        }
    }

    /// <summary>
    /// Headless Windows CI self-test: verifies process TEMP/TMP redirection, WinForms visual styles
    /// &amp; assemblies, shared SteamLocator, JSON serialization, writable agp tmp/ directory (never
    /// touching system %TEMP%), and constructs MainForm with real Win32 control handles.
    /// </summary>
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int RunSelfTest(
        GuiStartupLogSession startupLog,
        AgpRootResolution agpResolution,
        Exception? visualStylesException)
    {
        TryConfigureUtf8Console();
        try
        {
            startupLog.WriteInfo("开始执行窗口程序自检（--self-test）...");

            if (visualStylesException is not null)
            {
                throw new InvalidOperationException(
                    $"Application.EnableVisualStyles 初始化失败（可能误触不可写的系统 TEMP）：{visualStylesException.Message}",
                    visualStylesException);
            }

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

            var activeAgpRoot = agpResolution.AgpRoot
                ?? throw new InvalidOperationException("数据根目录解析结果为空。");
            var expectedTmpDir = AgpPaths.GetTempDirectoryForAgpRoot(activeAgpRoot);
            if (OperatingSystem.IsWindows())
            {
                var currentTemp = Environment.GetEnvironmentVariable("TEMP");
                var currentTmp = Environment.GetEnvironmentVariable("TMP");
                if (!string.Equals(currentTemp, expectedTmpDir, StringComparison.OrdinalIgnoreCase)
                    || !string.Equals(currentTmp, expectedTmpDir, StringComparison.OrdinalIgnoreCase))
                {
                    throw new InvalidOperationException(
                        $"进程 TEMP/TMP 未重定向到数据根目录 tmp 子目录：expected={expectedTmpDir}, TEMP={currentTemp}, TMP={currentTmp}");
                }
            }

            var probeResult = AgpPaths.ProbeDirectoryWritable(activeAgpRoot);
            if (!probeResult.IsWritable)
            {
                throw new InvalidOperationException($"数据根目录试写自检失败：{probeResult.ErrorMessage}");
            }

            var sampleOutputRoot = AgpPaths.GetDefaultOutputRootForAgpRoot(activeAgpRoot);
            var sampleOutput = GuiPaths.BuildDefaultOutputDirectory(
                sampleOutputRoot,
                SteamLocator.ResonanceGame.Code,
                "24250053");
            if (string.IsNullOrWhiteSpace(sampleOutput))
            {
                throw new InvalidOperationException("默认输出路径计算自检失败。");
            }

            var serialized = TextOutput.SerializeSystemJson(new GuiSettings(sampleOutputRoot, true));
            var deserialized = JsonSerializer.Deserialize<GuiSettings>(serialized);
            if (deserialized is null || !deserialized.IncludeJmap)
            {
                throw new InvalidOperationException("配置 JSON 序列化自检失败。");
            }

            startupLog.WriteInfo("自检：正在构造 MainForm 并调用 CreateControl()（不显示窗口）...");
            using (var form = new MainForm(startupLog, agpResolution))
            {
                form.CreateControl();
                var handle = form.Handle;
                if (handle == IntPtr.Zero || !form.IsHandleCreated)
                {
                    throw new InvalidOperationException("主窗口句柄创建失败。");
                }

                EnsureChildControlHandlesCreated(form);
                form.PerformLayout();

                if (form.Visible)
                {
                    throw new InvalidOperationException("自检期间主窗口不应可见。");
                }

                startupLog.WriteInfo(string.Create(
                    CultureInfo.InvariantCulture,
                    $"自检：MainForm 创建成功（handle=0x{handle.ToInt64():X}, topControls={form.Controls.Count}），正在校验 SettingsDialog。"));

                using var settingsDialog = new SettingsDialog(agpResolution, form.Font, form.Font);
                settingsDialog.CreateControl();
                if (settingsDialog.Handle == IntPtr.Zero || !settingsDialog.IsHandleCreated)
                {
                    throw new InvalidOperationException("设置对话框句柄创建失败。");
                }

                EnsureChildControlHandlesCreated(settingsDialog);
                settingsDialog.PerformLayout();
            }

            startupLog.WriteInfo(string.Create(
                CultureInfo.InvariantCulture,
                $"自检通过：assembly={winFormsAssembly}, dpi={highDpiMode}, integrity={agpResolution.Integrity.Label}, data_root={agpResolution.ToolRoot}, appid={SteamLocator.ResonanceGame.AppId}"));
            Debug.WriteLine(string.Create(
                CultureInfo.InvariantCulture,
                $"self-test ok: {winFormsAssembly}, dpi={highDpiMode}, integrity={agpResolution.Integrity.Label}, appid={SteamLocator.ResonanceGame.AppId}"));
            return 0;
        }
        catch (Exception exception)
        {
            startupLog.WriteException("self-test", exception);
            var errorReport = GuiStartupLog.FormatSelfTestFailureMessage(
                exception,
                startupLog.LogFilePath,
                startupLog.LastWriteError);
            try
            {
                Console.Error.WriteLine(errorReport);
            }
            catch
            {
                // Ignore stderr write errors if no standard error handle is attached.
            }

            Environment.ExitCode = 1;
            return 1;
        }
    }

    private static void EnsureChildControlHandlesCreated(Control parent)
    {
        foreach (Control child in parent.Controls)
        {
            child.CreateControl();
            var childHandle = child.Handle;
            if (childHandle == IntPtr.Zero || !child.IsHandleCreated)
            {
                throw new InvalidOperationException($"子控件 {child.GetType().Name} 句柄创建失败。");
            }

            EnsureChildControlHandlesCreated(child);
        }
    }

    private static void ShowCrashMessageBoxSafely(
        string message,
        string caption,
        GuiStartupLogSession startupLog)
    {
        try
        {
            MessageBox.Show(message, caption, MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
        catch (Exception dialogException)
        {
            startupLog.WriteException("MessageBox.Show 失败", dialogException);
            ShowNativeMessageBoxSafely(message, caption, MbOk | MbIconError);
        }
    }

    private static int ShowNativeMessageBoxSafely(string message, string caption, uint flags)
    {
        if (!OperatingSystem.IsWindows())
        {
            return 0;
        }

        try
        {
            return NativeMessageBoxW(IntPtr.Zero, message, caption, flags);
        }
        catch
        {
            return 0;
        }
    }

    [DllImport("user32.dll", CharSet = CharSet.Unicode, EntryPoint = "MessageBoxW", ExactSpelling = true)]
    private static extern int NativeMessageBoxW(IntPtr hWnd, string text, string caption, uint type);

    private static void TryConfigureUtf8Console()
    {
        try
        {
            Console.OutputEncoding = new UTF8Encoding(false);
        }
        catch
        {
            // WinExe processes without redirected stdio handles may fail when changing console encoding.
        }
    }
}
