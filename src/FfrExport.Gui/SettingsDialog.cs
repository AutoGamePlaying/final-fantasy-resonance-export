using System.Diagnostics;
using FfrExport.Gui.Core;

namespace FfrExport.Gui;

/// <summary>
/// Modal settings dialog displaying the unified %LOCALAPPDATA%\agp\ffr-export\ data directory
/// and providing the "卸载 UE4SS" action (enabled only when ffr-export's install marker exists).
/// </summary>
internal sealed class SettingsDialog : Form
{
    private readonly string? _gameDirectory;
    private readonly Action<string>? _logToMain;
    private readonly TextBox _dataDirTextBox;
    private readonly Button _openDataDirButton;
    private readonly Label _ue4ssStatusLabel;
    private readonly Label _ue4ssHintLabel;
    private readonly Button _uninstallUe4ssButton;
    private readonly Button _closeButton;
    private bool _isUninstalling;

    internal bool Ue4ssUninstalled { get; private set; }

    internal SettingsDialog(
        string? gameDirectory,
        Font uiFont,
        Font boldUiFont,
        Action<string>? logToMain = null)
    {
        _gameDirectory = gameDirectory;
        _logToMain = logToMain;

        SuspendLayout();

        Text = "设置";
        Font = uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.CenterParent;
        ClientSize = new Size(560, 290);

        var root = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 3,
            Padding = new Padding(12),
        };
        root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        // 1. Unified data directory
        var dataGroup = new GroupBox
        {
            Text = "数据目录（统一保存在 %LOCALAPPDATA%\\agp\\ffr-export\\）",
            Dock = DockStyle.Top,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            Padding = new Padding(10, 6, 10, 8),
        };
        var dataLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            AutoSize = true,
            ColumnCount = 2,
            RowCount = 2,
        };
        dataLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        dataLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _dataDirTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            ReadOnly = true,
            BackColor = SystemColors.ControlLightLight,
            Text = AgpPaths.ResolveToolRoot(),
            Margin = new Padding(0, 3, 8, 4),
        };
        _openDataDirButton = new Button
        {
            Text = "打开数据目录",
            Width = 112,
            Height = 29,
            Margin = new Padding(0, 1, 0, 4),
        };
        _openDataDirButton.Click += OnOpenDataDirClick;

        var dataNoteLabel = new Label
        {
            Text = "包含 output\\（默认导出）、usmap\\（映射文件）、downloads\\（下载缓存）、logs\\ 与 settings.json。",
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(500, 0),
            Margin = new Padding(0, 2, 0, 0),
        };

        dataLayout.Controls.Add(_dataDirTextBox, 0, 0);
        dataLayout.Controls.Add(_openDataDirButton, 1, 0);
        dataLayout.Controls.Add(dataNoteLabel, 0, 1);
        dataLayout.SetColumnSpan(dataNoteLabel, 2);
        dataGroup.Controls.Add(dataLayout);

        // 2. UE4SS management
        var ue4ssGroup = new GroupBox
        {
            Text = "UE4SS 管理（当前游戏）",
            Dock = DockStyle.Fill,
            Padding = new Padding(10, 6, 10, 8),
            Margin = new Padding(0, 8, 0, 0),
        };
        var ue4ssLayout = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 2,
            RowCount = 2,
        };
        ue4ssLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        ue4ssLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));
        ue4ssLayout.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        ue4ssLayout.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));

        _ue4ssStatusLabel = new Label
        {
            AutoSize = true,
            Font = boldUiFont,
            Anchor = AnchorStyles.Left,
            MaximumSize = new Size(370, 0),
            Margin = new Padding(0, 4, 8, 4),
        };
        _uninstallUe4ssButton = new Button
        {
            Text = "卸载 UE4SS",
            Width = 112,
            Height = 30,
            Anchor = AnchorStyles.Right,
            Enabled = false,
            Margin = new Padding(0, 2, 0, 4),
        };
        _uninstallUe4ssButton.Click += OnUninstallUe4ssClick;

        _ue4ssHintLabel = new Label
        {
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(500, 0),
            Margin = new Padding(0, 2, 0, 0),
        };

        ue4ssLayout.Controls.Add(_ue4ssStatusLabel, 0, 0);
        ue4ssLayout.Controls.Add(_uninstallUe4ssButton, 1, 0);
        ue4ssLayout.Controls.Add(_ue4ssHintLabel, 0, 1);
        ue4ssLayout.SetColumnSpan(_ue4ssHintLabel, 2);
        ue4ssGroup.Controls.Add(ue4ssLayout);

        // 3. Footer close button
        var footerPanel = new FlowLayoutPanel
        {
            Dock = DockStyle.Bottom,
            FlowDirection = FlowDirection.RightToLeft,
            AutoSize = true,
            Margin = new Padding(0, 10, 0, 0),
        };
        _closeButton = new Button
        {
            Text = "关闭",
            Width = 96,
            Height = 30,
            DialogResult = DialogResult.OK,
        };
        footerPanel.Controls.Add(_closeButton);
        CancelButton = _closeButton;
        AcceptButton = _closeButton;

        root.Controls.Add(dataGroup, 0, 0);
        root.Controls.Add(ue4ssGroup, 0, 1);
        root.Controls.Add(footerPanel, 0, 2);
        Controls.Add(root);

        ResumeLayout(false);

        RefreshUe4ssStatus();
    }

    protected override void OnFormClosing(FormClosingEventArgs e)
    {
        if (_isUninstalling)
        {
            e.Cancel = true;
            return;
        }

        base.OnFormClosing(e);
    }

    private void RefreshUe4ssStatus()
    {
        if (string.IsNullOrWhiteSpace(_gameDirectory))
        {
            _ue4ssStatusLabel.ForeColor = SystemColors.GrayText;
            _ue4ssStatusLabel.Text = "未选择游戏目录";
            _ue4ssHintLabel.Text = "请先在主窗口选择一个游戏目录后再检查 UE4SS 安装状态。";
            _uninstallUe4ssButton.Enabled = false;
            return;
        }

        try
        {
            var inspection = Ue4ssRuntimeSupport.InspectUe4ssState(_gameDirectory);
            var hasMarker = Ue4ssRuntimeSupport.HasOwnInstallMarker(_gameDirectory);

            switch (inspection.State)
            {
                case Ue4ssInstallState.InstalledByFfrExport when hasMarker:
                    _ue4ssStatusLabel.ForeColor = Color.FromArgb(0, 120, 0);
                    _ue4ssStatusLabel.Text = $"已安装 UE4SS（由本工具安装，版本：{inspection.InstalledVersion ?? "v4.0.0"}）";
                    _ue4ssHintLabel.Text =
                        "点击「卸载 UE4SS」仅删除安装标记文件（ue4ss\\.installed-by-ffr-export.json）记录的文件；" +
                        "已归档到 agp\\ffr-export\\usmap\\ 的 .usmap 文件会保留。";
                    _uninstallUe4ssButton.Enabled = !_isUninstalling;
                    break;

                case Ue4ssInstallState.InstalledByUser:
                    _ue4ssStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
                    _ue4ssStatusLabel.Text = "检测到用户自行安装的 UE4SS";
                    _ue4ssHintLabel.Text =
                        "未找到本工具的安装标记文件，本工具不会修改或卸载您原有的 UE4SS 配置与模组。";
                    _uninstallUe4ssButton.Enabled = false;
                    break;

                case Ue4ssInstallState.PartialUnknown:
                    _ue4ssStatusLabel.ForeColor = Color.FromArgb(180, 100, 0);
                    _ue4ssStatusLabel.Text = "检测到非本工具安装的局部文件";
                    _ue4ssHintLabel.Text =
                        "游戏目录 Win64 下存在未登记的 dwmapi.dll 或 UE4SS.dll，为防止误删，卸载按钮已禁用。";
                    _uninstallUe4ssButton.Enabled = false;
                    break;

                default:
                    _ue4ssStatusLabel.ForeColor = SystemColors.GrayText;
                    _ue4ssStatusLabel.Text = "当前游戏目录未安装 UE4SS";
                    _ue4ssHintLabel.Text =
                        "当缺少 .usmap 时，可在主窗口点击「一键获取 .usmap（会启动一次游戏）」自动安装并转储。";
                    _uninstallUe4ssButton.Enabled = false;
                    break;
            }
        }
        catch (Exception exception)
        {
            _ue4ssStatusLabel.ForeColor = Color.FromArgb(185, 28, 28);
            _ue4ssStatusLabel.Text = "检查 UE4SS 状态失败";
            _ue4ssHintLabel.Text = exception.Message;
            _uninstallUe4ssButton.Enabled = false;
        }
    }

    private void OnOpenDataDirClick(object? sender, EventArgs e)
    {
        var dir = _dataDirTextBox.Text.Trim();
        try
        {
            Directory.CreateDirectory(PathSupport.ToIoPath(dir));
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
                $"打开数据目录失败（{dir}）：{exception.Message}",
                "无法打开目录",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
    }

    private async void OnUninstallUe4ssClick(object? sender, EventArgs e)
    {
        if (_isUninstalling || string.IsNullOrWhiteSpace(_gameDirectory))
        {
            return;
        }

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

        var confirm = MessageBox.Show(
            this,
            $"确定要从以下游戏目录卸载由本工具安装的 UE4SS 吗？\n{_gameDirectory}\n\n"
            + "说明：\n"
            + "· 仅删除安装标记文件（ue4ss\\.installed-by-ffr-export.json）记录的文件；\n"
            + "· 不会删除已归档到 %LOCALAPPDATA%\\agp\\ffr-export\\usmap\\ 的 .usmap 文件；\n"
            + "· 卸载前请确保游戏已关闭。",
            "确认卸载 UE4SS",
            MessageBoxButtons.YesNo,
            MessageBoxIcon.Question,
            MessageBoxDefaultButton.Button2);
        if (confirm != DialogResult.Yes)
        {
            return;
        }

        _isUninstalling = true;
        _uninstallUe4ssButton.Enabled = false;
        _closeButton.Enabled = false;
        _ue4ssStatusLabel.ForeColor = SystemColors.ControlText;
        _ue4ssStatusLabel.Text = "正在卸载 UE4SS……";

        var stdoutLines = new List<string>();
        var stderrLines = new List<string>();
        try
        {
            var args = BuildCliCommand.BuildUninstallUe4ssArguments(_gameDirectory);
            _logToMain?.Invoke($"执行卸载命令：{cliLookup.FileName} uninstall-ue4ss --game \"{_gameDirectory}\"");
            var exitCode = await BuildCliCommand.RunAsync(
                cliLookup.ExpectedPath,
                args,
                line =>
                {
                    lock (stdoutLines)
                    {
                        stdoutLines.Add(line);
                    }
                },
                line =>
                {
                    lock (stderrLines)
                    {
                        stderrLines.Add(line);
                    }
                },
                CancellationToken.None);

            foreach (var line in stdoutLines)
            {
                _logToMain?.Invoke(line);
            }

            foreach (var line in stderrLines)
            {
                _logToMain?.Invoke($"[stderr] {line}");
            }

            if (exitCode == 0)
            {
                Ue4ssUninstalled = true;
                var detail = stdoutLines.Count > 0 ? string.Join(Environment.NewLine, stdoutLines) : "已卸载由本工具安装的 UE4SS。";
                MessageBox.Show(
                    this,
                    detail,
                    "卸载完成",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Information);
            }
            else
            {
                var errDetail = stderrLines.Count > 0
                    ? string.Join(Environment.NewLine, stderrLines)
                    : $"卸载失败（退出码：{exitCode}）。";
                MessageBox.Show(
                    this,
                    errDetail,
                    "卸载 UE4SS 失败",
                    MessageBoxButtons.OK,
                    MessageBoxIcon.Error);
            }
        }
        catch (Exception exception)
        {
            _logToMain?.Invoke($"[错误] 卸载 UE4SS 失败：{exception.Message}");
            MessageBox.Show(
                this,
                $"卸载 UE4SS 失败：{exception.Message}",
                "卸载 UE4SS 失败",
                MessageBoxButtons.OK,
                MessageBoxIcon.Error);
        }
        finally
        {
            _isUninstalling = false;
            _closeButton.Enabled = true;
            RefreshUe4ssStatus();
        }
    }
}
