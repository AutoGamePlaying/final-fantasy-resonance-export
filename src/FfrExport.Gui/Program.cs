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

        // 0. Pre-apply language from --lang or AGP_LANG (default: en) so even early error dialogs are localized.
        Localization.SetLanguage(Localization.ResolveLanguage(null));
        string? cliLanguageOverride = null;
        try
        {
            var extractedLang = Localization.ExtractCliLanguageArgs(args, stopAfterLocresSubcommand: false);
            if (extractedLang.CliLanguage is not null)
            {
                if (!Localization.TryNormalizeLanguageCode(extractedLang.CliLanguage, out var canonicalCli))
                {
                    throw new ArgumentException(L.Format("Common_UnsupportedLanguage", extractedLang.CliLanguage));
                }

                cliLanguageOverride = canonicalCli;
                Localization.SetLanguage(canonicalCli);
            }
        }
        catch (ArgumentException exception)
        {
            var errorLine = L.Format("Common_ErrorPrefix", exception.Message);
            if (isSelfTest)
            {
                TryConfigureUtf8Console();
                Console.Error.WriteLine(errorLine);
            }
            else
            {
                ShowNativeMessageBoxSafely(errorLine, L.Get("Gui_WindowTitle"), MbOk | MbIconError);
            }

            Environment.ExitCode = 2;
            return 2;
        }

        // 1. Detect process integrity level and resolve a writable data root BEFORE initializing
        //    any WinForms API (including SetHighDpiMode / EnableVisualStyles).
        var integrity = ProcessIntegritySupport.DetectCurrent();
        var agpResolution = AgpPaths.ResolveWritableAgpRoot(integrityOverride: integrity);
        if (!agpResolution.Success)
        {
            if (isSelfTest)
            {
                TryConfigureUtf8Console();
                Console.Error.WriteLine(L.Format("Gui_SelfTestFailedPrefix", agpResolution.ErrorMessage));
                Environment.ExitCode = 1;
                return 1;
            }

            if (agpResolution.Source == AgpRootSource.AgpHomeEnv)
            {
                // Explicit AGP_HOME must never silently fall back; use native Win32 MessageBoxW
                // which requires neither WinForms visual styles nor a writable %TEMP%.
                ShowNativeMessageBoxSafely(
                    agpResolution.ErrorMessage ?? L.Get("Gui_AgpEnvUnwritableFallback"),
                    L.Get("Gui_DataDirUnwritableTitle"),
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

        // Apply saved GUI language from settings.json unless overridden by --lang or AGP_LANG.
        if (cliLanguageOverride is null)
        {
            var savedLang = Localization.TryReadSavedLanguageFromSettings(agpHomeOverride: agpResolution.AgpRoot);
            Localization.SetLanguage(Localization.ResolveLanguage(null, savedLanguage: savedLang));
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
            startupLog.WriteInfo(L.Format("Gui_LogProcessTempRedirected", configuredTempDir));
        }
        else if (!string.IsNullOrWhiteSpace(tempConfigError))
        {
            startupLog.WriteWarning(L.Format("Gui_LogProcessTempFailed", tempConfigError));
        }

        if (!string.IsNullOrWhiteSpace(dpiError))
        {
            startupLog.WriteWarning(L.Format("Gui_LogDpiInitFailed", dpiError));
        }

        if (visualStylesException is not null)
        {
            startupLog.WriteWarning(L.Format(
                "Gui_LogVisualStylesFailed",
                visualStylesException.GetType().FullName,
                visualStylesException.Message));
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
                L.Get("Gui_ChooseWritableDataDirTitle"),
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
                    Description = L.Get("Gui_ChooseWritableDataDirDesc"),
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
                    L.Format("Gui_FolderPickerFailedBody", exception.GetType().Name, exception.Message, AgpPaths.AgpHomeEnvVar),
                    L.Get("Gui_FolderPickerFailedTitle"),
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
                accepted.ErrorMessage ?? L.Get("Gui_SelectedDirUnwritableFallback"),
                L.Get("Gui_SelectedDirUnwritableTitle"),
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

            var headline = mainFormShown
                ? L.Get("Gui_CrashRuntimeHeadline")
                : L.Get("GuiCore_CrashDefaultHeadline");
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
                    L.Format("Gui_UnhandledNonException", eventArgs.ExceptionObject?.ToString() ?? "(null)"));
                var headline = mainFormShown
                    ? L.Get("Gui_CrashRuntimeHeadline")
                    : L.Get("GuiCore_CrashDefaultHeadline");
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
            startupLog.WriteInfo(L.Get("Gui_LogConstructingMainForm"));
            using var mainForm = new MainForm(startupLog, agpResolution);
            mainForm.Shown += (_, _) =>
            {
                mainFormShown = true;
                startupLog.WriteInfo(L.Get("Gui_LogMainFormShown"));
            };

            startupLog.WriteInfo(L.Get("Gui_LogStartingMessageLoop"));
            Application.Run(mainForm);

            var finalExitCode = Volatile.Read(ref exitCode);
            startupLog.WriteInfo(L.Format("Gui_LogGuiExited", finalExitCode));
            if (finalExitCode != 0)
            {
                Environment.ExitCode = finalExitCode;
            }

            return finalExitCode;
        }
        catch (Exception exception)
        {
            ReportFatal(mainFormShown ? "Application.Run" : L.Get("Gui_LogMainFormInitPhase"), exception);
            Environment.ExitCode = 1;
            return 1;
        }
    }

    /// <summary>
    /// Headless Windows CI self-test: verifies process TEMP/TMP redirection, WinForms visual styles
    /// &amp; assemblies, shared SteamLocator, localization across en/zh-Hans/ja, JSON serialization,
    /// writable agp tmp/ directory (never touching system %TEMP%), and constructs MainForm with real Win32 control handles.
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
            startupLog.WriteInfo(L.Get("Gui_SelfTestStart"));

            if (visualStylesException is not null)
            {
                throw new InvalidOperationException(
                    L.Format("Gui_SelfTestVisualStylesFailed", visualStylesException.Message),
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
                throw new InvalidOperationException(L.Get("Gui_SelfTestWinFormsMetadataFailed"));
            }

            if (SteamLocator.ResonanceGame.AppId != 3259780)
            {
                throw new InvalidOperationException(L.Get("Gui_SelfTestSteamLocatorFailed"));
            }

            if (!BuildCliCommand.TryParseStepProgress("@@progress {\"step\":2,\"total\":5,\"message\":\"Exporting\"}", out var step)
                || step.CurrentStep != 2
                || step.TotalSteps != 5
                || BuildCliCommand.TryParseStepProgress("==> Step 2/5: Exporting", out _))
            {
                throw new InvalidOperationException(L.Get("Gui_SelfTestProgressParseFailed"));
            }

            var titleEn = Localization.GetForLanguage("Gui_WindowTitle", Localization.DefaultLanguage);
            var titleZh = Localization.GetForLanguage("Gui_WindowTitle", Localization.ChineseSimplifiedLanguage);
            var titleJa = Localization.GetForLanguage("Gui_WindowTitle", Localization.JapaneseLanguage);
            if (string.IsNullOrWhiteSpace(titleEn)
                || string.IsNullOrWhiteSpace(titleZh)
                || string.IsNullOrWhiteSpace(titleJa)
                || string.Equals(titleEn, titleZh, StringComparison.Ordinal)
                || string.Equals(titleEn, titleJa, StringComparison.Ordinal)
                || string.Equals(titleZh, titleJa, StringComparison.Ordinal))
            {
                throw new InvalidOperationException(L.Get("Gui_SelfTestLocalizationFailed"));
            }

            var activeAgpRoot = agpResolution.AgpRoot
                ?? throw new InvalidOperationException(L.Get("Gui_SelfTestAgpRootEmpty"));
            var expectedTmpDir = AgpPaths.GetTempDirectoryForAgpRoot(activeAgpRoot);
            if (OperatingSystem.IsWindows())
            {
                var currentTemp = Environment.GetEnvironmentVariable("TEMP");
                var currentTmp = Environment.GetEnvironmentVariable("TMP");
                if (!string.Equals(currentTemp, expectedTmpDir, StringComparison.OrdinalIgnoreCase)
                    || !string.Equals(currentTmp, expectedTmpDir, StringComparison.OrdinalIgnoreCase))
                {
                    throw new InvalidOperationException(
                        L.Format("Gui_SelfTestTempNotRedirected", expectedTmpDir, currentTemp, currentTmp));
                }
            }

            var probeResult = AgpPaths.ProbeDirectoryWritable(activeAgpRoot);
            if (!probeResult.IsWritable)
            {
                throw new InvalidOperationException(L.Format("Gui_SelfTestProbeFailed", probeResult.ErrorMessage));
            }

            var sampleOutputRoot = AgpPaths.GetDefaultOutputRootForAgpRoot(activeAgpRoot);
            var sampleOutput = GuiPaths.BuildDefaultOutputDirectory(
                sampleOutputRoot,
                SteamLocator.ResonanceGame.Code,
                "24250053");
            if (string.IsNullOrWhiteSpace(sampleOutput))
            {
                throw new InvalidOperationException(L.Get("Gui_SelfTestOutputCalcFailed"));
            }

            var serialized = TextOutput.SerializeSystemJson(new GuiSettings(sampleOutputRoot, Localization.DefaultLanguage));
            var deserialized = JsonSerializer.Deserialize<GuiSettings>(serialized);
            if (deserialized is null
                || !string.Equals(deserialized.OutputRootDirectory, sampleOutputRoot, StringComparison.Ordinal)
                || !string.Equals(deserialized.Language, Localization.DefaultLanguage, StringComparison.Ordinal))
            {
                throw new InvalidOperationException(L.Get("Gui_SelfTestSettingsJsonFailed"));
            }

            startupLog.WriteInfo(L.Get("Gui_SelfTestCreatingMainForm"));
            using (var form = new MainForm(startupLog, agpResolution))
            {
                form.CreateControl();
                var handle = form.Handle;
                if (handle == IntPtr.Zero || !form.IsHandleCreated)
                {
                    throw new InvalidOperationException(L.Get("Gui_SelfTestMainFormHandleFailed"));
                }

                EnsureChildControlHandlesCreated(form);
                form.PerformLayout();

                if (form.Visible)
                {
                    throw new InvalidOperationException(L.Get("Gui_SelfTestMainFormShouldBeHidden"));
                }

                startupLog.WriteInfo(L.Format(
                    "Gui_SelfTestMainFormCreated",
                    handle.ToInt64().ToString("X", CultureInfo.InvariantCulture),
                    form.Controls.Count));

                using var settingsDialog = new SettingsDialog(agpResolution, form.Font, form.Font);
                settingsDialog.CreateControl();
                if (settingsDialog.Handle == IntPtr.Zero || !settingsDialog.IsHandleCreated)
                {
                    throw new InvalidOperationException(L.Get("Gui_SelfTestSettingsHandleFailed"));
                }

                EnsureChildControlHandlesCreated(settingsDialog);
                settingsDialog.PerformLayout();

                using var elevationDialog = new Ue4ssElevationDialog(
                    Path.Combine(activeAgpRoot, "sample-denied.dll"),
                    form.Font,
                    form.Font);
                elevationDialog.CreateControl();
                if (elevationDialog.Handle == IntPtr.Zero || !elevationDialog.IsHandleCreated)
                {
                    throw new InvalidOperationException(L.Format("Gui_SelfTestChildHandleFailed", nameof(Ue4ssElevationDialog)));
                }

                EnsureChildControlHandlesCreated(elevationDialog);
                elevationDialog.PerformLayout();
            }

            startupLog.WriteInfo(L.Format(
                "Gui_SelfTestPassed",
                winFormsAssembly,
                highDpiMode,
                agpResolution.Integrity.Label,
                agpResolution.ToolRoot,
                SteamLocator.ResonanceGame.AppId));
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
                throw new InvalidOperationException(L.Format("Gui_SelfTestChildHandleFailed", child.GetType().Name));
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
            startupLog.WriteException(L.Get("Gui_LogMessageBoxFailed"), dialogException);
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
