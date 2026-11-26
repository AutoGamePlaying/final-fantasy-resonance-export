using System.Diagnostics;

namespace FfrExport.Gui;

/// <summary>
/// Modal settings dialog displaying the active data directory, selection reason, and process integrity level.
/// UE4SS cleanup lives directly on the main window's .usmap row ("清理 UE4SS…").
/// </summary>
internal sealed class SettingsDialog : Form
{
    private readonly TextBox _dataDirTextBox;
    private readonly Button _openDataDirButton;
    private readonly Button _closeButton;

    internal SettingsDialog(
        AgpRootResolution agpResolution,
        Font uiFont,
        Font boldUiFont)
    {
        ArgumentNullException.ThrowIfNull(agpResolution);

        var toolRoot = agpResolution.ToolRoot ?? AgpPaths.ResolveToolRoot();

        SuspendLayout();

        Text = "设置";
        Font = uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.CenterParent;
        ClientSize = new Size(560, 260);

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

        // 1. Active data directory
        var dataGroup = new GroupBox
        {
            Text = "数据目录",
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
            RowCount = 3,
        };
        dataLayout.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        dataLayout.ColumnStyles.Add(new ColumnStyle(SizeType.AutoSize));

        _dataDirTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            ReadOnly = true,
            BackColor = SystemColors.ControlLightLight,
            Text = toolRoot,
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
            Text = "包含 output\\（默认导出）、usmap\\（映射文件）、downloads\\（下载缓存）、logs\\、tmp\\、ue4ss-backup\\ 与 settings.json。",
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(500, 0),
            Margin = new Padding(0, 2, 0, 4),
        };

        var statusSummary = BuildEnvironmentSummary(agpResolution);
        var envSummaryLabel = new Label
        {
            Text = statusSummary,
            Font = agpResolution.UsedFallback || agpResolution.Integrity.IsLowOrUntrusted ? boldUiFont : uiFont,
            AutoSize = true,
            ForeColor = agpResolution.UsedFallback || agpResolution.Integrity.IsLowOrUntrusted
                ? Color.FromArgb(160, 85, 0)
                : SystemColors.GrayText,
            MaximumSize = new Size(500, 0),
            Margin = new Padding(0, 2, 0, 0),
        };

        dataLayout.Controls.Add(_dataDirTextBox, 0, 0);
        dataLayout.Controls.Add(_openDataDirButton, 1, 0);
        dataLayout.Controls.Add(dataNoteLabel, 0, 1);
        dataLayout.SetColumnSpan(dataNoteLabel, 2);
        dataLayout.Controls.Add(envSummaryLabel, 0, 2);
        dataLayout.SetColumnSpan(envSummaryLabel, 2);
        dataGroup.Controls.Add(dataLayout);

        // 2. Tip pointing to main window UE4SS cleanup button
        var ue4ssNoteLabel = new Label
        {
            Text = "提示：UE4SS 清理功能位于主窗口「2. 映射与反射文件」区域的「清理 UE4SS…」按钮。",
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(520, 0),
            Margin = new Padding(2, 10, 0, 0),
        };

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
        root.Controls.Add(ue4ssNoteLabel, 0, 1);
        root.Controls.Add(footerPanel, 0, 2);
        Controls.Add(root);

        ResumeLayout(false);
    }

    private static string BuildEnvironmentSummary(AgpRootResolution resolution)
    {
        var integrityLine = $"进程完整性级别：{resolution.Integrity.DisplayTextZh}";
        var banner = AgpPaths.FormatGuiBannerMessage(resolution);
        return string.IsNullOrWhiteSpace(banner)
            ? integrityLine
            : $"{integrityLine}{Environment.NewLine}{banner}";
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
}
