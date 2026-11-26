using System.Diagnostics;
using System.Globalization;
using System.Runtime.CompilerServices;
using System.Text;
using System.Text.Json;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

internal static class Program
{
    [STAThread]
    public static int Main(string[] args)
    {
        var isSelfTest = args.Any(arg => string.Equals(arg, "--self-test", StringComparison.OrdinalIgnoreCase));
        var mode = isSelfTest ? "self-test" : "gui";

        // Configure DPI awareness before collecting startup metadata, isolated in a non-inlined helper
        // so Main can still create a startup log even if WinForms initialization throws.
        var dpiSetting = ConfigureHighDpiSafely(out var dpiError);
        var startupInfo = GuiStartupLog.CollectStartupInfo(typeof(Program).Assembly, dpiSetting, mode);
        var startupLog = GuiStartupLog.StartSession(startupInfo);

        if (!string.IsNullOrWhiteSpace(dpiError))
        {
            startupLog.WriteWarning($"高 DPI 初始化失败：{dpiError}");
        }

        if (isSelfTest)
        {
            return RunSelfTest(startupLog);
        }

        return RunGui(startupLog);
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static string ConfigureHighDpiSafely(out string? error)
    {
        error = null;
        try
        {
            var applied = Application.SetHighDpiMode(HighDpiMode.PerMonitorV2);
            var actual = Application.HighDpiMode;
            return $"{actual} (SetPerMonitorV2={applied})";
        }
        catch (Exception exception)
        {
            error = $"{exception.GetType().FullName}: {exception.Message}";
            return "unavailable";
        }
    }

    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int RunGui(GuiStartupLogSession startupLog)
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

        try
        {
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
        }
        catch (Exception exception)
        {
            startupLog.WriteException("SetUnhandledExceptionMode", exception);
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
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);

            startupLog.WriteInfo("正在构造主窗口 MainForm...");
            using var mainForm = new MainForm(startupLog);
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
    /// Headless Windows CI self-test: verifies WinForms assemblies, shared SteamLocator, JSON serialization,
    /// GUI core helpers, and constructs MainForm with real Win32 control handles (without showing the window).
    /// </summary>
    [MethodImpl(MethodImplOptions.NoInlining)]
    private static int RunSelfTest(GuiStartupLogSession startupLog)
    {
        TryConfigureUtf8Console();
        try
        {
            startupLog.WriteInfo("开始执行窗口程序自检（--self-test）...");

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

            var serialized = TextOutput.SerializeSystemJson(new GuiSettings(Path.GetTempPath(), true));
            var deserialized = JsonSerializer.Deserialize<GuiSettings>(serialized);
            if (deserialized is null || !deserialized.IncludeJmap)
            {
                throw new InvalidOperationException("配置 JSON 序列化自检失败。");
            }

            // Exercise the exact same WinForms startup sequence and MainForm construction without showing the form.
            Application.SetUnhandledExceptionMode(UnhandledExceptionMode.CatchException);
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);

            startupLog.WriteInfo("自检：正在构造 MainForm 并调用 CreateControl()（不显示窗口）...");
            using (var form = new MainForm(startupLog))
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

                using var settingsDialog = new SettingsDialog(null, form.Font, form.Font);
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
                $"自检通过：assembly={winFormsAssembly}, dpi={highDpiMode}, appid={SteamLocator.ResonanceGame.AppId}"));
            Debug.WriteLine(string.Create(
                CultureInfo.InvariantCulture,
                $"self-test ok: {winFormsAssembly}, dpi={highDpiMode}, appid={SteamLocator.ResonanceGame.AppId}"));
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
        }
    }

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
