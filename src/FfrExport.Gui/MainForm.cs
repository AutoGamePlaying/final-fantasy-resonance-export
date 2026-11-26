using System.Diagnostics;
using System.Globalization;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

/// <summary>
/// Single-window Chinese WinForms frontend for ffr-export build-db.
/// Delegates all extraction and database building to the sibling ffr-export.exe CLI.
/// </summary>
internal sealed class MainForm : Form
{
    private const int MinimumWindowWidth = 760;
    private const int MinimumWindowHeight = 740;
    private const int MaxLogCharacters = 400_000;

    private enum ActiveOperation
    {
        None,
        Exporting,
        FetchingUsmap,
        CleaningUe4ss,
    }

    private readonly GuiStartupLogSession? _startupLog;
    private readonly AgpRootResolution _agpResolution;
    private readonly string _activeAgpRoot;
    private readonly string _defaultOutputHintText;
    private readonly List<Font> _ownedFonts = [];
    private readonly HashSet<string> _recordedFontWarnings = new(StringComparer.Ordinal);

    private readonly Font _uiFont;
    private readonly Font _boldUiFont;
    private readonly Font _startButtonFont;
    private readonly Font _verdictFont;
    private readonly Font _monoFont;

    private readonly List<GameEntry> _games = [];
    private readonly List<DiscoveredMappingFile> _jmapFiles = [];

    private readonly Panel _dataRootBannerPanel;
    private readonly Label _dataRootBannerLabel;

    private readonly ListView _gameListView;
    private readonly Button _manualGameButton;
    private readonly Button _rescanGameButton;
    private readonly Button _settingsButton;
    private readonly Label _gameStatusLabel;

    private readonly Label _usmapStatusLabel;
    private readonly TextBox _usmapPathTextBox;
    private readonly Button _browseUsmapButton;
    private readonly Button _cleanupUe4ssButton;
    private readonly Button _fetchUsmapButton;
    private readonly CheckBox _includeJmapCheckBox;
    private readonly ListBox _jmapListBox;
    private readonly Button _addJmapButton;
    private readonly Button _removeJmapButton;
    private readonly Label _jmapStatusLabel;

    private readonly TextBox _outputDirTextBox;
    private readonly Button _browseOutputButton;
    private readonly Label _outputHintLabel;

    private readonly Button _startButton;
    private readonly Button _cancelButton;
    private readonly Label _progressLabel;
    private readonly ProgressBar _progressBar;
    private readonly TextBox _logTextBox;

    private readonly Label _verdictLabel;
    private readonly Label _verdictDetailLabel;
    private readonly TextBox _dbPathTextBox;
    private readonly TextBox _fileShaTextBox;
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
    private bool _usmapManuallySelected;
    private bool _userCustomizedOutputDir;
    private bool _suppressOutputTextChanged;
    private bool _suppressJmapToggle;
    private ActiveOperation _activeOperation = ActiveOperation.None;
    private bool _isExporting => _activeOperation != ActiveOperation.None;
    private CancellationTokenSource? _exportCts;
    private string? _lastFetchedUsmapPath;
    private BuildReportSummary? _lastReport;
    private string? _lastCompletedOutputDir;

    internal MainForm(GuiStartupLogSession? startupLog = null, AgpRootResolution? agpResolution = null)
    {
        _startupLog = startupLog;
        var startupWarnings = new List<string>();

        _agpResolution = agpResolution ?? AgpPaths.ResolveWritableAgpRoot();
        _activeAgpRoot = _agpResolution.AgpRoot ?? AppContext.BaseDirectory;
        var activeToolRoot = AgpPaths.ResolveToolRootForAgpRoot(_activeAgpRoot);
        var defaultOutputRoot = AgpPaths.GetDefaultOutputRootForAgpRoot(_activeAgpRoot);
        _defaultOutputHintText =
            $"默认位置：{defaultOutputRoot}\\ffr-<Steam 构建号>\\（导出只读游戏文件，不会修改游戏目录）";

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
            var migratedNote =
                $"已自动将旧版设置从 {settingsResult.MigratedFromPath} 迁移至 {_settingsPath}（旧文件已重命名为 .migrated）。";
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
                _outputRootDirectory = defaultOutputRoot;
                initialOutputWarning =
                    $"上次保存的输出目录无效（{loadedSettings.OutputRootDirectory}：{savedRootError}），已改用默认目录：{_outputRootDirectory}（可点击「浏览…」重新选择）。";
            }
        }
        else
        {
            _outputRootDirectory = defaultOutputRoot;
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

        Text = "《FINAL FANTASY RESONANCE》数据导出工具";
        Font = _uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        StartPosition = FormStartPosition.CenterScreen;
        MinimumSize = new Size(MinimumWindowWidth, MinimumWindowHeight);
        ClientSize = new Size(880, 850);

        var rootLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 6,
            Padding = new Padding(12, 10, 12, 8),
        };
        rootLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));
        rootLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));

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
            MaximumSize = new Size(830, 0),
            Dock = DockStyle.Fill,
        };
        _dataRootBannerPanel.Controls.Add(_dataRootBannerLabel);

        // Section 1: Game selection
        var gameGroup = new GroupBox
        {
            Text = "1. 游戏（FINAL FANTASY RESONANCE）",
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
        _gameListView.Columns.Add("游戏名称", 210);
        _gameListView.Columns.Add("Steam 构建号", 110);
        _gameListView.Columns.Add("安装目录", 320);
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
            Text = "手动选择游戏目录…",
            Width = 152,
            Height = 30,
            Margin = new Padding(0, 0, 0, 5),
        };
        _manualGameButton.Click += OnManualSelectGameClick;

        _rescanGameButton = new Button
        {
            Text = "重新扫描 Steam",
            Width = 152,
            Height = 30,
            Margin = new Padding(0, 0, 0, 5),
        };
        _rescanGameButton.Click += (_, _) => ScanAndPopulateGames();

        _settingsButton = new Button
        {
            Text = "设置…",
            Width = 152,
            Height = 30,
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
        gameGroup.Controls.Add(gameLayout);

        // Section 2: Resonance-specific .usmap and optional .jmap inputs
        var mappingGroup = new GroupBox
        {
            Text = "2. 映射与反射文件（.usmap / .jmap）",
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
            RowCount = 7,
        };
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        mappingLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        var usmapCaption = new Label
        {
            Text = ".usmap（必填）：",
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
            Text = "选择 .usmap…",
            Width = 112,
            Height = 30,
            Margin = new Padding(0, 0, 6, 0),
        };
        _browseUsmapButton.Click += OnBrowseUsmapClick;

        _cleanupUe4ssButton = new Button
        {
            Text = "清理 UE4SS…",
            Width = 112,
            Height = 30,
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
            Text = "一键获取 .usmap（会启动一次游戏）",
            Font = _boldUiFont,
            AutoSize = true,
            Height = 32,
            Padding = new Padding(10, 2, 10, 2),
            Visible = false,
            Margin = new Padding(0, 2, 0, 6),
        };
        _fetchUsmapButton.Click += OnFetchUsmapClick;

        var usmapPathCaption = new Label
        {
            Text = "文件路径：",
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

        _includeJmapCheckBox = new CheckBox
        {
            Text = "包含反射数据（.jmap，可选）",
            AutoSize = true,
            Margin = new Padding(0, 4, 0, 2),
        };
        _includeJmapCheckBox.CheckedChanged += OnIncludeJmapCheckedChanged;

        var jmapComboHint = new Label
        {
            Text = BuildReportReader.DualComboHint,
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(900, 0),
            Margin = new Padding(20, 0, 0, 4),
        };

        _jmapListBox = new ListBox
        {
            Dock = DockStyle.Fill,
            Height = 64,
            HorizontalScrollbar = true,
            IntegralHeight = false,
            Margin = new Padding(0, 2, 8, 4),
        };

        var jmapButtonsPanel = new FlowLayoutPanel
        {
            FlowDirection = FlowDirection.TopDown,
            WrapContents = false,
            AutoSize = true,
            Margin = new Padding(0, 2, 0, 4),
        };
        _addJmapButton = new Button
        {
            Text = "添加 .jmap…",
            Width = 122,
            Height = 29,
            Margin = new Padding(0, 0, 0, 4),
        };
        _addJmapButton.Click += OnAddJmapClick;

        _removeJmapButton = new Button
        {
            Text = "移除选中",
            Width = 122,
            Height = 29,
            Margin = new Padding(0),
        };
        _removeJmapButton.Click += OnRemoveJmapClick;
        jmapButtonsPanel.Controls.Add(_addJmapButton);
        jmapButtonsPanel.Controls.Add(_removeJmapButton);

        _jmapStatusLabel = new Label
        {
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            Margin = new Padding(0, 0, 0, 0),
        };

        mappingLayout.Controls.Add(usmapCaption, 0, 0);
        mappingLayout.Controls.Add(_usmapStatusLabel, 1, 0);
        mappingLayout.Controls.Add(usmapActionButtons, 2, 0);

        mappingLayout.Controls.Add(_fetchUsmapButton, 1, 1);
        mappingLayout.SetColumnSpan(_fetchUsmapButton, 2);

        mappingLayout.Controls.Add(usmapPathCaption, 0, 2);
        mappingLayout.Controls.Add(_usmapPathTextBox, 1, 2);
        mappingLayout.SetColumnSpan(_usmapPathTextBox, 2);

        mappingLayout.Controls.Add(_includeJmapCheckBox, 0, 3);
        mappingLayout.SetColumnSpan(_includeJmapCheckBox, 3);

        mappingLayout.Controls.Add(jmapComboHint, 0, 4);
        mappingLayout.SetColumnSpan(jmapComboHint, 3);

        mappingLayout.Controls.Add(_jmapListBox, 0, 5);
        mappingLayout.SetColumnSpan(_jmapListBox, 2);
        mappingLayout.Controls.Add(jmapButtonsPanel, 2, 5);

        mappingLayout.Controls.Add(_jmapStatusLabel, 0, 6);
        mappingLayout.SetColumnSpan(_jmapStatusLabel, 3);
        mappingGroup.Controls.Add(mappingLayout);

        // Section 3: Output directory
        var outputGroup = new GroupBox
        {
            Text = "3. 输出位置",
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

        var outputCaption = new Label
        {
            Text = "输出目录：",
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
            Text = "浏览…",
            Width = 100,
            Height = 30,
            Margin = new Padding(0, 1, 0, 4),
        };
        _browseOutputButton.Click += OnBrowseOutputClick;

        _outputHintLabel = new Label
        {
            Text = string.IsNullOrWhiteSpace(initialOutputWarning)
                ? _defaultOutputHintText
                : initialOutputWarning,
            AutoSize = true,
            ForeColor = string.IsNullOrWhiteSpace(initialOutputWarning)
                ? SystemColors.GrayText
                : Color.FromArgb(180, 100, 0),
            MaximumSize = new Size(900, 0),
            Margin = new Padding(0, 2, 0, 0),
        };

        outputLayout.Controls.Add(outputCaption, 0, 0);
        outputLayout.Controls.Add(_outputDirTextBox, 1, 0);
        outputLayout.Controls.Add(_browseOutputButton, 2, 0);
        outputLayout.Controls.Add(_outputHintLabel, 0, 1);
        outputLayout.SetColumnSpan(_outputHintLabel, 3);
        outputGroup.Controls.Add(outputLayout);

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
            Text = "开始导出",
            Font = _startButtonFont,
            Dock = DockStyle.Fill,
            Height = 42,
            Margin = new Padding(0, 0, 8, 0),
        };
        _startButton.Click += OnStartExportClick;

        _cancelButton = new Button
        {
            Text = "取消",
            Width = 112,
            Height = 42,
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
            Text = "状态：就绪",
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
        var resultGroup = new GroupBox
        {
            Text = "4. 完成结果",
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
            Text = "尚未导出",
            Font = _verdictFont,
            ForeColor = SystemColors.GrayText,
            AutoSize = true,
            Margin = new Padding(0, 2, 0, 2),
        };
        _verdictDetailLabel = new Label
        {
            Text = "点击“开始导出”后，此处将显示数据库路径、哈希及与 known-builds.json 的比对结论。",
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(900, 0),
            Margin = new Padding(0, 0, 0, 6),
        };

        var dbPathCaption = new Label
        {
            Text = "数据库路径：",
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 4),
        };
        _dbPathTextBox = CreateReadOnlyField(_uiFont);
        _openOutputButton = new Button
        {
            Text = "打开输出文件夹",
            Width = 132,
            Height = 29,
            Enabled = false,
            Margin = new Padding(0, 1, 0, 4),
        };
        _openOutputButton.Click += OnOpenOutputFolderClick;

        var fileShaCaption = new Label
        {
            Text = "文件哈希：",
            AutoSize = true,
            Anchor = AnchorStyles.Left,
            Margin = new Padding(0, 5, 6, 4),
        };
        _fileShaTextBox = CreateReadOnlyField(_monoFont);
        _copyHashButton = new Button
        {
            Text = "复制哈希",
            Width = 132,
            Height = 29,
            Enabled = false,
            Margin = new Padding(0, 1, 0, 4),
        };
        _copyHashButton.Click += OnCopyHashClick;

        var contentShaCaption = new Label
        {
            Text = "内容哈希：",
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

        resultLayout.Controls.Add(dbPathCaption, 0, 2);
        resultLayout.Controls.Add(_dbPathTextBox, 1, 2);
        resultLayout.Controls.Add(_openOutputButton, 2, 2);

        resultLayout.Controls.Add(fileShaCaption, 0, 3);
        resultLayout.Controls.Add(_fileShaTextBox, 1, 3);
        resultLayout.Controls.Add(_copyHashButton, 2, 3);

        resultLayout.Controls.Add(contentShaCaption, 0, 4);
        resultLayout.Controls.Add(_contentShaTextBox, 1, 4);
        resultLayout.Controls.Add(_copyFeedbackLabel, 2, 4);

        resultGroup.Controls.Add(resultLayout);

        rootLayout.Controls.Add(_dataRootBannerPanel, 0, 0);
        rootLayout.Controls.Add(gameGroup, 0, 1);
        rootLayout.Controls.Add(mappingGroup, 0, 2);
        rootLayout.Controls.Add(outputGroup, 0, 3);
        rootLayout.Controls.Add(runPanel, 0, 4);
        rootLayout.Controls.Add(resultGroup, 0, 5);

        _statusStrip = new StatusStrip
        {
            Dock = DockStyle.Bottom,
            SizingGrip = true,
        };
        _integrityStatusLabel = new ToolStripStatusLabel
        {
            Text = $"完整性级别：{_agpResolution.Integrity.DisplayTextZh}",
            BorderSides = ToolStripStatusLabelBorderSides.Right,
            BorderStyle = Border3DStyle.Etched,
            ForeColor = _agpResolution.Integrity.IsLowOrUntrusted
                ? Color.FromArgb(180, 100, 0)
                : SystemColors.ControlText,
        };
        _dataRootStatusLabel = new ToolStripStatusLabel
        {
            Text = $"数据目录：{activeToolRoot}",
            Spring = true,
            TextAlign = ContentAlignment.MiddleLeft,
        };
        _statusStrip.Items.Add(_integrityStatusLabel);
        _statusStrip.Items.Add(_dataRootStatusLabel);

        // Add Dock.Fill before Dock.Bottom so WinForms reserves StatusStrip height first.
        Controls.Add(rootLayout);
        Controls.Add(_statusStrip);

        // Apply persisted .jmap checkbox state without firing redundant scans before initial game discovery.
        _suppressJmapToggle = true;
        _includeJmapCheckBox.Checked = loadedSettings.IncludeJmap;
        _suppressJmapToggle = false;

        ResumeLayout(false);
        Shown += OnFormShown;

        foreach (var warning in startupWarnings)
        {
            AppendLogLine($"[提示] {warning}");
        }

        ScanAndPopulateGames();
        UpdateJmapControlsEnabledState();
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
                ActiveOperation.FetchingUsmap => "获取 .usmap",
                ActiveOperation.CleaningUe4ss => "清理 UE4SS",
                _ => "导出",
            };
            var answer = MessageBox.Show(
                this,
                $"{opName}仍在进行中。关闭窗口会取消{opName}并清理后台进程，确定要退出吗？",
                "确认退出",
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
                var errorMessage = cliLookup.ErrorMessage ?? "找不到命令行程序 ffr-export.exe。";
                AppendLogLine($"[提示] {errorMessage}");
                _startupLog?.WriteWarning(errorMessage);
                MessageBox.Show(
                    this,
                    errorMessage,
                    "缺少命令行程序",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
            }
        }
        catch (Exception exception)
        {
            _startupLog?.WriteException("OnFormShown", exception);
            AppendLogLine($"[警告] 窗口首次显示检查失败：{exception.Message}");
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
                $"自动扫描 Steam 失败：{exception.Message}；请点击右侧「手动选择游戏目录…」。");
        }

        var scanFailed = !string.IsNullOrWhiteSpace(scanResult.ErrorMessage);
        if (scanFailed)
        {
            _gameStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _gameStatusLabel.Text = scanResult.ErrorMessage!;
            _startupLog?.WriteWarning(scanResult.ErrorMessage!);
            AppendLogLine($"[提示] {scanResult.ErrorMessage}");
        }

        foreach (var game in scanResult.Games)
        {
            AddGameToListView(game);
        }

        if (_games.Count > 0)
        {
            _gameStatusLabel.ForeColor = SystemColors.GrayText;
            _gameStatusLabel.Text = $"已在 Steam 游戏库中找到 {_games.Count} 个可导出游戏。";

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
                _gameStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
                _gameStatusLabel.Text =
                    "未在 Steam 库中自动找到《FINAL FANTASY RESONANCE》，请点击右侧“手动选择游戏目录…”。";
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
        item.SubItems.Add(entry.SteamBuildId ?? "未知");
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
            Description = "选择《FINAL FANTASY RESONANCE》的游戏安装根目录（包含 FFRS 文件夹）",
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
                validation.ErrorMessage ?? "所选目录不是支持的游戏目录。",
                "游戏目录检查失败",
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
            existingItem.SubItems[1].Text = validation.Entry.SteamBuildId ?? "未知";
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

        _gameStatusLabel.ForeColor = SystemColors.GrayText;
        _gameStatusLabel.Text = $"已手动添加游戏目录：{validation.Entry.GameDirectory}";
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
                if (!string.IsNullOrWhiteSpace(usmapResult.ScanError))
                {
                    var warn = $"自动查找 .usmap 时遇到目录访问错误：{usmapResult.ScanError}";
                    _startupLog?.WriteWarning(warn);
                    AppendLogLine($"[提示] {warn}");
                }

                if (usmapResult.Selected is not null)
                {
                    _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                    _usmapStatusLabel.Font = _boldUiFont;
                    _usmapStatusLabel.Text =
                        $"已自动找到：{MappingDiscovery.FormatUsmapDisplay(usmapResult.Selected, usmapResult.Candidates.Count)}";
                    _usmapPathTextBox.Text = usmapResult.Selected.FullPath;
                    _fetchUsmapButton.Visible = false;
                }
                else
                {
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
                var errorMessage = $"自动查找 .usmap 失败：{exception.Message}；请点击「一键获取 .usmap」或「选择 .usmap…」。";
                _usmapStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
                _usmapStatusLabel.Font = _uiFont;
                _usmapStatusLabel.Text = errorMessage;
                _usmapPathTextBox.Text = string.Empty;
                _fetchUsmapButton.Visible = true;
                _startupLog?.WriteWarning(errorMessage);
                AppendLogLine($"[提示] {errorMessage}");
            }
        }

        if (_includeJmapCheckBox.Checked)
        {
            RefreshDiscoveredJmaps();
        }
        else
        {
            _jmapStatusLabel.ForeColor = SystemColors.GrayText;
            _jmapStatusLabel.Text = "未勾选（仅使用 .usmap 导出）。";
        }
    }

    private void OnBrowseUsmapClick(object? sender, EventArgs e)
    {
        using var dialog = new OpenFileDialog
        {
            Title = "选择 .usmap 映射文件",
            Filter = "UE 映射文件 (*.usmap)|*.usmap|所有文件 (*.*)|*.*",
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

        _selectedUsmap = MappingDiscovery.CreateFromExistingFile(dialog.FileName, "手动选择");
        _usmapManuallySelected = true;
        _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
        _usmapStatusLabel.Font = _boldUiFont;
        _usmapStatusLabel.Text = $"已手动选择：{MappingDiscovery.FormatUsmapDisplay(_selectedUsmap)}";
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
            AppendLogLine($"[错误] 清理 UE4SS 时发生未预期异常：{exception.Message}");
            _startupLog?.WriteException("OnCleanupUe4ssClick", exception);
            MessageBox.Show(
                this,
                $"清理 UE4SS 失败：{exception.Message}",
                "清理 UE4SS 失败",
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
                cliLookup.ErrorMessage ?? "找不到命令行程序 ffr-export.exe。",
                "缺少命令行程序",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                "请先选择一个游戏，或点击「手动选择游戏目录…」指定《FINAL FANTASY RESONANCE》安装目录。",
                "未选择游戏",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? "所选游戏目录无效，请重新选择。",
                "游戏目录无效",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        if (Ue4ssRuntimeSupport.IsGameProcessRunning())
        {
            MessageBox.Show(
                this,
                $"检测到游戏（{Ue4ssRuntimeSupport.GameExeFileName}）正在运行，请先关闭游戏后再清理 UE4SS。",
                "游戏正在运行",
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
                "未发现 UE4SS（当前游戏目录未安装 UE4SS，也未发现本工具残留文件）。",
                "未发现 UE4SS",
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
                $"检测到游戏（{Ue4ssRuntimeSupport.GameExeFileName}）正在运行，请先关闭游戏后再清理 UE4SS。",
                "游戏正在运行",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var cliArgs = BuildCliCommand.BuildUninstallUe4ssArguments(
            gameDir,
            includeForeign: cleanupDialog.IncludeForeign,
            yes: true);
        AppendLogLine($"启动命令：{cliLookup.FileName} {string.Join(" ", cliArgs.Select(QuoteForLog))}");
        _startupLog?.WriteInfo($"开始清理 UE4SS：game={gameDir}, includeForeign={cleanupDialog.IncludeForeign}");
        _progressBar.Value = 20;
        _progressLabel.Text = "状态：正在清理 UE4SS……";

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
                _progressLabel.Text = "状态：UE4SS 清理完成";
                var summary = stdoutLines.Count > 0
                    ? string.Join(Environment.NewLine, stdoutLines)
                    : "UE4SS 清理已完成。";
                _startupLog?.WriteInfo($"UE4SS 清理完成：{GuiStartupLog.SanitizeSingleLine(summary)}");
                MessageBox.Show(
                    this,
                    summary,
                    "清理 UE4SS 完成",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Information);
            }
            else
            {
                _progressBar.Value = 0;
                _progressLabel.Text = string.Create(
                    CultureInfo.InvariantCulture,
                    $"状态：清理 UE4SS 失败（退出码：{exitCode}）");
                var errDetail = stderrLines.Count > 0
                    ? string.Join(Environment.NewLine, stderrLines)
                    : $"清理 UE4SS 失败（退出码：{exitCode}）。";
                _startupLog?.WriteWarning($"UE4SS 清理失败（exit={exitCode}）：{errDetail}");
                MessageBox.Show(
                    this,
                    errDetail,
                    "清理 UE4SS 失败",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressLabel.Text = "状态：已取消清理 UE4SS";
            AppendLogLine("[已取消] 已取消清理 UE4SS。");
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
            AppendLogLine($"[错误] 一键获取 .usmap 时发生未预期异常：{exception.Message}");
            MessageBox.Show(
                this,
                $"获取 .usmap 过程发生错误：{exception.Message}",
                "获取 .usmap 失败",
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
                "缺少命令行程序",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                "请先选择一个游戏，或点击「手动选择游戏目录…」指定《FINAL FANTASY RESONANCE》安装目录。",
                "未选择游戏",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? "所选游戏目录无效，请重新选择。",
                "游戏目录无效",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        if (Ue4ssRuntimeSupport.IsGameProcessRunning())
        {
            MessageBox.Show(
                this,
                $"检测到游戏（{Ue4ssRuntimeSupport.GameExeFileName}）正在运行，请先关闭游戏后再获取 .usmap。",
                "游戏正在运行",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var confirmText =
            "即将一键获取 .usmap 映射文件，执行前请知悉：\n\n"
            + "1. 若游戏目录尚未安装 UE4SS，将从 UE4SS 官方 GitHub（RE-UE4SS v4.0.0）下载约 8.4 MB 压缩包并严格校验 SHA-256，安装进游戏目录；若您原本已安装 UE4SS，则完全不动原有配置。\n"
            + "2. 会放入临时转储模组并通过 Steam 启动一次游戏，等待引擎初始化后在标题画面转储 .usmap，随后自动移除临时模组并关闭游戏。\n"
            + "3. 转储完成后会保留 UE4SS 供下次复用，您可以随时点击主窗口「清理 UE4SS…」按钮清理本工具安装的文件。\n"
            + "4. 全程只读取类型映射，停留在标题画面，不会触碰或修改任何游戏存档。\n\n"
            + "确定要开始获取 .usmap 吗？";

        var answer = MessageBox.Show(
            this,
            confirmText,
            "确认一键获取 .usmap",
            MessageBoxButtons.YesNo,
            MessageBoxIcon.Question,
            MessageBoxDefaultButton.Button1);
        if (answer != DialogResult.Yes)
        {
            return;
        }

        var gameDir = _selectedGame.GameDirectory;
        var cliArgs = BuildCliCommand.BuildFetchUsmapArguments(gameDir, yes: true);

        _lastFetchedUsmapPath = null;
        _logTextBox.Clear();
        AppendLogLine($"启动命令：{cliLookup.FileName} {string.Join(" ", cliArgs.Select(QuoteForLog))}");
        _progressBar.Value = 5;
        _progressLabel.Text = "状态：正在准备获取 .usmap……";

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
                _progressLabel.Text = "状态：.usmap 获取完成";

                if (!string.IsNullOrWhiteSpace(_lastFetchedUsmapPath)
                    && File.Exists(PathSupport.ToIoPath(_lastFetchedUsmapPath)))
                {
                    _selectedUsmap = MappingDiscovery.CreateFromExistingFile(_lastFetchedUsmapPath, "一键获取");
                    _usmapManuallySelected = false;
                    _usmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                    _usmapStatusLabel.Font = _boldUiFont;
                    _usmapStatusLabel.Text = $"已自动获取：{MappingDiscovery.FormatUsmapDisplay(_selectedUsmap)}";
                    _usmapPathTextBox.Text = _selectedUsmap.FullPath;
                    _fetchUsmapButton.Visible = false;
                }
                else
                {
                    RefreshMappingsForSelectedGame(resetManualUsmap: true);
                }

                AppendLogLine("[完成] .usmap 已获取并自动选中，现在可以点击「开始导出」。");
            }
            else
            {
                _progressLabel.Text = string.Create(
                    CultureInfo.InvariantCulture,
                    $"状态：获取 .usmap 失败（退出码：{exitCode}）");
                AppendLogLine($"[错误] fetch-usmap 异常结束（退出码：{exitCode}）。");
                MessageBox.Show(
                    this,
                    $"获取 .usmap 失败（退出码：{exitCode}），请查看窗口日志了解具体原因和排查建议。",
                    "获取 .usmap 失败",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressLabel.Text = "状态：正在取消——停用临时模组并关闭游戏……";
            AppendLogLine("[已取消] 用户取消了获取 .usmap，正在停用临时模组并关闭游戏……");

            await Task.Run(async () =>
            {
                Ue4ssRuntimeSupport.DisableAndRemoveDumpMod(
                    gameDir,
                    msg => PostLogLineSafely(msg));
                await Ue4ssRuntimeSupport.CloseGameProcessesAsync(
                    TimeSpan.FromSeconds(15),
                    msg => PostLogLineSafely(msg)).ConfigureAwait(false);
            });

            _progressLabel.Text = "状态：已取消获取 .usmap";
            AppendLogLine("[已取消] 已停用临时转储模组并结束游戏进程。");
        }
        catch (Exception exception)
        {
            _progressLabel.Text = "状态：获取 .usmap 失败";
            AppendLogLine($"[错误] 获取 .usmap 失败：{exception.Message}");
            MessageBox.Show(
                this,
                $"获取 .usmap 失败：{exception.Message}",
                "获取 .usmap 失败",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
        finally
        {
            SetOperationState(ActiveOperation.None);
        }
    }

    private void OnIncludeJmapCheckedChanged(object? sender, EventArgs e)
    {
        UpdateJmapControlsEnabledState();
        if (_suppressJmapToggle)
        {
            return;
        }

        if (_includeJmapCheckBox.Checked)
        {
            RefreshDiscoveredJmaps();
        }
        else
        {
            _jmapStatusLabel.ForeColor = SystemColors.GrayText;
            _jmapStatusLabel.Text = "未勾选（仅使用 .usmap 导出）。";
        }

        SaveSettingsSafely();
    }

    private void RefreshDiscoveredJmaps()
    {
        _jmapFiles.Clear();
        _jmapListBox.Items.Clear();

        JmapDiscoveryResult jmapResult;
        try
        {
            jmapResult = MappingDiscovery.DiscoverJmapsWithDiagnostics(_selectedGame?.GameDirectory);
        }
        catch (Exception exception)
        {
            var errorMessage = $"自动查找 .jmap 失败：{exception.Message}；请点「添加 .jmap…」手动选择，或取消勾选继续导出。";
            _jmapStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _jmapStatusLabel.Text = errorMessage;
            _startupLog?.WriteWarning(errorMessage);
            AppendLogLine($"[提示] {errorMessage}");
            return;
        }

        foreach (var file in jmapResult.Files)
        {
            _jmapFiles.Add(file);
            _jmapListBox.Items.Add(MappingDiscovery.FormatJmapDisplay(file));
        }

        if (!string.IsNullOrWhiteSpace(jmapResult.ScanError))
        {
            var warn = $"自动查找 .jmap 时遇到目录访问错误：{jmapResult.ScanError}";
            _startupLog?.WriteWarning(warn);
            AppendLogLine($"[提示] {warn}");
        }

        if (_jmapFiles.Count > 0)
        {
            if (string.IsNullOrWhiteSpace(jmapResult.ScanError))
            {
                _jmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                _jmapStatusLabel.Text = $"已自动找到 {_jmapFiles.Count} 份 .jmap 文件（可继续添加或移除）。";
            }
            else
            {
                _jmapStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
                _jmapStatusLabel.Text =
                    $"已找到 {_jmapFiles.Count} 份 .jmap，但部分目录无法读取（{jmapResult.ScanError}），可点「添加 .jmap…」手动补充。";
            }
        }
        else if (!string.IsNullOrWhiteSpace(jmapResult.ScanError))
        {
            _jmapStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _jmapStatusLabel.Text =
                $"自动查找 .jmap 失败（{jmapResult.ScanError}）；请检查权限，或点「添加 .jmap…」手动选择。";
        }
        else
        {
            _jmapStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
            _jmapStatusLabel.Text =
                "未在游戏目录的 ue4ss / Win64 下找到 .jmap 文件，可点“添加 .jmap…”手动选择，或取消勾选继续导出。";
        }
    }

    private void OnAddJmapClick(object? sender, EventArgs e)
    {
        using var dialog = new OpenFileDialog
        {
            Title = "选择 .jmap 反射转储文件（可多选）",
            Filter = "UE 反射转储 (*.jmap)|*.jmap|所有文件 (*.*)|*.*",
            CheckFileExists = true,
            Multiselect = true,
        };
        if (_selectedGame is not null)
        {
            var ue4ssDir = SteamLocator.CombineRelativeSubpath(
                _selectedGame.GameDirectory,
                MappingDiscovery.RelativeUe4ssSubpath);
            if (Directory.Exists(ue4ssDir))
            {
                dialog.InitialDirectory = ue4ssDir;
            }
        }

        if (dialog.ShowDialog(this) != DialogResult.OK)
        {
            return;
        }

        var added = 0;
        foreach (var fileName in dialog.FileNames)
        {
            var entry = MappingDiscovery.CreateFromExistingFile(fileName, "手动添加");
            var duplicate = _jmapFiles.Any(existing =>
                string.Equals(existing.FullPath, entry.FullPath, StringComparison.OrdinalIgnoreCase));
            if (duplicate)
            {
                continue;
            }

            _jmapFiles.Add(entry);
            _jmapListBox.Items.Add(MappingDiscovery.FormatJmapDisplay(entry));
            added++;
        }

        if (added > 0 || _jmapFiles.Count > 0)
        {
            _jmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
            _jmapStatusLabel.Text = $"当前共选择 {_jmapFiles.Count} 份 .jmap 文件。";
        }
    }

    private void OnRemoveJmapClick(object? sender, EventArgs e)
    {
        var index = _jmapListBox.SelectedIndex;
        if (index < 0 || index >= _jmapFiles.Count)
        {
            return;
        }

        _jmapFiles.RemoveAt(index);
        _jmapListBox.Items.RemoveAt(index);
        if (_jmapFiles.Count == 0)
        {
            _jmapStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
            _jmapStatusLabel.Text = "当前 .jmap 列表为空，可点“添加 .jmap…”手动选择，或取消勾选继续导出。";
        }
        else
        {
            _jmapStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
            _jmapStatusLabel.Text = $"当前共选择 {_jmapFiles.Count} 份 .jmap 文件。";
        }
    }

    private void OnBrowseOutputClick(object? sender, EventArgs e)
    {
        using var dialog = new FolderBrowserDialog
        {
            Description = "选择导出输出目录",
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
        _outputHintLabel.ForeColor = SystemColors.GrayText;
        _outputHintLabel.Text = _defaultOutputHintText;
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
            AppendLogLine($"[错误] 启动导出时发生未预期异常：{exception.Message}");
            MessageBox.Show(
                this,
                $"导出过程发生错误：{exception.Message}",
                "导出失败",
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
                "缺少命令行程序",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
            return;
        }

        // 2. Validate selected game.
        if (_selectedGame is null)
        {
            MessageBox.Show(
                this,
                "请先选择一个游戏，或点击“手动选择游戏目录…”指定《FINAL FANTASY RESONANCE》安装目录。",
                "未选择游戏",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        var gameCheck = GameScanner.ValidateManualGameDirectory(_selectedGame.GameDirectory);
        if (!gameCheck.IsValid)
        {
            MessageBox.Show(
                this,
                gameCheck.ErrorMessage ?? "所选游戏目录无效，请重新选择。",
                "游戏目录无效",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        // 3. Validate .usmap.
        if (_selectedUsmap is null || !File.Exists(PathSupport.ToIoPath(_selectedUsmap.FullPath)))
        {
            MessageBox.Show(
                this,
                "请先指定有效的 .usmap 映射文件。\n\n" + MappingDiscovery.UsmapNotFoundGuidance,
                "缺少 .usmap 映射文件",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
            return;
        }

        // 4. Validate optional .jmap selection.
        var jmapPaths = new List<string>();
        if (_includeJmapCheckBox.Checked)
        {
            foreach (var file in _jmapFiles)
            {
                if (!File.Exists(PathSupport.ToIoPath(file.FullPath)))
                {
                    MessageBox.Show(
                        this,
                        $"反射数据文件不存在：{file.FullPath}\n请移除该条目或重新选择。",
                        "找不到 .jmap 文件",
                        MessageBoxButtons.OK,
                        MessageBoxIcon.Warning);
                    return;
                }

                jmapPaths.Add(file.FullPath);
            }

            if (jmapPaths.Count == 0)
            {
                var proceedWithoutJmap = MessageBox.Show(
                    this,
                    "已勾选“包含反射数据（.jmap，可选）”，但当前列表里没有 .jmap 文件。\n\n是否按“仅使用 .usmap”继续导出？",
                    "未选择 .jmap 文件",
                    MessageBoxButtons.YesNo,
                    MessageBoxIcon.Question,
                    MessageBoxDefaultButton.Button1);
                if (proceedWithoutJmap != DialogResult.Yes)
                {
                    return;
                }
            }
        }

        // 5. Inspect output directory and confirm before overwriting existing files.
        var inspection = GuiPaths.InspectOutputDirectory(_outputDirTextBox.Text, _selectedGame.GameDirectory);
        var useForce = false;

        switch (inspection.Status)
        {
            case OutputDirectoryStatus.InvalidPath:
            case OutputDirectoryStatus.OverlapsGameDirectory:
            case OutputDirectoryStatus.TargetIsFile:
                MessageBox.Show(
                    this,
                    inspection.ErrorMessage ?? "输出目录无效，请重新选择。",
                    "输出目录无效",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Warning);
                return;

            case OutputDirectoryStatus.ExistsPreviousBuild:
                {
                    var confirmRebuild = MessageBox.Show(
                        this,
                        $"目标输出目录已存在之前的导出结果：\n{inspection.NormalizedPath}\n\n是否清空该目录并重新导出？",
                        "确认覆盖已有导出",
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
                            inspection.ErrorMessage ?? "拒绝覆盖受保护目录，请选择其他子文件夹。",
                            "无法使用该输出目录",
                            MessageBoxButtons.OK,
                            MessageBoxIcon.Error);
                        return;
                    }

                    var confirmForeign = MessageBox.Show(
                        this,
                        $"输出目录已存在，且不是本工具创建的目录（当前包含 {inspection.EntryCount} 个项目）：\n"
                        + $"{inspection.NormalizedPath}\n\n"
                        + "为避免与无关文件混杂，继续导出前需要清空该目录内的所有原有文件。\n"
                        + "确定要清空并继续导出吗？（如不想删除原有文件，请点“否”并更换输出目录）",
                        "确认覆盖非本工具目录",
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
                            $"清空输出目录失败（{inspection.NormalizedPath}）：{exception.Message}",
                            "清理目录失败",
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
            jmapPaths,
            inspection.NormalizedPath,
            _selectedGame.SteamBuildId,
            useForce);
        var cliArgs = BuildCliCommand.BuildArguments(request);

        ResetResultPanel("正在导出……", "导出完成后将自动读取构建报告（*.build.json）并展示哈希与比对结论。");
        _logTextBox.Clear();
        AppendLogLine($"启动命令：{cliLookup.FileName} {string.Join(" ", cliArgs.Select(QuoteForLog))}");
        _progressBar.Value = 5;
        _progressLabel.Text = "状态：正在启动导出流程……";

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
                _progressLabel.Text = exitCode == 0
                    ? "状态：导出完成"
                    : "状态：导出完成（有少量资源包失败）";

                try
                {
                    var report = BuildReportReader.ReadFromOutputDirectory(inspection.NormalizedPath);
                    DisplayBuildReport(report, inspection.NormalizedPath);
                }
                catch (Exception reportException)
                {
                    ResetResultPanel("✗ 无法读取构建报告", reportException.Message);
                    _lastCompletedOutputDir = inspection.NormalizedPath;
                    _openOutputButton.Enabled = Directory.Exists(inspection.NormalizedPath);
                    AppendLogLine($"[错误] {reportException.Message}");
                }
            }
            else
            {
                _progressLabel.Text = string.Create(
                    CultureInfo.InvariantCulture,
                    $"状态：导出失败（退出码：{exitCode}）");
                ResetResultPanel(
                    "✗ 导出失败",
                    $"命令行程序返回退出码 {exitCode}，请查看上方日志定位具体原因。");
                AppendLogLine($"[错误] 命令行程序异常结束（退出码：{exitCode}）。");
                MessageBox.Show(
                    this,
                    $"导出失败（退出码：{exitCode}），请查看窗口中的日志信息。",
                    "导出失败",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (OperationCanceledException)
        {
            _progressBar.Value = 0;
            _progressLabel.Text = "状态：已取消";
            ResetResultPanel("已取消导出", "已终止后台导出进程。");
            AppendLogLine("[已取消] 用户已取消导出，后台进程树已终止。");
        }
        catch (Exception exception)
        {
            _progressLabel.Text = "状态：导出失败";
            ResetResultPanel("✗ 导出失败", exception.Message);
            AppendLogLine($"[错误] 执行失败：{exception.Message}");
            MessageBox.Show(
                this,
                $"启动或执行导出程序失败：{exception.Message}",
                "导出失败",
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
        _progressLabel.Text = "状态：正在取消并结束后台进程……";
        AppendLogLine("[提示] 正在取消导出并结束子进程树……");
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

                AppendLogLine(line);
                if (BuildCliCommand.TryParseFetchedUsmapPath(line, out var savedUsmap))
                {
                    _lastFetchedUsmapPath = savedUsmap;
                }

                if (BuildCliCommand.TryParseStepProgress(line, out var step))
                {
                    _progressBar.Value = Math.Clamp(step.Percent, _progressBar.Minimum, _progressBar.Maximum);
                    _progressLabel.Text = string.IsNullOrEmpty(step.Title)
                        ? string.Create(CultureInfo.InvariantCulture, $"状态：步骤 {step.CurrentStep}/{step.TotalSteps}")
                        : string.Create(
                            CultureInfo.InvariantCulture,
                            $"状态：步骤 {step.CurrentStep}/{step.TotalSteps} — {step.Title}");
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
        _copyFeedbackLabel.Text = string.Empty;

        _verdictLabel.Text = $"{report.VerdictSymbol} {report.VerdictTitle}";
        _verdictLabel.ForeColor = report.Verdict switch
        {
            BuildComparisonVerdict.Matched => Color.FromArgb(0, 128, 0),
            BuildComparisonVerdict.NewVersion => Color.FromArgb(180, 100, 0),
            _ => Color.FromArgb(185, 28, 28),
        };

        var elapsedSuffix = report.TotalElapsedMs.HasValue
            ? string.Create(CultureInfo.InvariantCulture, $"（总耗时：{report.TotalElapsedMs.Value / 1000.0:F2}s）")
            : string.Empty;
        _verdictDetailLabel.ForeColor = SystemColors.ControlText;
        _verdictDetailLabel.Text = report.VerdictExplanation + elapsedSuffix;

        _dbPathTextBox.Text = report.DatabasePath;
        _fileShaTextBox.Text = report.FileSha256;
        _contentShaTextBox.Text = report.ContentSha256 ?? "无";

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
                "输出文件夹不存在或已被移动。",
                "无法打开文件夹",
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
                $"打开输出文件夹失败（{dir}）：{exception.Message}",
                "无法打开文件夹",
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
            _copyFeedbackLabel.Text = "已复制到剪贴板";
        }
        catch (Exception exception)
        {
            MessageBox.Show(
                this,
                $"复制到剪贴板失败：{exception.Message}",
                "复制失败",
                MessageBoxButtons.OK,
                MessageBoxIcon.Warning);
        }
    }

    private void SetOperationState(ActiveOperation operation)
    {
        _activeOperation = operation;
        var busy = operation != ActiveOperation.None;

        _gameListView.Enabled = !busy;
        _manualGameButton.Enabled = !busy;
        _rescanGameButton.Enabled = !busy;
        _settingsButton.Enabled = !busy;
        _browseUsmapButton.Enabled = !busy;
        _cleanupUe4ssButton.Enabled = !busy;
        _fetchUsmapButton.Enabled = !busy;
        _includeJmapCheckBox.Enabled = !busy;
        _outputDirTextBox.Enabled = !busy;
        _browseOutputButton.Enabled = !busy;
        _startButton.Enabled = !busy;
        _cancelButton.Enabled = busy;

        UpdateJmapControlsEnabledState();
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

    private void UpdateJmapControlsEnabledState()
    {
        var enabled = !_isExporting && _includeJmapCheckBox.Checked;
        _jmapListBox.Enabled = enabled;
        _addJmapButton.Enabled = enabled;
        _removeJmapButton.Enabled = enabled;
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
                _outputHintLabel.ForeColor = Color.FromArgb(180, 100, 0);
                _outputHintLabel.Text = result.WarningMessage;
                _startupLog?.WriteWarning(result.WarningMessage);
                AppendLogLine($"[提示] {result.WarningMessage}");
            }
        }
        catch (Exception exception)
        {
            var errorMessage = $"计算默认输出目录失败：{exception.Message}；请点击「浏览…」手动选择输出目录。";
            _outputHintLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _outputHintLabel.Text = errorMessage;
            _startupLog?.WriteWarning(errorMessage);
            AppendLogLine($"[提示] {errorMessage}");
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
            var settings = new GuiSettings(_outputRootDirectory, _includeJmapCheckBox.Checked);
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
