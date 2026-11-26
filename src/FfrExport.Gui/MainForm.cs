using System.Diagnostics;
using System.Globalization;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

/// <summary>
/// Single-window WinForms frontend for ffr-export build-db (supports English, Simplified Chinese, and Japanese).
/// Delegates all extraction and database building to the sibling ffr-export.exe CLI.
/// </summary>
internal sealed class MainForm : Form
{
    private const int MinimumWindowWidth = 800;
    private const int MinimumWindowHeight = 680;
    private const int MaxLogCharacters = 400_000;

    private enum ActiveOperation
    {
        None,
        Exporting,
        FetchingUsmap,
        CleaningUe4ss,
    }

    private enum GameStatusKind
    {
        None,
        FoundCount,
        NotFound,
        ManualAdded,
        RawError,
    }

    private enum UsmapStatusKind
    {
        None,
        AutoFound,
        NotFound,
        ManualSelected,
        Fetched,
        RawError,
    }

    private enum ProgressDisplayKind
    {
        Ready,
        ExportComplete,
        ExportCompletePartial,
        ExportFailedExit,
        ExportFailed,
        ExportCancelled,
        FetchComplete,
        FetchFailedExit,
        FetchCancelled,
        FetchFailed,
        CleanupComplete,
        CleanupFailedExit,
        CleanupCancelled,
        Raw,
    }

    private enum ResultPanelKind
    {
        NotRun,
        Exporting,
        Cancelled,
        ExportFailedExit,
        Report,
        Raw,
    }

    private readonly GuiStartupLogSession? _startupLog;
    private readonly AgpRootResolution _agpResolution;
    private readonly string _activeAgpRoot;
    private readonly string _activeToolRoot;
    private readonly string _defaultOutputRoot;
    private readonly List<Font> _ownedFonts = [];
    private readonly HashSet<string> _recordedFontWarnings = new(StringComparer.Ordinal);

    private readonly Font _uiFont;
    private readonly Font _boldUiFont;
    private readonly Font _startButtonFont;
    private readonly Font _verdictFont;
    private readonly Font _monoFont;

    private readonly List<GameEntry> _games = [];

    private readonly Label _languageLabel;
    private readonly ComboBox _languageComboBox;

    private readonly Panel _dataRootBannerPanel;
    private readonly Label _dataRootBannerLabel;

    private readonly GroupBox _gameGroup;
    private readonly ListView _gameListView;
    private readonly Button _manualGameButton;
    private readonly Button _rescanGameButton;
    private readonly Button _settingsButton;
    private readonly Label _gameStatusLabel;

    private readonly GroupBox _mappingGroup;
    private readonly Label _usmapCaption;
    private readonly Label _usmapStatusLabel;
    private readonly Label _usmapPathCaption;
    private readonly TextBox _usmapPathTextBox;
    private readonly Button _browseUsmapButton;
    private readonly Button _cleanupUe4ssButton;
    private readonly Button _fetchUsmapButton;

    private readonly GroupBox _outputGroup;
    private readonly Label _outputCaption;
    private readonly TextBox _outputDirTextBox;
    private readonly Button _browseOutputButton;
    private readonly Label _outputHintLabel;

    private readonly Button _startButton;
    private readonly Button _cancelButton;
    private readonly Label _progressLabel;
    private readonly ProgressBar _progressBar;
    private readonly TextBox _logTextBox;

    private readonly GroupBox _resultGroup;
    private readonly Label _verdictLabel;
    private readonly Label _verdictDetailLabel;
    private readonly Label _dbPathCaption;
    private readonly TextBox _dbPathTextBox;
    private readonly Label _fileShaCaption;
    private readonly TextBox _fileShaTextBox;
    private readonly Label _contentShaCaption;
    private readonly TextBox _contentShaTextBox;
    private readonly Button _openOutputButton;
    private readonly Button _copyHashButton;
    private readonly Label _copyFeedbackLabel;

    private readonly StatusStrip _statusStrip;
    private readonly ToolStripStatusLabel _integrityStatusLabel;
    private readonly ToolStripStatusLabel _dataRootStatusLabel;

    private readonly string _settingsPath;
    private string _outputRootDirectory;
    private GameEntry? _selectedGame;
    private DiscoveredMappingFile? _selectedUsmap;
    private int _usmapCandidateCount = 1;
    private bool _usmapManuallySelected;
    private bool _userCustomizedOutputDir;
    private bool _suppressOutputTextChanged;
    private bool _suppressLanguageChanged;
    private ActiveOperation _activeOperation = ActiveOperation.None;
    private bool _isExporting => _activeOperation != ActiveOperation.None;
    private CancellationTokenSource? _exportCts;
    private string? _lastFetchedUsmapPath;
    private BuildReportSummary? _lastReport;
    private string? _lastCompletedOutputDir;

    private GameStatusKind _gameStatusKind = GameStatusKind.None;
    private string? _gameStatusArg;
    private UsmapStatusKind _usmapStatusKind = UsmapStatusKind.None;
    private bool _outputHintIsDefault = true;
    private ProgressDisplayKind _progressDisplayKind = ProgressDisplayKind.Ready;
    private int _lastProgressExitCode;
    private ResultPanelKind _resultPanelKind = ResultPanelKind.NotRun;
    private int _lastResultExitCode;

    internal MainForm(GuiStartupLogSession? startupLog = null, AgpRootResolution? agpResolution = null)
    {
        _startupLog = startupLog;
        var startupWarnings = new List<string>();

        _agpResolution = agpResolution ?? AgpPaths.ResolveWritableAgpRoot();
        _activeAgpRoot = _agpResolution.AgpRoot ?? AppContext.BaseDirectory;
        _activeToolRoot = AgpPaths.ResolveToolRootForAgpRoot(_activeAgpRoot);
        _defaultOutputRoot = AgpPaths.GetDefaultOutputRootForAgpRoot(_activeAgpRoot);

        var bannerMessage = AgpPaths.FormatGuiBannerMessage(_agpResolution);
        if (!string.IsNullOrWhiteSpace(bannerMessage))
        {
            startupWarnings.Add(bannerMessage);
        }

        _settingsPath = GuiSettingsStore.GetDefaultSettingsPath(agpHomeOverride: _activeAgpRoot);
        var legacyPath = _agpResolution.Source == AgpRootSource.LocalAppData
            && !AgpPaths.IsAgpHomeEnvironmentOverrideActive()
            ? GuiSettingsStore.GetLegacySettingsPath()
            : null;
        var settingsResult = GuiSettingsStore.LoadWithDiagnostics(
            _settingsPath,
            legacySettingsPathOverride: legacyPath);
        if (!string.IsNullOrWhiteSpace(settingsResult.MigratedFromPath))
        {
            var migratedNote = L.Format("Gui_SettingsMigratedNote", settingsResult.MigratedFromPath, _settingsPath);
            startupWarnings.Add(migratedNote);
            _startupLog?.WriteInfo(migratedNote);
        }

        if (!string.IsNullOrWhiteSpace(settingsResult.WarningMessage))
        {
            startupWarnings.Add(settingsResult.WarningMessage);
            _startupLog?.WriteWarning(settingsResult.WarningMessage);
        }

        var loadedSettings = settingsResult.Settings;
        string? initialOutputWarning = null;
        if (!string.IsNullOrWhiteSpace(loadedSettings.OutputRootDirectory))
        {
            if (GuiPaths.TryNormalizeDirectoryPath(
                loadedSettings.OutputRootDirectory,
                out var normalizedSavedRoot,
                out var savedRootError))
            {
                _outputRootDirectory = normalizedSavedRoot;
            }
            else
            {
                _outputRootDirectory = _defaultOutputRoot;
                initialOutputWarning = L.Format(
                    "Gui_SavedOutputInvalidHint",
                    loadedSettings.OutputRootDirectory,
                    savedRootError,
                    _outputRootDirectory);
            }
        }
        else
        {
            _outputRootDirectory = _defaultOutputRoot;
        }

        if (!string.IsNullOrWhiteSpace(initialOutputWarning))
        {
            startupWarnings.Add(initialOutputWarning);
            _startupLog?.WriteWarning(initialOutputWarning);
        }

        _uiFont = CreateUiFont(9.5f, FontStyle.Regular, startupWarnings);
        _boldUiFont = CreateUiFont(9.5f, FontStyle.Bold, startupWarnings);
        _startButtonFont = CreateUiFont(11f, FontStyle.Bold, startupWarnings);
        _verdictFont = CreateUiFont(10.5f, FontStyle.Bold, startupWarnings);
        _monoFont = CreateMonoFont(9f, startupWarnings);

        SuspendLayout();

        Text = L.Get("Gui_WindowTitle");
        Font = _uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        StartPosition = FormStartPosition.CenterScreen;
        MinimumSize = new Size(MinimumWindowWidth, MinimumWindowHeight);
        ClientSize = new Size(920, 800);

        var rootLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 7,
            Padding = new Padding(12, 8, 12, 8),
        };
        rootLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        // Top-right language selector bar
        var topBarPanel = new FlowLayoutPanel
        {
            Dock = DockStyle.Top,
            FlowDirection = FlowDirection.RightToLeft,
            WrapContents = false,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Margin = new Padding(0, 0, 0, 4),
        };
        _languageComboBox = new ComboBox
        {
            DropDownStyle = ComboBoxStyle.DropDownList,
            Width = 136,
            Margin = new Padding(4, 0, 0, 0),
        };
        foreach (var option in Localization.LanguageOptions)
        {
            _languageComboBox.Items.Add(option.NativeName);
        }

        _suppressLanguageChanged = true;
        _languageComboBox.SelectedIndex = FindLanguageIndex(Localization.CurrentLanguage);
        _suppressLanguageChanged = false;
        _languageComboBox.SelectedIndexChanged += OnLanguageComboBoxSelectedIndexChanged;

        _languageLabel = new Label
        {
            Text = L.Get("Gui_LanguageLabel"),
            AutoSize = true,
            Anchor = AnchorStyles.Right,
            Margin = new Padding(0, 4, 4, 0),
        };
        // FlowDirection is RightToLeft, so add combo first to place it at the far right.
        topBarPanel.Controls.Add(_languageComboBox);
        topBarPanel.Controls.Add(_languageLabel);

        // Optional prominent banner for Low IL or fallback data root
        _dataRootBannerPanel = new Panel
        {
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            BackColor = Color.FromArgb(255, 248, 225),
            BorderStyle = BorderStyle.FixedSingle,
            Padding = new Padding(10, 6, 10, 6),
            Margin = new Padding(0, 0, 0, 8),
            Visible = !string.IsNullOrWhiteSpace(bannerMessage),
        };
        _dataRootBannerLabel = new Label
        {
            Text = bannerMessage ?? string.Empty,
            Font = _boldUiFont,
            ForeColor = Color.FromArgb(146, 64, 14),
            AutoSize = true,
            MaximumSize = new Size(870, 0),
            Dock = DockStyle.Fill,
        };
        _dataRootBannerPanel.Controls.Add(_dataRootBannerLabel);

        // Section 1: Game selection
        _gameGroup = new GroupBox
        {
            Text = L.Get("Gui_GroupGame"),
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Padding = new Padding(10, 6, 10, 8),
        };
        var gameLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            AutoSize = true,
            ColumnCount = 2,
            RowCount = 2,
        };
        gameLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        gameLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        gameLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        gameLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        _gameListView = new ListView
        {
            View = View.Details,
            FullRowSelect = true,
            MultiSelect = false,
            HideSelection = false,
            GridLines = true,
            HeaderStyle = ColumnHeaderStyle.Nonclickable,
            Dock = DockStyle.Fill,
            Height = 104,
            Margin = new Padding(0, 2, 8, 4),
        };
        _gameListView.Columns.Add(L.Get("Gui_ColGameName"), 210);
        _gameListView.Columns.Add(L.Get("Gui_ColSteamBuildId"), 115);
        _gameListView.Columns.Add(L.Get("Gui_ColInstallDir"), 320);
        _gameListView.SelectedIndexChanged += OnGameSelectedIndexChanged;
        _gameListView.Resize += (_, _) => AdjustGameListColumnWidths();

        var gameButtonPanel = new FlowLayoutPanel
        {
            FlowDirection = FlowDirection.TopDown,
            WrapContents = false,
            AutoSize = true,
            Margin = new Padding(0, 2, 0, 4),
        };
        _manualGameButton = new Button
        {
            Text = L.Get("Gui_BtnManualGame"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(168, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0, 0, 0, 5),
        };
        _manualGameButton.Click += OnManualSelectGameClick;

        _rescanGameButton = new Button
        {
            Text = L.Get("Gui_BtnRescanSteam"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(168, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0, 0, 0, 5),
        };
        _rescanGameButton.Click += (_, _) => ScanAndPopulateGames();

        _settingsButton = new Button
        {
            Text = L.Get("Gui_BtnSettings"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(168, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0),
        };
        _settingsButton.Click += OnOpenSettingsClick;

        gameButtonPanel.Controls.Add(_manualGameButton);
        gameButtonPanel.Controls.Add(_rescanGameButton);
        gameButtonPanel.Controls.Add(_settingsButton);

        _gameStatusLabel = new Label
        {
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            Margin = new Padding(0, 2, 0, 0),
        };

        gameLayout.Controls.Add(_gameListView, 0, 0);
        gameLayout.Controls.Add(gameButtonPanel, 1, 0);
        gameLayout.Controls.Add(_gameStatusLabel, 0, 1);
        gameLayout.SetColumnSpan(_gameStatusLabel, 2);
        _gameGroup.Controls.Add(gameLayout);

        // Section 2: Resonance-specific .usmap mapping file
        _mappingGroup = new GroupBox
        {
            Text = L.Get("Gui_GroupMappings"),
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Padding = new Padding(10, 6, 10, 8),
            Margin = new Padding(0, 6, 0, 0),
        };
        var mappingLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            AutoSize = true,
            ColumnCount = 3,
            RowCount = 3,
        };
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _usmapCaption = new Label
        {
            Text = L.Get("Gui_LabelUsmapRequired"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 4),
        };
        _usmapStatusLabel = new Label
        {
            AutoSize = true,
            Anchor = AnchorStyles.Left | AnchorStyles.Right,
            MaximumSize = new Size(900, 0),
            Margin = new Padding(0, 5, 8, 4),
        };
        _browseUsmapButton = new Button
        {
            Text = L.Get("Gui_BtnBrowseUsmap"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(118, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0, 0, 6, 0),
        };
        _browseUsmapButton.Click += OnBrowseUsmapClick;

        _cleanupUe4ssButton = new Button
        {
            Text = L.Get("Gui_BtnCleanupUe4ss"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(124, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0),
        };
        _cleanupUe4ssButton.Click += OnCleanupUe4ssClick;

        var usmapActionButtons = new FlowLayoutPanel
        {
            FlowDirection = FlowDirection.LeftToRight,
            WrapContents = false,
            AutoSize = true,
            Anchor = AnchorStyles.Right,
            Margin = new Padding(0, 2, 0, 4),
        };
        usmapActionButtons.Controls.Add(_browseUsmapButton);
        usmapActionButtons.Controls.Add(_cleanupUe4ssButton);

        _fetchUsmapButton = new Button
        {
            Text = L.Get("Gui_BtnFetchUsmap"),
            Font = _boldUiFont,
            AutoSize = true,
            Height = 32,
            Padding = new Padding(10, 2, 10, 2),
            Visible = false,
            Margin = new Padding(0, 2, 0, 6),
        };
        _fetchUsmapButton.Click += OnFetchUsmapClick;

        _usmapPathCaption = new Label
        {
            Text = L.Get("Gui_LabelFilePath"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            ForeColor = SystemColors.GrayText,
            Margin = new Padding(0, 4, 6, 6),
        };
        _usmapPathTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            ReadOnly = true,
            BackColor = SystemColors.ControlLightLight,
            Margin = new Padding(0, 2, 8, 6),
        };

        mappingLayout.Controls.Add(_usmapCaption, 0, 0);
        mappingLayout.Controls.Add(_usmapStatusLabel, 1, 0);
        mappingLayout.Controls.Add(usmapActionButtons, 2, 0);

        mappingLayout.Controls.Add(_fetchUsmapButton, 1, 1);
        mappingLayout.SetColumnSpan(_fetchUsmapButton, 2);

        mappingLayout.Controls.Add(_usmapPathCaption, 0, 2);
        mappingLayout.Controls.Add(_usmapPathTextBox, 1, 2);
        mappingLayout.SetColumnSpan(_usmapPathTextBox, 2);
        _mappingGroup.Controls.Add(mappingLayout);

        // Section 3: Output directory
        _outputGroup = new GroupBox
        {
            Text = L.Get("Gui_GroupOutput"),
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Padding = new Padding(10, 6, 10, 8),
            Margin = new Padding(0, 6, 0, 0),
        };
        var outputLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            AutoSize = true,
            ColumnCount = 3,
            RowCount = 2,
        };
        outputLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        outputLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        outputLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _outputCaption = new Label
        {
            Text = L.Get("Gui_LabelOutputDir"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 6, 6, 4),
        };
        _outputDirTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            Margin = new Padding(0, 3, 8, 4),
        };
        _outputDirTextBox.TextChanged += (_, _) =>
        {
            if (!_suppressOutputTextChanged)
            {
                _userCustomizedOutputDir = true;
            }
        };
        _browseOutputButton = new Button
        {
            Text = L.Get("Gui_BtnBrowseOutput"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(106, 30),
            Padding = new Padding(8, 0, 8, 0),
            Margin = new Padding(0, 1, 0, 4),
        };
        _browseOutputButton.Click += OnBrowseOutputClick;

        _outputHintIsDefault = string.IsNullOrWhiteSpace(initialOutputWarning);
        _outputHintLabel = new Label
        {
            Text = _outputHintIsDefault
                ? FormatDefaultOutputHintText()
                : initialOutputWarning!,
            AutoSize = true,
            ForeColor = _outputHintIsDefault
                ? SystemColors.GrayText
                : Color.FromArgb(180, 100, 0),
            MaximumSize = new Size(900, 0),
            Margin = new Padding(0, 2, 0, 0),
        };

        outputLayout.Controls.Add(_outputCaption, 0, 0);
        outputLayout.Controls.Add(_outputDirTextBox, 1, 0);
        outputLayout.Controls.Add(_browseOutputButton, 2, 0);
        outputLayout.Controls.Add(_outputHintLabel, 0, 1);
        outputLayout.SetColumnSpan(_outputHintLabel, 3);
        _outputGroup.Controls.Add(outputLayout);

        // Section 4: Start / Cancel, progress bar, scrollable log
        var runPanel = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 3,
            Margin = new Padding(0, 8, 0, 0),
        };
        runPanel.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        runPanel.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        runPanel.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        runPanel.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));

        var actionRow = new TableLayoutPanel
        {
            Dock = DockStyle.Top,
            AutoSize = true,
            ColumnCount = 2,
            RowCount = 1,
            Margin = new Padding(0, 0, 0, 6),
        };
        actionRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        actionRow.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _startButton = new Button
        {
            Text = L.Get("Gui_BtnStartExport"),
            Font = _startButtonFont,
            Dock = DockStyle.Fill,
            Height = 42,
            Margin = new Padding(0, 0, 8, 0),
        };
        _startButton.Click += OnStartExportClick;

        _cancelButton = new Button
        {
            Text = L.Get("Gui_BtnCancel"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(112, 42),
            Padding = new Padding(12, 0, 12, 0),
            Enabled = false,
            Margin = new Padding(0),
        };
        _cancelButton.Click += OnCancelExportClick;

        actionRow.Controls.Add(_startButton, 0, 0);
        actionRow.Controls.Add(_cancelButton, 1, 0);

        var progressRow = new TableLayoutPanel
        {
            Dock = DockStyle.Top,
            AutoSize = true,
            ColumnCount = 2,
            RowCount = 1,
            Margin = new Padding(0, 0, 0, 6),
        };
        progressRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 45f));
        progressRow.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 55f));

        _progressLabel = new Label
        {
            Text = L.Get("Gui_StatusReady"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 3, 8, 0),
        };
        _progressBar = new ProgressBar
        {
            Dock = DockStyle.Fill,
            Height = 20,
            Minimum = 0,
            Maximum = 100,
            Style = ProgressBarStyle.Continuous,
            Margin = new Padding(0),
        };

        progressRow.Controls.Add(_progressLabel, 0, 0);
        progressRow.Controls.Add(_progressBar, 1, 0);

        _logTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            Multiline = true,
            ReadOnly = true,
            ScrollBars = ScrollBars.Both,
            WordWrap = false,
            Font = _monoFont,
            BackColor = SystemColors.Window,
            MinimumSize = new Size(0, 120),
            Margin = new Padding(0),
        };

        runPanel.Controls.Add(actionRow, 0, 0);
        runPanel.Controls.Add(progressRow, 0, 1);
        runPanel.Controls.Add(_logTextBox, 0, 2);

        // Section 5: Completion summary
        _resultGroup = new GroupBox
        {
            Text = L.Get("Gui_GroupResult"),
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Padding = new Padding(10, 6, 10, 8),
            Margin = new Padding(0, 8, 0, 0),
        };
        var resultLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            AutoSize = true,
            ColumnCount = 3,
            RowCount = 5,
        };
        resultLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        resultLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        resultLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _verdictLabel = new Label
        {
            Text = L.Get("Gui_ResultNotRun"),
            Font = _verdictFont,
            ForeColor = SystemColors.GrayText,
            AutoSize = true,
            Margin = new Padding(0, 2, 0, 2),
        };
        _verdictDetailLabel = new Label
        {
            Text = L.Get("Gui_ResultNotRunDetail"),
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(900, 0),
            Margin = new Padding(0, 0, 0, 6),
        };

        _dbPathCaption = new Label
        {
            Text = L.Get("Gui_LabelDbPath"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 4),
        };
        _dbPathTextBox = CreateReadOnlyField(_uiFont);
        _openOutputButton = new Button
        {
            Text = L.Get("Gui_BtnOpenOutput"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(144, 29),
            Padding = new Padding(8, 0, 8, 0),
            Enabled = false,
            Margin = new Padding(0, 1, 0, 4),
        };
        _openOutputButton.Click += OnOpenOutputFolderClick;

        _fileShaCaption = new Label
        {
            Text = L.Get("Gui_LabelFileSha"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 4),
        };
        _fileShaTextBox = CreateReadOnlyField(_monoFont);
        _copyHashButton = new Button
        {
            Text = L.Get("Gui_BtnCopyHash"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(144, 29),
            Padding = new Padding(8, 0, 8, 0),
            Enabled = false,
            Margin = new Padding(0, 1, 0, 4),
        };
        _copyHashButton.Click += OnCopyHashClick;

        _contentShaCaption = new Label
        {
            Text = L.Get("Gui_LabelContentSha"),
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 2),
        };
        _contentShaTextBox = CreateReadOnlyField(_monoFont);
        _copyFeedbackLabel = new Label
        {
            Text = string.Empty,
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            ForeColor = Color.FromArgb(0, 128, 0),
            Margin = new Padding(2, 5, 0, 2),
        };

        resultLayout.Controls.Add(_verdictLabel, 0, 0);
        resultLayout.SetColumnSpan(_verdictLabel, 3);
        resultLayout.Controls.Add(_verdictDetailLabel, 0, 1);
        resultLayout.SetColumnSpan(_verdictDetailLabel, 3);

        resultLayout.Controls.Add(_dbPathCaption, 0, 2);
        resultLayout.Controls.Add(_dbPathTextBox, 1, 2);
        resultLayout.Controls.Add(_openOutputButton, 2, 2);

        resultLayout.Controls.Add(_fileShaCaption, 0, 3);
        resultLayout.Controls.Add(_fileShaTextBox, 1, 3);
        resultLayout.Controls.Add(_copyHashButton, 2, 3);

        resultLayout.Controls.Add(_contentShaCaption, 0, 4);
        resultLayout.Controls.Add(_contentShaTextBox, 1, 4);
        resultLayout.Controls.Add(_copyFeedbackLabel, 2, 4);

        _resultGroup.Controls.Add(resultLayout);

        rootLayout.Controls.Add(topBarPanel, 0, 0);
        rootLayout.Controls.Add(_dataRootBannerPanel, 0, 1);
        rootLayout.Controls.Add(_gameGroup, 0, 2);
        rootLayout.Controls.Add(_mappingGroup, 0, 3);
        rootLayout.Controls.Add(_outputGroup, 0, 4);
        rootLayout.Controls.Add(runPanel, 0, 5);
        rootLayout.Controls.Add(_resultGroup, 0, 6);

        _statusStrip = new StatusStrip
        {
            Dock = DockStyle.Bottom,
            SizingGrip = true,
        };
        _integrityStatusLabel = new ToolStripStatusLabel
        {
            Text = L.Format("Gui_StatusStripIntegrity", _agpResolution.Integrity.DisplayText),
            BorderSides = ToolStripStatusLabelBorderSides.Right,
            BorderStyle = Border3DStyle.Etched,
            ForeColor = _agpResolution.Integrity.IsLowOrUntrusted
                ? Color.FromArgb(180, 100, 0)
                : SystemColors.ControlText,
        };
        _dataRootStatusLabel = new ToolStripStatusLabel
        {
            Text = L.Format("Gui_StatusStripDataRoot", _activeToolRoot),
            Spring = true,
            TextAlign = ContentAlignment.MiddleLeft,
        };
        _statusStrip.Items.Add(_integrityStatusLabel);
        _statusStrip.Items.Add(_dataRootStatusLabel);

        // Add Dock.Fill before Dock.Bottom so WinForms reserves StatusStrip height first.
        Controls.Add(rootLayout);
        Controls.Add(_statusStrip);

        ResumeLayout(false);
        Shown += OnFormShown;

        foreach (var warning in startupWarnings)
        {
            AppendLogLine(L.Format("Gui_LogPrefixInfo", warning));
        }

        ScanAndPopulateGames();
    }

    private string FormatDefaultOutputHintText()
    {
        return L.Format("Gui_DefaultOutputHint", _defaultOutputRoot);
    }

    private static int FindLanguageIndex(string languageCode)
    {
        for (var i = 0; i < Localization.LanguageOptions.Count; i++)
        {
            if (string.Equals(Localization.LanguageOptions[i].Code, languageCode, StringComparison.OrdinalIgnoreCase))
            {
                return i;
            }
        }

        return 0;
    }

    private void OnLanguageComboBoxSelectedIndexChanged(object? sender, EventArgs e)
    {
        if (_suppressLanguageChanged || _isExporting)
        {
            return;
        }

        var index = _languageComboBox.SelectedIndex;
        if (index < 0 || index >= Localization.LanguageOptions.Count)
        {
            return;
        }

        var chosenCode = Localization.LanguageOptions[index].Code;
        if (string.Equals(chosenCode, Localization.CurrentLanguage, StringComparison.OrdinalIgnoreCase))
        {
            return;
        }

        Localization.SetLanguage(chosenCode);
        ApplyLocalizedTexts();
        SaveSettingsSafely();
    }

    private void ApplyLocalizedTexts()
    {
        SuspendLayout();
        try
        {
            Text = L.Get("Gui_WindowTitle");
            _languageLabel.Text = L.Get("Gui_LanguageLabel");

            var bannerMessage = AgpPaths.FormatGuiBannerMessage(_agpResolution);
            _dataRootBannerPanel.Visible = !string.IsNullOrWhiteSpace(bannerMessage);
            _dataRootBannerLabel.Text = bannerMessage ?? string.Empty;

            _gameGroup.Text = L.Get("Gui_GroupGame");
            if (_gameListView.Columns.Count >= 3)
            {
                _gameListView.Columns[0].Text = L.Get("Gui_ColGameName");
                _gameListView.Columns[1].Text = L.Get("Gui_ColSteamBuildId");
                _gameListView.Columns[2].Text = L.Get("Gui_ColInstallDir");
            }

            foreach (ListViewItem item in _gameListView.Items)
            {
                if (item.Tag is GameEntry entry && item.SubItems.Count >= 2)
                {
                    item.SubItems[1].Text = entry.SteamBuildId ?? L.Get("Common_Unknown");
                }
            }

            _manualGameButton.Text = L.Get("Gui_BtnManualGame");
            _rescanGameButton.Text = L.Get("Gui_BtnRescanSteam");
            _settingsButton.Text = L.Get("Gui_BtnSettings");
            RefreshGameStatusLabelText();

            _mappingGroup.Text = L.Get("Gui_GroupMappings");
            _usmapCaption.Text = L.Get("Gui_LabelUsmapRequired");
            _browseUsmapButton.Text = L.Get("Gui_BtnBrowseUsmap");
            _cleanupUe4ssButton.Text = L.Get("Gui_BtnCleanupUe4ss");
            _fetchUsmapButton.Text = L.Get("Gui_BtnFetchUsmap");
            _usmapPathCaption.Text = L.Get("Gui_LabelFilePath");
            RefreshUsmapStatusLabelText();

            _outputGroup.Text = L.Get("Gui_GroupOutput");
            _outputCaption.Text = L.Get("Gui_LabelOutputDir");
            _browseOutputButton.Text = L.Get("Gui_BtnBrowseOutput");
            if (_outputHintIsDefault)
            {
                _outputHintLabel.Text = FormatDefaultOutputHintText();
            }

            _startButton.Text = L.Get("Gui_BtnStartExport");
            _cancelButton.Text = L.Get("Gui_BtnCancel");
            RefreshProgressLabelText();

            _resultGroup.Text = L.Get("Gui_GroupResult");
            _dbPathCaption.Text = L.Get("Gui_LabelDbPath");
            _openOutputButton.Text = L.Get("Gui_BtnOpenOutput");
            _fileShaCaption.Text = L.Get("Gui_LabelFileSha");
            _copyHashButton.Text = L.Get("Gui_BtnCopyHash");
            _contentShaCaption.Text = L.Get("Gui_LabelContentSha");
            RefreshResultPanelText();

            if (!string.IsNullOrEmpty(_copyFeedbackLabel.Text))
            {
                _copyFeedbackLabel.Text = L.Get("Gui_CopiedFeedback");
            }

            _integrityStatusLabel.Text = L.Format("Gui_StatusStripIntegrity", _agpResolution.Integrity.DisplayText);
            _dataRootStatusLabel.Text = L.Format("Gui_StatusStripDataRoot", _activeToolRoot);
        }
        finally
        {
            ResumeLayout(true);
            PerformLayout();
            AdjustGameListColumnWidths();
        }
    }

    private void RefreshGameStatusLabelText()
    {
        switch (_gameStatusKind)
        {
            case GameStatusKind.FoundCount:
                _gameStatusLabel.Text = L.Format("Gui_GamesFoundStatus", _games.Count);
                break;
            case GameStatusKind.NotFound:
                _gameStatusLabel.Text = L.Get("Gui_GamesNotFoundStatus");
                break;
            case GameStatusKind.ManualAdded:
                _gameStatusLabel.Text = L.Format("Gui_GameAddedManually", _gameStatusArg);
                break;
        }
    }

    private void RefreshUsmapStatusLabelText()
    {
        switch (_usmapStatusKind)
        {
            case UsmapStatusKind.AutoFound when _selectedUsmap is not null:
                _usmapStatusLabel.Text = L.Format(
                    "Gui_UsmapAutoFoundStatus",
                    MappingDiscovery.FormatUsmapDisplay(_selectedUsmap, _usmapCandidateCount));
                break;
            case UsmapStatusKind.NotFound:
                _usmapStatusLabel.Text = MappingDiscovery.UsmapNotFoundGuidance;
                break;
            case UsmapStatusKind.ManualSelected when _selectedUsmap is not null:
                _usmapStatusLabel.Text = L.Format(
                    "Gui_UsmapManualStatus",
                    MappingDiscovery.FormatUsmapDisplay(_selectedUsmap));
                break;
            case UsmapStatusKind.Fetched when _selectedUsmap is not null:
                _usmapStatusLabel.Text = L.Format(
                    "Gui_UsmapFetchedStatus",
                    MappingDiscovery.FormatUsmapDisplay(_selectedUsmap));
                break;
        }
    }

    private void RefreshProgressLabelText()
    {
        switch (_progressDisplayKind)
        {
            case ProgressDisplayKind.Ready:
                _progressLabel.Text = L.Get("Gui_StatusReady");
                break;
            case ProgressDisplayKind.ExportComplete:
                _progressLabel.Text = L.Get("Gui_StatusExportComplete");
                break;
            case ProgressDisplayKind.ExportCompletePartial:
                _progressLabel.Text = L.Get("Gui_StatusExportCompletePartial");
                break;
            case ProgressDisplayKind.ExportFailedExit:
                _progressLabel.Text = L.Format("Gui_StatusExportFailedExit", _lastProgressExitCode);
                break;
            case ProgressDisplayKind.ExportFailed:
                _progressLabel.Text = L.Get("Gui_StatusExportFailed");
                break;
            case ProgressDisplayKind.ExportCancelled:
                _progressLabel.Text = L.Get("Gui_StatusCancelled");
                break;
            case ProgressDisplayKind.FetchComplete:
                _progressLabel.Text = L.Get("Gui_StatusFetchUsmapComplete");
                break;
            case ProgressDisplayKind.FetchFailedExit:
                _progressLabel.Text = L.Format("Gui_StatusFetchUsmapFailedExit", _lastProgressExitCode);
                break;
            case ProgressDisplayKind.FetchCancelled:
                _progressLabel.Text = L.Get("Gui_StatusFetchUsmapCancelled");
                break;
            case ProgressDisplayKind.FetchFailed:
                _progressLabel.Text = L.Get("Gui_StatusFetchUsmapFailed");
                break;
            case ProgressDisplayKind.CleanupComplete:
                _progressLabel.Text = L.Get("Gui_StatusCleanupComplete");
                break;
            case ProgressDisplayKind.CleanupFailedExit:
                _progressLabel.Text = L.Format("Gui_StatusCleanupFailedExit", _lastProgressExitCode);
                break;
            case ProgressDisplayKind.CleanupCancelled:
                _progressLabel.Text = L.Get("Gui_StatusCleanupCancelled");
                break;
        }
    }

    private void RefreshResultPanelText()
    {
        switch (_resultPanelKind)
        {
            case ResultPanelKind.NotRun:
                _verdictLabel.Text = L.Get("Gui_ResultNotRun");
                _verdictDetailLabel.Text = L.Get("Gui_ResultNotRunDetail");
                break;
            case ResultPanelKind.Exporting:
                _verdictLabel.Text = L.Get("Gui_ResultExporting");
                _verdictDetailLabel.Text = L.Get("Gui_ResultExportingDetail");
                break;
            case ResultPanelKind.Cancelled:
                _verdictLabel.Text = L.Get("Gui_ResultCancelled");
                _verdictDetailLabel.Text = L.Get("Gui_ResultCancelledDetail");
                break;
            case ResultPanelKind.ExportFailedExit:
                _verdictLabel.Text = L.Get("Gui_ResultExportFailed");
                _verdictDetailLabel.Text = L.Format("Gui_ResultExportFailedDetail", _lastResultExitCode);
                break;
            case ResultPanelKind.Report when _lastReport is not null && !string.IsNullOrWhiteSpace(_lastCompletedOutputDir):
                DisplayBuildReport(_lastReport, _lastCompletedOutputDir);
                break;
        }
    }

    protected override void Dispose(bool disposing)
    {
        if (disposing)
        {
            try
            {
                _exportCts?.Dispose();
            }
            catch
            {
                // Ignore cancellation source disposal errors during teardown.
            }

            foreach (var font in _ownedFonts)
            {
                try
                {
                    font.Dispose();
                }
                catch
                {
                    // Ignore font disposal errors during teardown.
                }
            }

            _ownedFonts.Clear();
        }

        base.Dispose(disposing);
    }

    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        if (_isExporting)
        {
            var opName = _activeOperation switch
            {
                ActiveOperation.FetchingUsmap => L.Get("Gui_OpNameFetchUsmap"),
                ActiveOperation.CleaningUe4ss => L.Get("Gui_OpNameCleanUe4ss"),
                _ => L.Get("Gui_OpNameExport"),
            };
            var answer = MessageBox.Show(
                this,
                L.Format("Gui_ConfirmCloseBody", opName),
                L.Get("Gui_ConfirmCloseTitle"),
                MessageBoxButtons.YesNo,
                MessageBoxIcon.Warning,
                MessageBoxDefaultButton.Button2);
            if (answer != DialogResult.Yes)
            {
                e.Cancel = true;
                return;
            }

            try
            {
                _exportCts?.Cancel();
            }
            catch
            {
                // Best-effort cancellation during window shutdown.
            }

            if (_activeOperation == ActiveOperation.FetchingUsmap && _selectedGame is not null)
            {
                Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(_selectedGame.GameDirectory);
            }
        }

        SaveSettingsSafely();
        base.OnFormClosing(e);
    }

    private void OnFormShown(object? sender, EventArgs e)
    {
        try
        {
            AdjustGameListColumnWidths();
            var cliLookup = GuiPaths.ResolveCliExecutable();
            if (!cliLookup.Found)
            {
                var errorMessage = cliLookup.ErrorMessage ?? L.Get("Gui_CliMissingFallback");
                AppendLogLine(L.Format("Gui_LogPrefixInfo", errorMessage));
                _startupLog?.WriteWarning(errorMessage);
                MessageBox.Show(
                    this,
                    errorMessage,
                    L.Get("Gui_CliMissingTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
            }
        }
        catch (Exception exception)
        {
            _startupLog?.WriteException("OnFormShown", exception);
            AppendLogLine(L.Format("Gui_LogPrefixWarn", L.Format("Gui_ShownCheckFailed", exception.Message)));
        }
    }

    private void ScanAndPopulateGames()
    {
        var previousSelectedDir = _selectedGame?.GameDirectory;
        _games.Clear();
        _gameListView.Items.Clear();
        _selectedGame = null;

        GameScanResult scanResult;
        try
        {
            scanResult = GameScanner.ScanSteamGamesSafe();
        }
        catch (Exception exception)
        {
            scanResult = new GameScanResult(
                [],
                L.Format("GuiCore_SteamScanFailed", exception.Message));
        }

        var scanFailed = !string.IsNullOrWhiteSpace(scanResult.ErrorMessage);
        if (scanFailed)
        {
            _gameStatusKind = GameStatusKind.RawError;
            _gameStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _gameStatusLabel.Text = scanResult.ErrorMessage!;
            _startupLog?.WriteWarning(scanResult.ErrorMessage!);
            AppendLogLine(L.Format("Gui_LogPrefixInfo", scanResult.ErrorMessage));
        }

        foreach (var game in scanResult.Games)
        {
            AddGameToListView(game);
        }

        if (_games.Count > 0)
        {
            _gameStatusKind = GameStatusKind.FoundCount;
            _gameStatusLabel.ForeColor = SystemColors.GrayText;
            _gameStatusLabel.Text = L.Format("Gui_GamesFoundStatus", _games.Count);

            var indexToSelect = 0;
            if (!string.IsNullOrWhiteSpace(previousSelectedDir))
            {
                var matchedIndex = _games.FindIndex(g =>
                    string.Equals(g.GameDirectory, previousSelectedDir, StringComparison.OrdinalIgnoreCase));
                if (matchedIndex >= 0)
                {
                    indexToSelect = matchedIndex;
                }
            }

            _gameListView.Items[indexToSelect].Selected = true;
            _gameListView.Items[indexToSelect].Focused = true;

            // ListView may defer SelectedIndexChanged until handle creation; apply initial selection immediately.
            if (_selectedGame is null)
            {
                ApplySelectedGame(_games[indexToSelect]);
            }
        }
        else
        {
            if (!scanFailed)
            {
                _gameStatusKind = GameStatusKind.NotFound;
                _gameStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
                _gameStatusLabel.Text = L.Get("Gui_GamesNotFoundStatus");
            }

            RefreshMappingsForSelectedGame(resetManualUsmap: false);
            UpdateDefaultOutputDirectory(SteamLocator.ResonanceGame.Code, null);
        }

        AdjustGameListColumnWidths();
    }

    private void AddGameToListView(GameEntry entry)
    {
        _games.Add(entry);
        var item = new ListViewItem(entry.DisplayName);
        item.SubItems.Add(entry.SteamBuildId ?? L.Get("Common_Unknown"));
        item.SubItems.Add(entry.GameDirectory);
        item.Tag = entry;
        _gameListView.Items.Add(item);
    }

    private void OnGameSelectedIndexChanged(object? sender, EventArgs e)
    {
        if (_gameListView.SelectedItems.Count == 0)
        {
            return;
        }

        if (_gameListView.SelectedItems[0].Tag is not GameEntry entry)
        {
            return;
        }

        if (ReferenceEquals(_selectedGame, entry))
        {
            return;
        }

        ApplySelectedGame(entry);
    }

    private void ApplySelectedGame(GameEntry entry)
    {
        _selectedGame = entry;
        RefreshMappingsForSelectedGame(resetManualUsmap: true);
        if (!_userCustomizedOutputDir)
        {
            UpdateDefaultOutputDirectory(entry.GameCode, entry.SteamBuildId);
        }
    }

    private void OnManualSelectGameClick(object? sender, EventArgs e)
    {
        using var dialog = new FolderBrowserDialog
        {
            Description = L.Get("Gui_SelectGameDialogDesc"),
            UseDescriptionForTitle = true,
            ShowNewFolderButton = false,
        };
        if (_selectedGame is not null && Directory.Exists(_selectedGame.GameDirectory))
        {
            dialog.InitialDirectory = _selectedGame.GameDirectory;
        }

        if (dialog.ShowDialog(this) != DialogResult.OK)
        {
            return;
        }

        var validation = GameScanner.ValidateManualGameDirectory(dialog.SelectedPath);
        if (!validation.IsValid || validation.Entry is null)
        {
            MessageBox.Show(
                this,
                validation.ErrorMessage ?? L.Get("Gui_GameInvalidFallback"),
                L.Get("Gui_GameCheckFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var existingIndex = _games.FindIndex(g =>
            string.Equals(g.GameDirectory, validation.Entry.GameDirectory, StringComparison.OrdinalIgnoreCase));
        if (existingIndex >= 0)
        {
            _games[existingIndex] = validation.Entry;
            var existingItem = _gameListView.Items[existingIndex];
            existingItem.Text = validation.Entry.DisplayName;
            existingItem.SubItems[1].Text = validation.Entry.SteamBuildId ?? L.Get("Common_Unknown");
            existingItem.SubItems[2].Text = validation.Entry.GameDirectory;
            existingItem.Tag = validation.Entry;
            existingItem.Selected = true;
            existingItem.Focused = true;
        }
        else
        {
            AddGameToListView(validation.Entry);
            var addedIndex = _gameListView.Items.Count - 1;
            _gameListView.Items[addedIndex].Selected = true;
            _gameListView.Items[addedIndex].Focused = true;
        }

        _gameStatusKind = GameStatusKind.ManualAdded;
        _gameStatusArg = validation.Entry.GameDirectory;
        _gameStatusLabel.ForeColor = SystemColors.GrayText;
        _gameStatusLabel.Text = L.Format("Gui_GameAddedManually", validation.Entry.GameDirectory);
    }

    private void RefreshMappingsForSelectedGame(bool resetManualUsmap)
    {
        if (resetManualUsmap || !_usmapManuallySelected)
        {
            _usmapManuallySelected = false;
            try
            {
                var usmapResult = MappingDiscovery.DiscoverUsmap(
                    _selectedGame?.GameDirectory,
                    agpUsmapDirectoryOverride: AgpPaths.GetUsmapDirectoryForAgpRoot(_activeAgpRoot));
                _selectedUsmap = usmapResult.Selected;
                _usmapCandidateCount = usmapResult.Candidates.Count;
                if (!string.IsNullOrWhiteSpace(usmapResult.ScanError))
                {
                    var warn = L.Format("Gui_UsmapDirAccessWarn", usmapResult.ScanError);
                    _startupLog?.WriteWarning(warn);
                    AppendLogLine(L.Format("Gui_LogPrefixInfo", warn));
                }

                if (usmapResult.Selected is not null)
                {
                    _usmapStatusKind = UsmapStatusKind.AutoFound;
                    _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                    _usmapStatusLabel.Font = _boldUiFont;
                    _usmapStatusLabel.Text = L.Format(
                        "Gui_UsmapAutoFoundStatus",
                        MappingDiscovery.FormatUsmapDisplay(usmapResult.Selected, usmapResult.Candidates.Count));
                    _usmapPathTextBox.Text = usmapResult.Selected.FullPath;
                    _fetchUsmapButton.Visible = false;
                }
                else
                {
                    _usmapStatusKind = string.IsNullOrWhiteSpace(usmapResult.ScanError)
                        ? UsmapStatusKind.NotFound
                        : UsmapStatusKind.RawError;
                    _usmapStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
                    _usmapStatusLabel.Font = _uiFont;
                    _usmapStatusLabel.Text = usmapResult.NotFoundGuidance ?? MappingDiscovery.UsmapNotFoundGuidance;
                    _usmapPathTextBox.Text = string.Empty;
                    _fetchUsmapButton.Visible = true;
                }
            }
            catch (Exception exception)
            {
                _selectedUsmap = null;
                _usmapStatusKind = UsmapStatusKind.RawError;
                var errorMessage = L.Format("Gui_UsmapAutoFindError", exception.Message);
                _usmapStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
                _usmapStatusLabel.Font = _uiFont;
                _usmapStatusLabel.Text = errorMessage;
                _usmapPathTextBox.Text = string.Empty;
                _fetchUsmapButton.Visible = true;
                _startupLog?.WriteWarning(errorMessage);
                AppendLogLine(L.Format("Gui_LogPrefixInfo", errorMessage));
            }
        }
    }

    private void OnBrowseUsmapClick(object? sender, EventArgs e)
    {
        using var dialog = new OpenFileDialog
        {
            Title = L.Get("Gui_OpenUsmapDialogTitle"),
            Filter = L.Get("Gui_OpenUsmapDialogFilter"),
            CheckFileExists = true,
            Multiselect = false,
        };
        if (_selectedUsmap is not null)
        {
            var dir = Path.GetDirectoryName(_selectedUsmap.FullPath);
            if (!string.IsNullOrEmpty(dir) && Directory.Exists(dir))
            {
                dialog.InitialDirectory = dir;
            }
        }
        else if (_selectedGame is not null)
        {
            var agpUsmapDir = AgpPaths.GetUsmapDirectoryForAgpRoot(_activeAgpRoot);
            var ue4ssDir = SteamLocator.CombineRelativeSubpath(
                _selectedGame.GameDirectory,
                MappingDiscovery.RelativeUe4ssSubpath);
            var win64Dir = SteamLocator.CombineRelativeSubpath(
                _selectedGame.GameDirectory,
                MappingDiscovery.RelativeWin64Subpath);
            if (Directory.Exists(agpUsmapDir))
            {
                dialog.InitialDirectory = agpUsmapDir;
            }
            else if (Directory.Exists(ue4ssDir))
            {
                dialog.InitialDirectory = ue4ssDir;
            }
            else if (Directory.Exists(win64Dir))
            {
                dialog.InitialDirectory = win64Dir;
            }
        }

        if (dialog.ShowDialog(this) != DialogResult.OK)
        {
            return;
        }

        _selectedUsmap = MappingDiscovery.CreateFromExistingFile(dialog.FileName, "manual");
        _usmapCandidateCount = 1;
        _usmapManuallySelected = true;
        _usmapStatusKind = UsmapStatusKind.ManualSelected;
        _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
        _usmapStatusLabel.Font = _boldUiFont;
        _usmapStatusLabel.Text = L.Format("Gui_UsmapManualStatus", MappingDiscovery.FormatUsmapDisplay(_selectedUsmap));
        _usmapPathTextBox.Text = _selectedUsmap.FullPath;
        _fetchUsmapButton.Visible = false;
    }

    private void OnOpenSettingsClick(object? sender, EventArgs e)
    {
        if (_isExporting)
        {
            return;
        }

        using var dialog = new SettingsDialog(
            _agpResolution,
            _uiFont,
            _boldUiFont);
        dialog.ShowDialog(this);
    }

    private async void OnCleanupUe4ssClick(object? sender, EventArgs e)
    {
        if (_isExporting)
        {
            return;
        }

        try
        {
            await StartCleanupUe4ssWorkflowAsync();
        }
        catch (Exception exception)
        {
            AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_CleanupUnexpectedLog", exception.Message)));
            _startupLog?.WriteException("OnCleanupUe4ssClick", exception);
            MessageBox.Show(
                this,
                L.Format("Gui_CleanupFailedBody", exception.Message),
                L.Get("Gui_CleanupFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
    }

    private async Task StartCleanupUe4ssWorkflowAsync()
    {
        var cliLookup = GuiPaths.ResolveCliExecutable();
        if (!cliLookup.Found)
        {
            MessageBox.Show(
                this,
                cliLookup.ErrorMessage ?? L.Get("Gui_CliMissingFallback"),
                L.Get("Gui_CliMissingTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                L.Get("Gui_NoGameSelectedBody"),
                L.Get("Gui_NoGameSelectedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? L.Get("Gui_GameDirInvalidBody"),
                L.Get("Gui_GameDirInvalidTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        if (Ue4ssRuntimeSupport.IsGameProcessRunning())
        {
            MessageBox.Show(
                this,
                L.Format("Gui_GameRunningCleanupBody", Ue4ssRuntimeSupport.GameExeFileName),
                L.Get("Gui_GameRunningTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameDir = _selectedGame.GameDirectory;
        var plan = Ue4ssRuntimeSupport.InspectCleanupPlan(gameDir);
        if (!plan.HasAnythingDetected)
        {
            MessageBox.Show(
                this,
                L.Get("Ue4ss_UninstallNotFound"),
                L.Get("Gui_Ue4ssNotFoundTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Information);
            return;
        }

        using var cleanupDialog = new Ue4ssCleanupDialog(
            gameDir,
            plan,
            _activeAgpRoot,
            _uiFont,
            _boldUiFont);
        if (cleanupDialog.ShowDialog(this) != DialogResult.OK || !cleanupDialog.Confirmed)
        {
            return;
        }

        if (Ue4ssRuntimeSupport.IsGameProcessRunning())
        {
            MessageBox.Show(
                this,
                L.Format("Gui_GameRunningCleanupBody", Ue4ssRuntimeSupport.GameExeFileName),
                L.Get("Gui_GameRunningTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var cliArgs = BuildCliCommand.BuildUninstallUe4ssArguments(
            gameDir,
            includeForeign: cleanupDialog.IncludeForeign,
            yes: true,
            language: Localization.CurrentLanguage);
        AppendLogLine(L.Format("Gui_LaunchCommandLog", cliLookup.FileName, string.Join(" ", cliArgs.Select(QuoteForLog))));
        _startupLog?.WriteInfo(L.Format("Gui_LogStartCleanup", gameDir, cleanupDialog.IncludeForeign));
        _progressBar.Value = 20;
        _progressDisplayKind = ProgressDisplayKind.Raw;
        _progressLabel.Text = L.Get("Gui_StatusCleaningUe4ss");

        var stdoutLines = new List<string>();
        var stderrLines = new List<string>();

        _exportCts?.Dispose();
        _exportCts = new CancellationTokenSource();
        var token = _exportCts.Token;

        SetOperationState(ActiveOperation.CleaningUe4ss);
        try
        {
            var exitCode = await BuildCliCommand.RunAsync(
                cliLookup.ExpectedPath,
                cliArgs,
                line =>
                {
                    lock (stdoutLines)
                    {
                        stdoutLines.Add(line);
                    }

                    OnChildStdoutLine(line);
                },
                line =>
                {
                    lock (stderrLines)
                    {
                        stderrLines.Add(line);
                    }

                    OnChildStderrLine(line);
                },
                token,
                agpHomeOverride: _activeAgpRoot);

            if (exitCode == 0)
            {
                _progressBar.Value = 100;
                _progressDisplayKind = ProgressDisplayKind.CleanupComplete;
                _progressLabel.Text = L.Get("Gui_StatusCleanupComplete");
                var summary = stdoutLines.Count > 0
                    ? string.Join(Environment.NewLine, stdoutLines)
                    : L.Get("Gui_CleanupCompleteDefaultBody");
                _startupLog?.WriteInfo(L.Format("Gui_LogCleanupComplete", GuiStartupLog.SanitizeSingleLine(summary)));
                MessageBox.Show(
                    this,
                    summary,
                    L.Get("Gui_CleanupCompleteTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Information);
            }
            else
            {
                _progressBar.Value = 0;
                _progressDisplayKind = ProgressDisplayKind.CleanupFailedExit;
                _lastProgressExitCode = exitCode;
                _progressLabel.Text = L.Format("Gui_StatusCleanupFailedExit", exitCode);
                var errDetail = stderrLines.Count > 0
                    ? string.Join(Environment.NewLine, stderrLines)
                    : L.Format("Gui_CleanupFailedExitBody", exitCode);
                _startupLog?.WriteWarning(L.Format("Gui_LogCleanupFailedExit", exitCode, errDetail));
                MessageBox.Show(
                    this,
                    errDetail,
                    L.Get("Gui_CleanupFailedTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressDisplayKind = ProgressDisplayKind.CleanupCancelled;
            _progressLabel.Text = L.Get("Gui_StatusCleanupCancelled");
            AppendLogLine(L.Format("Gui_LogPrefixCancelled", L.Get("Gui_CleanupCancelledLog")));
        }
        finally
        {
            SetOperationState(ActiveOperation.None);
            var selectedUsmapMissing = _selectedUsmap is null
                || !File.Exists(PathSupport.ToIoPath(_selectedUsmap.FullPath));
            RefreshMappingsForSelectedGame(resetManualUsmap: selectedUsmapMissing);
        }
    }

    private async void OnFetchUsmapClick(object? sender, EventArgs e)
    {
        if (_isExporting)
        {
            return;
        }

        try
        {
            await StartFetchUsmapWorkflowAsync();
        }
        catch (Exception exception)
        {
            AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_FetchUsmapUnexpectedLog", exception.Message)));
            MessageBox.Show(
                this,
                L.Format("Gui_FetchUsmapErrorBody", exception.Message),
                L.Get("Gui_FetchUsmapFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
    }

    private async Task StartFetchUsmapWorkflowAsync()
    {
        var cliLookup = GuiPaths.ResolveCliExecutable();
        if (!cliLookup.Found)
        {
            MessageBox.Show(
                this,
                cliLookup.ErrorMessage,
                L.Get("Gui_CliMissingTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                L.Get("Gui_NoGameSelectedBody"),
                L.Get("Gui_NoGameSelectedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? L.Get("Gui_GameDirInvalidBody"),
                L.Get("Gui_GameDirInvalidTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        if (Ue4ssRuntimeSupport.IsGameProcessRunning())
        {
            MessageBox.Show(
                this,
                L.Format("Gui_GameRunningFetchBody", Ue4ssRuntimeSupport.GameExeFileName),
                L.Get("Gui_GameRunningTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var confirmText = L.Get("Gui_FetchUsmapConfirmBody");

        var answer = MessageBox.Show(
            this,
            confirmText,
            L.Get("Gui_FetchUsmapConfirmTitle"),
            MessageBoxButtons.YesNo,
            MessageBoxIcon.Question,
            MessageBoxDefaultButton.Button1);
        if (answer != DialogResult.Yes)
        {
            return;
        }

        var gameDir = _selectedGame.GameDirectory;
        var cliArgs = BuildCliCommand.BuildFetchUsmapArguments(
            gameDir,
            yes: true,
            language: Localization.CurrentLanguage,
            progressJson: true);

        _lastFetchedUsmapPath = null;
        _logTextBox.Clear();
        AppendLogLine(L.Format("Gui_LaunchCommandLog", cliLookup.FileName, string.Join(" ", cliArgs.Select(QuoteForLog))));
        _progressBar.Value = 5;
        _progressDisplayKind = ProgressDisplayKind.Raw;
        _progressLabel.Text = L.Get("Gui_StatusPreparingFetchUsmap");

        _exportCts?.Dispose();
        _exportCts = new CancellationTokenSource();
        var token = _exportCts.Token;

        SetOperationState(ActiveOperation.FetchingUsmap);
        try
        {
            var exitCode = await BuildCliCommand.RunAsync(
                cliLookup.ExpectedPath,
                cliArgs,
                OnChildStdoutLine,
                OnChildStderrLine,
                token,
                agpHomeOverride: _activeAgpRoot);

            if (exitCode == 0)
            {
                _progressBar.Value = 100;
                _progressDisplayKind = ProgressDisplayKind.FetchComplete;
                _progressLabel.Text = L.Get("Gui_StatusFetchUsmapComplete");

                if (!string.IsNullOrWhiteSpace(_lastFetchedUsmapPath)
                    && File.Exists(PathSupport.ToIoPath(_lastFetchedUsmapPath)))
                {
                    _selectedUsmap = MappingDiscovery.CreateFromExistingFile(_lastFetchedUsmapPath, "fetched");
                    _usmapCandidateCount = 1;
                    _usmapManuallySelected = false;
                    _usmapStatusKind = UsmapStatusKind.Fetched;
                    _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                    _usmapStatusLabel.Font = _boldUiFont;
                    _usmapStatusLabel.Text = L.Format("Gui_UsmapFetchedStatus", MappingDiscovery.FormatUsmapDisplay(_selectedUsmap));
                    _usmapPathTextBox.Text = _selectedUsmap.FullPath;
                    _fetchUsmapButton.Visible = false;
                }
                else
                {
                    RefreshMappingsForSelectedGame(resetManualUsmap: true);
                }

                AppendLogLine(L.Format("Gui_LogPrefixDone", L.Get("Gui_FetchUsmapDoneLog")));
            }
            else
            {
                _progressDisplayKind = ProgressDisplayKind.FetchFailedExit;
                _lastProgressExitCode = exitCode;
                _progressLabel.Text = L.Format("Gui_StatusFetchUsmapFailedExit", exitCode);
                AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_FetchUsmapFailedExitLog", exitCode)));
                MessageBox.Show(
                    this,
                    L.Format("Gui_FetchUsmapFailedExitBody", exitCode),
                    L.Get("Gui_FetchUsmapFailedTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressDisplayKind = ProgressDisplayKind.Raw;
            _progressLabel.Text = L.Get("Gui_StatusCancellingFetchUsmap");
            AppendLogLine(L.Format("Gui_LogPrefixCancelled", L.Get("Gui_FetchUsmapCancellingLog")));

            await Task.Run(async () =>
            {
                Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(
                    gameDir,
                    msg => PostLogLineSafely(msg));
                await Ue4ssRuntimeSupport.CloseGameProcessesAsync(
                    TimeSpan.FromSeconds(15),
                    msg => PostLogLineSafely(msg)).ConfigureAwait(false);
            });

            _progressDisplayKind = ProgressDisplayKind.FetchCancelled;
            _progressLabel.Text = L.Get("Gui_StatusFetchUsmapCancelled");
            AppendLogLine(L.Format("Gui_LogPrefixCancelled", L.Get("Gui_FetchUsmapCancelledDoneLog")));
        }
        catch (Exception exception)
        {
            _progressDisplayKind = ProgressDisplayKind.FetchFailed;
            _progressLabel.Text = L.Get("Gui_StatusFetchUsmapFailed");
            AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_FetchUsmapErrorBody", exception.Message)));
            MessageBox.Show(
                this,
                L.Format("Gui_FetchUsmapErrorBody", exception.Message),
                L.Get("Gui_FetchUsmapFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
        finally
        {
            SetOperationState(ActiveOperation.None);
        }
    }

    private void OnBrowseOutputClick(object? sender, EventArgs e)
    {
        using var dialog = new FolderBrowserDialog
        {
            Description = L.Get("Gui_BrowseOutputDialogDesc"),
            UseDescriptionForTitle = true,
            ShowNewFolderButton = true,
        };
        var currentPath = GuiPaths.CleanPathInput(_outputDirTextBox.Text);
        if (!string.IsNullOrWhiteSpace(currentPath) && Directory.Exists(currentPath))
        {
            dialog.InitialDirectory = currentPath;
        }
        else if (Directory.Exists(_outputRootDirectory))
        {
            dialog.InitialDirectory = _outputRootDirectory;
        }

        if (dialog.ShowDialog(this) != DialogResult.OK)
        {
            return;
        }

        SetOutputDirectoryText(dialog.SelectedPath, userCustomized: true);
        _outputHintIsDefault = true;
        _outputHintLabel.ForeColor = SystemColors.GrayText;
        _outputHintLabel.Text = FormatDefaultOutputHintText();
        var gameCode = _selectedGame?.GameCode ?? SteamLocator.ResonanceGame.Code;
        var extractedRoot = GuiPaths.ExtractOutputRootDirectory(dialog.SelectedPath, gameCode);
        if (!string.IsNullOrWhiteSpace(extractedRoot))
        {
            _outputRootDirectory = extractedRoot;
        }

        SaveSettingsSafely();
    }

    private async void OnStartExportClick(object? sender, EventArgs e)
    {
        if (_isExporting)
        {
            return;
        }

        try
        {
            await StartExportWorkflowAsync();
        }
        catch (Exception exception)
        {
            AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_ExportUnexpectedLog", exception.Message)));
            MessageBox.Show(
                this,
                L.Format("Gui_ExportErrorBody", exception.Message),
                L.Get("Gui_ExportFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
    }

    private async Task StartExportWorkflowAsync()
    {
        // 1. Ensure sibling ffr-export.exe exists.
        var cliLookup = GuiPaths.ResolveCliExecutable();
        if (!cliLookup.Found)
        {
            MessageBox.Show(
                this,
                cliLookup.ErrorMessage,
                L.Get("Gui_CliMissingTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        // 2. Validate selected game.
        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                L.Get("Gui_NoGameSelectedBody"),
                L.Get("Gui_NoGameSelectedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? L.Get("Gui_GameDirInvalidBody"),
                L.Get("Gui_GameDirInvalidTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        // 3. Validate .usmap.
        if (_selectedUsmap is null || !File.Exists(PathSupport.ToIoPath(_selectedUsmap.FullPath)))
        {
            MessageBox.Show(
                this,
                L.Format("Gui_MissingUsmapBody", MappingDiscovery.UsmapNotFoundGuidance),
                L.Get("Gui_MissingUsmapTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        // 4. Inspect output directory and confirm before overwriting existing files.
        var inspection = GuiPaths.InspectOutputDirectory(_outputDirTextBox.Text, _selectedGame.GameDirectory);
        var useForce = false;

        switch (inspection.Status)
        {
            case OutputDirectoryStatus.InvalidPath:
            case OutputDirectoryStatus.OverlapsGameDirectory:
            case OutputDirectoryStatus.TargetIsFile:
                MessageBox.Show(
                    this,
                    inspection.ErrorMessage ?? L.Get("Gui_OutputDirInvalidFallback"),
                    L.Get("Gui_OutputDirInvalidTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
                return;

            case OutputDirectoryStatus.ExistsPreviousBuild:
                {
                    var confirmRebuild = MessageBox.Show(
                        this,
                        L.Format("Gui_OverwritePrevBuildBody", inspection.NormalizedPath),
                        L.Get("Gui_OverwritePrevBuildTitle"),
                        MessageBoxButtons.YesNo,
                        MessageBoxIcon.Question,
                        MessageBoxDefaultButton.Button1);
                    if (confirmRebuild != DialogResult.Yes)
                    {
                        return;
                    }

                    useForce = true;
                    break;
                }

            case OutputDirectoryStatus.ExistsForeignNonEmpty:
                {
                    if (inspection.IsProtectedDirectory)
                    {
                        MessageBox.Show(
                            this,
                            inspection.ErrorMessage ?? L.Get("Gui_ProtectedDirFallback"),
                            L.Get("Gui_CannotUseOutputDirTitle"),
                            MessageBoxButtons.OK,
                            MessageBoxIcon.Error);
                        return;
                    }

                    var confirmForeign = MessageBox.Show(
                        this,
                        L.Format("Gui_OverwriteForeignDirBody", inspection.EntryCount, inspection.NormalizedPath),
                        L.Get("Gui_OverwriteForeignDirTitle"),
                        MessageBoxButtons.YesNo,
                        MessageBoxIcon.Warning,
                        MessageBoxDefaultButton.Button2);
                    if (confirmForeign != DialogResult.Yes)
                    {
                        return;
                    }

                    try
                    {
                        GuiPaths.ClearDirectoryContents(inspection.NormalizedPath);
                    }
                    catch (Exception exception)
                    {
                        MessageBox.Show(
                            this,
                            L.Format("Gui_ClearDirFailedBody", inspection.NormalizedPath, exception.Message),
                            L.Get("Gui_ClearDirFailedTitle"),
                            MessageBoxButtons.OK,
                            MessageBoxIcon.Error);
                        return;
                    }

                    break;
                }
        }

        // Persist user settings before starting.
        var extractedRoot = GuiPaths.ExtractOutputRootDirectory(inspection.NormalizedPath, _selectedGame.GameCode);
        if (!string.IsNullOrWhiteSpace(extractedRoot))
        {
            _outputRootDirectory = extractedRoot;
        }

        SaveSettingsSafely();

        var request = new BuildCommandRequest(
            _selectedGame.GameDirectory,
            _selectedUsmap.FullPath,
            inspection.NormalizedPath,
            _selectedGame.SteamBuildId,
            useForce,
            Language: Localization.CurrentLanguage,
            ProgressJson: true);
        var cliArgs = BuildCliCommand.BuildArguments(request);

        _resultPanelKind = ResultPanelKind.Exporting;
        ResetResultPanel(L.Get("Gui_ResultExporting"), L.Get("Gui_ResultExportingDetail"));
        _logTextBox.Clear();
        AppendLogLine(L.Format("Gui_LaunchCommandLog", cliLookup.FileName, string.Join(" ", cliArgs.Select(QuoteForLog))));
        _progressBar.Value = 5;
        _progressDisplayKind = ProgressDisplayKind.Raw;
        _progressLabel.Text = L.Get("Gui_StatusStartingExport");

        _exportCts?.Dispose();
        _exportCts = new CancellationTokenSource();
        var token = _exportCts.Token;

        SetOperationState(ActiveOperation.Exporting);
        try
        {
            var exitCode = await BuildCliCommand.RunAsync(
                cliLookup.ExpectedPath,
                cliArgs,
                OnChildStdoutLine,
                OnChildStderrLine,
                token,
                agpHomeOverride: _activeAgpRoot);

            if (exitCode is 0 or 1)
            {
                _progressBar.Value = 100;
                _progressDisplayKind = exitCode == 0
                    ? ProgressDisplayKind.ExportComplete
                    : ProgressDisplayKind.ExportCompletePartial;
                _progressLabel.Text = exitCode == 0
                    ? L.Get("Gui_StatusExportComplete")
                    : L.Get("Gui_StatusExportCompletePartial");

                try
                {
                    var report = BuildReportReader.ReadFromOutputDirectory(inspection.NormalizedPath);
                    DisplayBuildReport(report, inspection.NormalizedPath);
                }
                catch (Exception reportException)
                {
                    _resultPanelKind = ResultPanelKind.Raw;
                    ResetResultPanel(L.Get("Gui_ResultReadReportFailed"), reportException.Message);
                    _lastCompletedOutputDir = inspection.NormalizedPath;
                    _openOutputButton.Enabled = Directory.Exists(inspection.NormalizedPath);
                    AppendLogLine(L.Format("Gui_LogPrefixError", reportException.Message));
                }
            }
            else
            {
                _progressDisplayKind = ProgressDisplayKind.ExportFailedExit;
                _lastProgressExitCode = exitCode;
                _progressLabel.Text = L.Format("Gui_StatusExportFailedExit", exitCode);
                _resultPanelKind = ResultPanelKind.ExportFailedExit;
                _lastResultExitCode = exitCode;
                ResetResultPanel(
                    L.Get("Gui_ResultExportFailed"),
                    L.Format("Gui_ResultExportFailedDetail", exitCode));
                AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_ExportFailedExitLog", exitCode)));
                MessageBox.Show(
                    this,
                    L.Format("Gui_ExportFailedExitBody", exitCode),
                    L.Get("Gui_ExportFailedTitle"),
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressDisplayKind = ProgressDisplayKind.ExportCancelled;
            _progressLabel.Text = L.Get("Gui_StatusCancelled");
            _resultPanelKind = ResultPanelKind.Cancelled;
            ResetResultPanel(L.Get("Gui_ResultCancelled"), L.Get("Gui_ResultCancelledDetail"));
            AppendLogLine(L.Format("Gui_LogPrefixCancelled", L.Get("Gui_ExportCancelledLog")));
        }
        catch (Exception exception)
        {
            _progressDisplayKind = ProgressDisplayKind.ExportFailed;
            _progressLabel.Text = L.Get("Gui_StatusExportFailed");
            _resultPanelKind = ResultPanelKind.Raw;
            ResetResultPanel(L.Get("Gui_ResultExportFailed"), exception.Message);
            AppendLogLine(L.Format("Gui_LogPrefixError", L.Format("Gui_ExportExecFailedLog", exception.Message)));
            MessageBox.Show(
                this,
                L.Format("Gui_ExportExecFailedBody", exception.Message),
                L.Get("Gui_ExportFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
        finally
        {
            SetOperationState(ActiveOperation.None);
        }
    }

    private void OnCancelExportClick(object? sender, EventArgs e)
    {
        if (!_isExporting)
        {
            return;
        }

        _cancelButton.Enabled = false;
        _progressDisplayKind = ProgressDisplayKind.Raw;
        _progressLabel.Text = L.Get("Gui_StatusCancellingExport");
        AppendLogLine(L.Format("Gui_LogPrefixInfo", L.Get("Gui_ExportCancellingLog")));
        try
        {
            _exportCts?.Cancel();
        }
        catch
        {
            // Ignore cancellation race if already completed.
        }
    }

    private void OnChildStdoutLine(string line)
    {
        if (IsDisposed || !IsHandleCreated)
        {
            return;
        }

        try
        {
            BeginInvoke(() =>
            {
                if (IsDisposed)
                {
                    return;
                }

                if (BuildCliCommand.TryParseFetchedUsmapPath(line, out var savedUsmap))
                {
                    _lastFetchedUsmapPath = savedUsmap;
                }

                if (BuildCliCommand.TryParseStepProgress(line, out var step))
                {
                    _progressBar.Value = Math.Clamp(step.Percent, _progressBar.Minimum, _progressBar.Maximum);
                    _progressDisplayKind = ProgressDisplayKind.Raw;
                    _progressLabel.Text = string.IsNullOrEmpty(step.Title)
                        ? L.Format("Gui_StatusStep", step.CurrentStep, step.TotalSteps)
                        : L.Format("Gui_StatusStepWithTitle", step.CurrentStep, step.TotalSteps, step.Title);
                }

                // Keep machine-readable @@progress / @@usmap lines out of the human-readable log window.
                if (!ProgressProtocol.IsMachineProtocolLine(line))
                {
                    AppendLogLine(line);
                }
            });
        }
        catch (ObjectDisposedException)
        {
            // Form closed while background output was flushing.
        }
        catch (InvalidOperationException)
        {
            // Form handle destroyed during shutdown.
        }
    }

    private void OnChildStderrLine(string line)
    {
        if (IsDisposed || !IsHandleCreated)
        {
            return;
        }

        try
        {
            BeginInvoke(() =>
            {
                if (IsDisposed)
                {
                    return;
                }

                AppendLogLine($"[stderr] {line}");
            });
        }
        catch (ObjectDisposedException)
        {
            // Form closed while background output was flushing.
        }
        catch (InvalidOperationException)
        {
            // Form handle destroyed during shutdown.
        }
    }

    private void DisplayBuildReport(BuildReportSummary report, string outputDirectory)
    {
        _lastReport = report;
        _lastCompletedOutputDir = outputDirectory;
        _resultPanelKind = ResultPanelKind.Report;
        _copyFeedbackLabel.Text = string.Empty;

        var (symbol, title, explanation) = BuildReportReader.FormatVerdictDisplay(
            report.Verdict,
            report.MatchedRegisteredAt,
            report.FileSha256MismatchOnly,
            report.TableDiffCount,
            report.HasPartialExportFailure);

        _verdictLabel.Text = $"{symbol} {title}";
        _verdictLabel.ForeColor = report.Verdict switch
        {
            BuildComparisonVerdict.Matched => Color.FromArgb(0, 128, 0),
            BuildComparisonVerdict.NewVersion => Color.FromArgb(180, 100, 0),
            _ => Color.FromArgb(185, 28, 28),
        };

        var elapsedSuffix = report.TotalElapsedMs.HasValue
            ? L.Format("Gui_ResultElapsedSuffix", (report.TotalElapsedMs.Value / 1000.0).ToString("F2", CultureInfo.InvariantCulture))
            : string.Empty;
        _verdictDetailLabel.ForeColor = SystemColors.ControlText;
        _verdictDetailLabel.Text = explanation + elapsedSuffix;

        _dbPathTextBox.Text = report.DatabasePath;
        _fileShaTextBox.Text = report.FileSha256;
        _contentShaTextBox.Text = report.ContentSha256 ?? L.Get("Common_None");

        _openOutputButton.Enabled = Directory.Exists(outputDirectory);
        _copyHashButton.Enabled = true;
    }

    private void ResetResultPanel(string headline, string detail)
    {
        _lastReport = null;
        _copyFeedbackLabel.Text = string.Empty;
        _verdictLabel.Text = headline;
        _verdictLabel.ForeColor = headline.StartsWith("✗", StringComparison.Ordinal)
            ? Color.FromArgb(185, 28, 28)
            : SystemColors.GrayText;
        _verdictDetailLabel.ForeColor = SystemColors.GrayText;
        _verdictDetailLabel.Text = detail;
        _dbPathTextBox.Text = string.Empty;
        _fileShaTextBox.Text = string.Empty;
        _contentShaTextBox.Text = string.Empty;
        _openOutputButton.Enabled = false;
        _copyHashButton.Enabled = false;
    }

    private void OnOpenOutputFolderClick(object? sender, EventArgs e)
    {
        var dir = _lastCompletedOutputDir;
        if (string.IsNullOrWhiteSpace(dir) || !Directory.Exists(dir))
        {
            MessageBox.Show(
                this,
                L.Get("Gui_OpenOutputMissingBody"),
                L.Get("Gui_OpenOutputFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        try
        {
            Process.Start(new ProcessStartInfo
            {
                FileName = dir,
                UseShellExecute = true,
            });
        }
        catch (Exception exception)
        {
            MessageBox.Show(
                this,
                L.Format("Gui_OpenOutputFailedBody", dir, exception.Message),
                L.Get("Gui_OpenOutputFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
    }

    private void OnCopyHashClick(object? sender, EventArgs e)
    {
        if (_lastReport is null)
        {
            return;
        }

        try
        {
            var text = BuildReportReader.FormatClipboardHashText(_lastReport);
            Clipboard.SetText(text);
            _copyFeedbackLabel.Text = L.Get("Gui_CopiedFeedback");
        }
        catch (Exception exception)
        {
            MessageBox.Show(
                this,
                L.Format("Gui_CopyFailedBody", exception.Message),
                L.Get("Gui_CopyFailedTitle"),
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
        }
    }

    private void SetOperationState(ActiveOperation operation)
    {
        _activeOperation = operation;
        var busy = operation != ActiveOperation.None;

        _languageComboBox.Enabled = !busy;
        _gameListView.Enabled = !busy;
        _manualGameButton.Enabled = !busy;
        _rescanGameButton.Enabled = !busy;
        _settingsButton.Enabled = !busy;
        _browseUsmapButton.Enabled = !busy;
        _cleanupUe4ssButton.Enabled = !busy;
        _fetchUsmapButton.Enabled = !busy;
        _outputDirTextBox.Enabled = !busy;
        _browseOutputButton.Enabled = !busy;
        _startButton.Enabled = !busy;
        _cancelButton.Enabled = busy;
    }

    private void PostLogLineSafely(string line)
    {
        if (IsDisposed || !IsHandleCreated)
        {
            return;
        }

        try
        {
            BeginInvoke(() =>
            {
                if (!IsDisposed)
                {
                    AppendLogLine(line);
                }
            });
        }
        catch (ObjectDisposedException)
        {
            // Ignore shutdown races.
        }
        catch (InvalidOperationException)
        {
            // Ignore shutdown races.
        }
    }

    private void UpdateDefaultOutputDirectory(string gameCode, string? steamBuildId)
    {
        try
        {
            var result = GuiPaths.BuildDefaultOutputDirectorySafe(
                _outputRootDirectory,
                gameCode,
                steamBuildId,
                agpHomeOverride: _activeAgpRoot);
            SetOutputDirectoryText(result.Path, userCustomized: false);
            if (!string.IsNullOrWhiteSpace(result.WarningMessage))
            {
                _outputHintIsDefault = false;
                _outputHintLabel.ForeColor = Color.FromArgb(180, 100, 0);
                _outputHintLabel.Text = result.WarningMessage;
                _startupLog?.WriteWarning(result.WarningMessage);
                AppendLogLine(L.Format("Gui_LogPrefixInfo", result.WarningMessage));
            }
        }
        catch (Exception exception)
        {
            _outputHintIsDefault = false;
            var errorMessage = L.Format("Gui_CalcDefaultOutputFailed", exception.Message);
            _outputHintLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _outputHintLabel.Text = errorMessage;
            _startupLog?.WriteWarning(errorMessage);
            AppendLogLine(L.Format("Gui_LogPrefixInfo", errorMessage));
        }
    }

    private void SetOutputDirectoryText(string value, bool userCustomized)
    {
        _suppressOutputTextChanged = true;
        _outputDirTextBox.Text = value;
        _suppressOutputTextChanged = false;
        _userCustomizedOutputDir = userCustomized;
    }

    private void AppendLogLine(string line)
    {
        if (_logTextBox.TextLength > MaxLogCharacters)
        {
            _logTextBox.Text = _logTextBox.Text[^((MaxLogCharacters * 3) / 4)..];
        }

        _logTextBox.AppendText(line + Environment.NewLine);
    }

    private void AdjustGameListColumnWidths()
    {
        if (_gameListView.Columns.Count < 3 || _gameListView.ClientSize.Width <= 0)
        {
            return;
        }

        var available = _gameListView.ClientSize.Width;
        var nameWidth = Math.Max(180, (int)(available * 0.30));
        var buildWidth = Math.Max(105, (int)(available * 0.16));
        var dirWidth = Math.Max(180, available - nameWidth - buildWidth - 4);
        _gameListView.Columns[0].Width = nameWidth;
        _gameListView.Columns[1].Width = buildWidth;
        _gameListView.Columns[2].Width = dirWidth;
    }

    private void SaveSettingsSafely()
    {
        try
        {
            var settings = new GuiSettings(
                _outputRootDirectory,
                Localization.CurrentLanguage);
            GuiSettingsStore.Save(settings, _settingsPath);
        }
        catch
        {
            // Ignore settings write errors so UI shutdown or export is never blocked.
        }
    }

    private static TextBox CreateReadOnlyField(Font font)
    {
        return new TextBox
        {
            Dock = DockStyle.Fill,
            ReadOnly = true,
            Font = font,
            BackColor = SystemColors.ControlLightLight,
            Margin = new Padding(0, 2, 8, 4),
        };
    }

    private Font CreateUiFont(float size, FontStyle style, List<string> startupWarnings)
    {
        var result = GuiFontFallback.ResolveUiFont(
            preferredFamily: "Microsoft YaHei UI",
            size: size,
            isBold: style.HasFlag(FontStyle.Bold),
            tryCreatePreferred: (family, pt, bold) =>
                TryCreateInstalledFont(family, pt, bold ? FontStyle.Bold : FontStyle.Regular),
            tryCreateMessageBoxFont: TryCreateMessageBoxFont,
            getFallbackDefaultFont: () => DefaultFont);

        if (result.IsOwned)
        {
            _ownedFonts.Add(result.Font);
        }

        if (!string.IsNullOrWhiteSpace(result.WarningMessage)
            && _recordedFontWarnings.Add(result.WarningMessage))
        {
            startupWarnings.Add(result.WarningMessage);
            _startupLog?.WriteWarning(result.WarningMessage);
        }

        return result.Font;
    }

    private Font CreateMonoFont(float size, List<string> startupWarnings)
    {
        var result = GuiFontFallback.ResolveMonoFont(
            preferredFamily: "Consolas",
            size: size,
            tryCreatePreferred: (family, pt) => TryCreateInstalledFont(family, pt, FontStyle.Regular),
            tryCreateGenericMonospace: TryCreateGenericMonospaceFont,
            tryCreateMessageBoxFont: TryCreateMessageBoxFont,
            getFallbackDefaultFont: () => DefaultFont);

        if (result.IsOwned)
        {
            _ownedFonts.Add(result.Font);
        }

        if (!string.IsNullOrWhiteSpace(result.WarningMessage)
            && _recordedFontWarnings.Add(result.WarningMessage))
        {
            startupWarnings.Add(result.WarningMessage);
            _startupLog?.WriteWarning(result.WarningMessage);
        }

        return result.Font;
    }

    private static Font? TryCreateInstalledFont(string familyName, float size, FontStyle style)
    {
        // Construct FontFamily explicitly so a missing font throws instead of silently falling back to Microsoft Sans Serif.
        using var family = new FontFamily(familyName);
        if (!family.IsStyleAvailable(style))
        {
            return null;
        }

        return new Font(family, size, style, GraphicsUnit.Point);
    }

    private static Font? TryCreateMessageBoxFont(float size, bool isBold)
    {
        var baseFont = SystemFonts.MessageBoxFont;
        if (baseFont is null)
        {
            return null;
        }

        var style = isBold ? FontStyle.Bold : FontStyle.Regular;
        if (baseFont.FontFamily.IsStyleAvailable(style))
        {
            return new Font(baseFont.FontFamily, size, style, GraphicsUnit.Point);
        }

        return !isBold ? (Font)baseFont.Clone() : null;
    }

    private static Font? TryCreateGenericMonospaceFont(float size)
    {
        if (!FontFamily.GenericMonospace.IsStyleAvailable(FontStyle.Regular))
        {
            return null;
        }

        return new Font(FontFamily.GenericMonospace, size, FontStyle.Regular, GraphicsUnit.Point);
    }

    private static string QuoteForLog(string arg)
    {
        return arg.Contains(' ', StringComparison.Ordinal) ? $"\"{arg}\"" : arg;
    }
}
