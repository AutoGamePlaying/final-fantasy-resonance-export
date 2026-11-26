namespace FfrExport.Gui;

/// <summary>
/// Confirmation dialog shown when the user clicks "Clean up UE4SS..." on the main window.
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

        Text = L.Get("Cleanup_Title");
        Font = uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.CenterParent;
        ClientSize = new Size(640, 450);

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
            MaximumSize = new Size(605, 0),
            Margin = new Padding(0, 0, 0, 4),
        };

        var gameLabel = new Label
        {
            Text = L.Format("Cleanup_GameDirLabel", gameDirectory),
            AutoSize = true,
            ForeColor = SystemColors.GrayText,
            MaximumSize = new Size(605, 0),
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
                Text = L.Get("Cleanup_CheckIncludeForeign"),
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
            MaximumSize = new Size(605, 0),
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
            Text = L.Get("Gui_BtnCancel"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(96, 30),
            Padding = new Padding(8, 0, 8, 0),
            DialogResult = DialogResult.Cancel,
            Margin = new Padding(8, 0, 0, 0),
        };
        _confirmButton = new Button
        {
            Text = L.Get("Cleanup_BtnConfirm"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(124, 30),
            Padding = new Padding(8, 0, 8, 0),
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
            return L.Format("Cleanup_HeaderOwn", version);
        }

        if (plan.IsForeignUe4ss)
        {
            return L.Get("Cleanup_HeaderForeign");
        }

        return L.Get("Cleanup_HeaderRemnantsOnly");
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
            _actionHintLabel.Text = L.Get("Cleanup_HintForeignNoRemnants");
            return;
        }

        _confirmButton.Enabled = true;
        _actionHintLabel.ForeColor = SystemColors.GrayText;
        _actionHintLabel.Text = includeForeign
            ? L.Get("Cleanup_HintIncludeForeign")
            : L.Get("Cleanup_HintDefault");
    }

    private void OnConfirmClick(object? sender, EventArgs e)
    {
        var includeForeign = _includeForeignCheckBox?.Checked == true;
        if (_plan.IsForeignUe4ss && includeForeign)
        {
            var warningText = L.Format("Cleanup_SecondConfirmBody", _backupRootPreview, _plan.Inspection.Win64Directory);

            var secondConfirm = MessageBox.Show(
                this,
                warningText,
                L.Get("Cleanup_SecondConfirmTitle"),
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
