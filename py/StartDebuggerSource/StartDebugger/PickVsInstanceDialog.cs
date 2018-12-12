using System.Windows.Forms;

namespace StartDebugger
{
    public partial class PickVsInstanceDialog : Form
    {
        private PickVsInstanceDialog()
        {
            InitializeComponent();

            this.rot = new RunningObjectTable();
            RefreshAvailableInstances();
        }

        private void RefreshAvailableInstances()
        {
            this.listViewAvailableInstances.Items.Clear();
            QueryVsInstances();
            this.listViewAvailableInstances.AutoResizeColumns(ColumnHeaderAutoResizeStyle.ColumnContent);
            this.listViewAvailableInstances.AutoResizeColumns(ColumnHeaderAutoResizeStyle.HeaderSize);
        }
        
        private void QueryVsInstances()
        {
            foreach (var version in VisualStudioAutomation.Versions)
            {
                QueryVsInstances(version.ProgId, version.FriendlyName);
            }
        }

        private void QueryVsInstances(string progId, string friendlyName)
        {
            // see https://msdn.microsoft.com/en-us/library/ms228755.aspx
            foreach (var moniker in this.rot.EnumerateRunningInstances(progId))
            {
                bool isVsItemMoniker = (this.rot.GetDisplayName(moniker)?.IndexOf("!" + progId) ?? -1) == 0;
                if (!isVsItemMoniker)
                    continue;
                var dte = this.rot.ProbeAs<EnvDTE80.DTE2>(moniker);
                if (dte == null)
                    continue;
                var solutionName = string.IsNullOrEmpty(dte.Solution.FullName) ? "[No active solution]" : dte.Solution.FullName;
                var item = new ListViewItem(new string[] { solutionName, friendlyName });
                item.Tag = dte;
                this.listViewAvailableInstances.Items.Add(item);
            }
        }

        public static EnvDTE80.DTE2 AskVsInstance()
        {
            var dialog = new PickVsInstanceDialog();
            if (dialog.ShowDialog() == DialogResult.OK)
            {
                var selectedItems = dialog.listViewAvailableInstances.SelectedItems;
                if (selectedItems.Count >= 1)
                {
                    return selectedItems[0].Tag as EnvDTE80.DTE2;
                }
            }
            return null;
        }

        private void buttonRefresh_Click(object sender, System.EventArgs e)
        {
            RefreshAvailableInstances();
        }

        private RunningObjectTable rot;

        private void listViewAvailableInstances_MouseDoubleClick(object sender, MouseEventArgs e)
        {
            if (!(sender is ListView listView))
                return;
            if (listView.SelectedItems.Count == 0)
                return;
            this.DialogResult = DialogResult.OK;
        }
    }
}
