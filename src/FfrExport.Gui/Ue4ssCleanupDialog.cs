namespace FfrExport.Gui;

/// <summary>
/// Confirmation dialog shown when the user clicks "清理 UE4SS…" on the main window.
/// Lists every file/directory to be deleted or moved before acting, and enforces a second
/// warning confirmation before moving a foreign (non-ffr-export) UE4SS installation to backup.
/// </summary>
internal sealed class Ue4ssCleanupDialog : Form
{
    private readonly Ue4ssCleanupPlan _plan;
    private readonly string _backupRootPreview;
    private readonly TextBox _previewTextBox;
    private readonly CheckBox? _includeForeignCheckBox;
    private readonly Label _actionHintLabel;
    private readonly Button _confirmButton;
    private readonly Button _cancelButton;

    internal bool Confirmed { get; private set; }

    internal bool IncludeForeign { get; private set; }

    internal Ue4ssCleanupDialog(
        string gameDirectory,
        Ue4ssCleanupPlan plan,
        string activeAgpRoot,
        Font uiFont,
        Font boldUiFont)
    {
        ArgumentNullException.ThrowIfNull(plan);

        _plan = plan;
        var backupBase = AgpPaths.GetUe4ssBackupDirectoryForAgpRoot(activeAgpRoot);
        _backupRootPreview = Path.Combine(backupBase, "<yyyyMMdd-HHmmss>");

        SuspendLayout();

        Text = "清理 UE4SS";
        Font = uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.CenterParent;
        ClientSize = new Size(620, 440);

        var root = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 6,
            Padding = new Padding(12),
        };
        root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        var titleLabel = new Label
        {
            Text = BuildHeaderSummary(plan),
            Font = boldUiFont,
            AutoSize = true,
            MaximumSize = new Size(585, 0),
            Margin = new Padding(0, 0, 0, 4),
        };

        var gameLabel = new Label
        {
            Text = $"游戏目录：{gameDirectory}",
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(585, 0),
            Margin = new Padding(0, 0, 0, 6),
        };

        _previewTextBox = new TextBox
        {
            Dock = DockStyle.Fill,
            Multiline = true,
            ReadOnly = true,
            ScrollBars = ScrollBars.Both,
            WordWrap = false,
            BackColor = SystemColors.Window,
            Margin = new Padding(0, 0, 0, 8),
        };

        if (plan.IsForeignUe4ss)
        {
            _includeForeignCheckBox = new CheckBox
            {
                Text = "连同 UE4SS 一起移除（不直接删除，移动到备份目录以便随时恢复）",
                AutoSize = true,
                Checked = false,
                Margin = new Padding(0, 0, 0, 6),
            };
            _includeForeignCheckBox.CheckedChanged += (_, _) => UpdatePreviewAndButtons();
        }

        _actionHintLabel = new Label
        {
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(585, 0),
            Margin = new Padding(0, 0, 0, 8),
        };

        var footerPanel = new FlowLayoutPanel
        {
            Dock = DockStyle.Fill,
            FlowDirection = FlowDirection.RightToLeft,
            AutoSize = true,
            Margin = new Padding(0),
        };

        _cancelButton = new Button
        {
            Text = "取消",
            Width = 96,
            Height = 30,
            DialogResult = DialogResult.Cancel,
            Margin = new Padding(8, 0, 0, 0),
        };
        _confirmButton = new Button
        {
            Text = "确认清理",
            Width = 112,
            Height = 30,
            Margin = new Padding(0),
        };
        _confirmButton.Click += OnConfirmClick;

        footerPanel.Controls.Add(_cancelButton);
        footerPanel.Controls.Add(_confirmButton);

        root.Controls.Add(titleLabel, 0, 0);
        root.Controls.Add(gameLabel, 0, 1);
        root.Controls.Add(_previewTextBox, 0, 2);
        if (_includeForeignCheckBox is not null)
        {
            root.Controls.Add(_includeForeignCheckBox, 0, 3);
        }

        root.Controls.Add(_actionHintLabel, 0, 4);
        root.Controls.Add(footerPanel, 0, 5);
        Controls.Add(root);

        CancelButton = _cancelButton;

        ResumeLayout(false);
        UpdatePreviewAndButtons();
    }

    private static string BuildHeaderSummary(Ue4ssCleanupPlan plan)
    {
        if (plan.HasOurManagedInstall)
        {
            var version = plan.Inspection.InstalledVersion ?? "v4.0.0";
            return $"检测到由本工具安装的 UE4SS（{version}），确认后将按安装清单清理下列内容：";
        }

        if (plan.IsForeignUe4ss)
        {
            return "检测到非本工具安装的 UE4SS（缺少本工具安装标记文件）：默认仅清理本工具残留，原有 UE4SS 保持不动。";
        }

        return "未发现完整 UE4SS 安装，检测到本工具留下的临时残留文件：";
    }

    private void UpdatePreviewAndButtons()
    {
        var includeForeign = _includeForeignCheckBox?.Checked == true;
        var previewText = Ue4ssRuntimeSupport.FormatCleanupPreviewText(_plan, includeForeign, _backupRootPreview);
        _previewTextBox.Text = previewText.Replace("\n", Environment.NewLine, StringComparison.Ordinal);

        if (_plan.IsForeignUe4ss && !includeForeign && !_plan.HasRemnants)
        {
            _confirmButton.Enabled = false;
            _actionHintLabel.ForeColor = Color.FromArgb(180, 100, 0);
            _actionHintLabel.Text =
                "当前没有本工具的残留文件需要清理；如需将非本工具安装的 UE4SS 移出游戏目录（移动至备份目录），请勾选上方选项。";
            return;
        }

        _confirmButton.Enabled = true;
        _actionHintLabel.ForeColor = SystemColors.GrayText;
        _actionHintLabel.Text = includeForeign
            ? "说明：执行时会弹出第二次确认；已复制到 agp\\ffr-export\\usmap\\ 的 .usmap 文件不受影响。"
            : "说明：已复制到 agp\\ffr-export\\usmap\\ 的 .usmap 文件不受影响，清理后仍可继续使用。";
    }

    private void OnConfirmClick(object? sender, EventArgs e)
    {
        var includeForeign = _includeForeignCheckBox?.Checked == true;
        if (_plan.IsForeignUe4ss && includeForeign)
        {
            var warningText =
                "二次确认：当前游戏目录中的 UE4SS 不是本工具安装的，里面可能包含您自行安装的其他模组或自定义配置！\n\n"
                + "执行时不会直接删除这些文件，而是将 dwmapi.dll 和整个 ue4ss\\ 目录移动到备份目录：\n"
                + $"{_backupRootPreview}\n\n"
                + "如需恢复原状，只需把备份目录内的 dwmapi.dll 和 ue4ss 文件夹复制回：\n"
                + $"{_plan.Inspection.Win64Directory}\n\n"
                + "确定要连同非本工具安装的 UE4SS 一起移出游戏目录吗？";

            var secondConfirm = MessageBox.Show(
                this,
                warningText,
                "二次确认：移除非本工具安装的 UE4SS",
                MessageBoxButtons.YesNo,
                MessageBoxIcon.Warning,
                MessageBoxDefaultButton.Button2);
            if (secondConfirm != DialogResult.Yes)
            {
                return;
            }
        }

        Confirmed = true;
        IncludeForeign = includeForeign;
        DialogResult = DialogResult.OK;
        Close();
    }
}
