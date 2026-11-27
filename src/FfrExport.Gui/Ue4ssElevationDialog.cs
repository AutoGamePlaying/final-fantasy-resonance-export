namespace FfrExport.Gui;

/// <summary>
/// Modal prompt displayed when UE4SS cleanup receives needs_elevation (access denied).
/// Explains which file was denied, why it likely occurred (previous installation as Administrator),
/// and offers two explicit actions: "Clean up as Administrator" or "Cancel".
/// </summary>
internal sealed class Ue4ssElevationDialog : Form
{
    private readonly Button _elevateButton;
    private readonly Button _cancelButton;

    internal bool RetryElevated { get; private set; }

    internal Ue4ssElevationDialog(
        string deniedPath,
        Font uiFont,
        Font boldUiFont)
    {
        SuspendLayout();

        Text = L.Get("Cleanup_ElevationTitle");
        Font = uiFont;
        AutoScaleMode = AutoScaleMode.Dpi;
        FormBorderStyle = FormBorderStyle.FixedDialog;
        MaximizeBox = false;
        MinimizeBox = false;
        ShowInTaskbar = false;
        StartPosition = FormStartPosition.CenterParent;
        ClientSize = new Size(560, 250);

        var root = new TableLayoutPanel
        {
            Dock = DockStyle.Fill,
            ColumnCount = 1,
            RowCount = 5,
            Padding = new Padding(14),
        };
        root.ColumnStyles.Add(new ColumnStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));
        root.RowStyles.Add(new RowStyle(SizeType.Percent, 100f));
        root.RowStyles.Add(new RowStyle(SizeType.AutoSize));

        var headlineLabel = new Label
        {
            Text = L.Get("Cleanup_ElevationHeadline"),
            Font = boldUiFont,
            ForeColor = Color.FromArgb(160, 85, 0),
            AutoSize = true,
            MaximumSize = new Size(528, 0),
            Margin = new Padding(0, 0, 0, 8),
        };

        var pathCaptionLabel = new Label
        {
            Text = L.Get("Cleanup_ElevationPathLabel"),
            AutoSize = true,
            Margin = new Padding(0, 0, 0, 3),
        };

        var pathTextBox = new TextBox
        {
            Dock = DockStyle.Top,
            ReadOnly = true,
            BackColor = SystemColors.ControlLightLight,
            Text = string.IsNullOrWhiteSpace(deniedPath) ? L.Get("Common_Unknown") : deniedPath,
            Margin = new Padding(0, 0, 0, 10),
        };

        var explanationLabel = new Label
        {
            Text = L.Get("Cleanup_ElevationExplanation"),
            AutoSize = true,
            MaximumSize = new Size(528, 0),
            Margin = new Padding(0, 0, 0, 10),
        };

        var footerPanel = new FlowLayoutPanel
        {
            Dock = DockStyle.Fill,
            FlowDirection = FlowDirection.RightToLeft,
            AutoSize = true,
            Margin = new Padding(0, 4, 0, 0),
        };

        _cancelButton = new Button
        {
            Text = L.Get("Gui_BtnCancel"),
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(96, 32),
            Padding = new Padding(10, 0, 10, 0),
            DialogResult = DialogResult.Cancel,
            Margin = new Padding(8, 0, 0, 0),
        };

        _elevateButton = new Button
        {
            Text = L.Get("Cleanup_BtnElevateRetry"),
            Font = boldUiFont,
            AutoSize = true,
            AutoSizeMode = AutoSizeMode.GrowAndShrink,
            MinimumSize = new Size(168, 32),
            Padding = new Padding(10, 0, 10, 0),
            Margin = new Padding(0),
        };
        _elevateButton.Click += (_, _) =>
        {
            RetryElevated = true;
            DialogResult = DialogResult.OK;
            Close();
        };

        footerPanel.Controls.Add(_cancelButton);
        footerPanel.Controls.Add(_elevateButton);

        root.Controls.Add(headlineLabel, 0, 0);
        root.Controls.Add(pathCaptionLabel, 0, 1);
        root.Controls.Add(pathTextBox, 0, 2);
        root.Controls.Add(explanationLabel, 0, 3);
        root.Controls.Add(footerPanel, 0, 4);
        Controls.Add(root);

        AcceptButton = _elevateButton;
        CancelButton = _cancelButton;

        ResumeLayout(false);
    }
}
