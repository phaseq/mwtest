namespace StartDebugger
{
    partial class PickVsInstanceDialog
    {
        /// <summary>
        /// Required designer variable.
        /// </summary>
        private System.ComponentModel.IContainer components = null;

        /// <summary>
        /// Clean up any resources being used.
        /// </summary>
        /// <param name="disposing">true if managed resources should be disposed; otherwise, false.</param>
        protected override void Dispose(bool disposing)
        {
            if (disposing && (components != null))
            {
                components.Dispose();
            }
            base.Dispose(disposing);
        }

        #region Windows Form Designer generated code

        /// <summary>
        /// Required method for Designer support - do not modify
        /// the contents of this method with the code editor.
        /// </summary>
        private void InitializeComponent()
        {
            this.groupBoxAvailableInstances = new System.Windows.Forms.GroupBox();
            this.listViewAvailableInstances = new System.Windows.Forms.ListView();
            this.columnHeaderSolution = ((System.Windows.Forms.ColumnHeader)(new System.Windows.Forms.ColumnHeader()));
            this.columnHeaderVersion = ((System.Windows.Forms.ColumnHeader)(new System.Windows.Forms.ColumnHeader()));
            this.buttonCancel = new System.Windows.Forms.Button();
            this.buttonOk = new System.Windows.Forms.Button();
            this.buttonRefresh = new System.Windows.Forms.Button();
            this.groupBoxAvailableInstances.SuspendLayout();
            this.SuspendLayout();
            // 
            // groupBoxAvailableInstances
            // 
            this.groupBoxAvailableInstances.Anchor = ((System.Windows.Forms.AnchorStyles)((((System.Windows.Forms.AnchorStyles.Top | System.Windows.Forms.AnchorStyles.Bottom) 
            | System.Windows.Forms.AnchorStyles.Left) 
            | System.Windows.Forms.AnchorStyles.Right)));
            this.groupBoxAvailableInstances.Controls.Add(this.listViewAvailableInstances);
            this.groupBoxAvailableInstances.Location = new System.Drawing.Point(12, 12);
            this.groupBoxAvailableInstances.Name = "groupBoxAvailableInstances";
            this.groupBoxAvailableInstances.Size = new System.Drawing.Size(560, 309);
            this.groupBoxAvailableInstances.TabIndex = 0;
            this.groupBoxAvailableInstances.TabStop = false;
            this.groupBoxAvailableInstances.Text = "Available instances";
            // 
            // listViewAvailableInstances
            // 
            this.listViewAvailableInstances.Anchor = ((System.Windows.Forms.AnchorStyles)((((System.Windows.Forms.AnchorStyles.Top | System.Windows.Forms.AnchorStyles.Bottom) 
            | System.Windows.Forms.AnchorStyles.Left) 
            | System.Windows.Forms.AnchorStyles.Right)));
            this.listViewAvailableInstances.Columns.AddRange(new System.Windows.Forms.ColumnHeader[] {
            this.columnHeaderSolution,
            this.columnHeaderVersion});
            this.listViewAvailableInstances.FullRowSelect = true;
            this.listViewAvailableInstances.Location = new System.Drawing.Point(6, 19);
            this.listViewAvailableInstances.Name = "listViewAvailableInstances";
            this.listViewAvailableInstances.Size = new System.Drawing.Size(548, 284);
            this.listViewAvailableInstances.TabIndex = 0;
            this.listViewAvailableInstances.UseCompatibleStateImageBehavior = false;
            this.listViewAvailableInstances.View = System.Windows.Forms.View.Details;
            this.listViewAvailableInstances.MouseDoubleClick += new System.Windows.Forms.MouseEventHandler(this.listViewAvailableInstances_MouseDoubleClick);
            // 
            // columnHeaderSolution
            // 
            this.columnHeaderSolution.Text = "Solution";
            // 
            // columnHeaderVersion
            // 
            this.columnHeaderVersion.Text = "Version";
            // 
            // buttonCancel
            // 
            this.buttonCancel.Anchor = ((System.Windows.Forms.AnchorStyles)((System.Windows.Forms.AnchorStyles.Bottom | System.Windows.Forms.AnchorStyles.Right)));
            this.buttonCancel.DialogResult = System.Windows.Forms.DialogResult.Cancel;
            this.buttonCancel.FlatStyle = System.Windows.Forms.FlatStyle.System;
            this.buttonCancel.Location = new System.Drawing.Point(497, 327);
            this.buttonCancel.Name = "buttonCancel";
            this.buttonCancel.Size = new System.Drawing.Size(75, 23);
            this.buttonCancel.TabIndex = 1;
            this.buttonCancel.Text = "&Cancel";
            this.buttonCancel.UseVisualStyleBackColor = true;
            // 
            // buttonOk
            // 
            this.buttonOk.Anchor = ((System.Windows.Forms.AnchorStyles)((System.Windows.Forms.AnchorStyles.Bottom | System.Windows.Forms.AnchorStyles.Right)));
            this.buttonOk.DialogResult = System.Windows.Forms.DialogResult.OK;
            this.buttonOk.FlatStyle = System.Windows.Forms.FlatStyle.System;
            this.buttonOk.Location = new System.Drawing.Point(416, 327);
            this.buttonOk.Name = "buttonOk";
            this.buttonOk.Size = new System.Drawing.Size(75, 23);
            this.buttonOk.TabIndex = 2;
            this.buttonOk.Text = "&OK";
            this.buttonOk.UseVisualStyleBackColor = true;
            // 
            // buttonRefresh
            // 
            this.buttonRefresh.Anchor = ((System.Windows.Forms.AnchorStyles)((System.Windows.Forms.AnchorStyles.Bottom | System.Windows.Forms.AnchorStyles.Left)));
            this.buttonRefresh.FlatStyle = System.Windows.Forms.FlatStyle.System;
            this.buttonRefresh.Location = new System.Drawing.Point(12, 327);
            this.buttonRefresh.Name = "buttonRefresh";
            this.buttonRefresh.Size = new System.Drawing.Size(75, 23);
            this.buttonRefresh.TabIndex = 3;
            this.buttonRefresh.Text = "&Refresh";
            this.buttonRefresh.UseVisualStyleBackColor = true;
            this.buttonRefresh.Click += new System.EventHandler(this.buttonRefresh_Click);
            // 
            // PickVsInstanceDialog
            // 
            this.AcceptButton = this.buttonOk;
            this.AutoScaleDimensions = new System.Drawing.SizeF(6F, 13F);
            this.AutoScaleMode = System.Windows.Forms.AutoScaleMode.Font;
            this.CancelButton = this.buttonCancel;
            this.ClientSize = new System.Drawing.Size(584, 362);
            this.Controls.Add(this.buttonRefresh);
            this.Controls.Add(this.buttonOk);
            this.Controls.Add(this.buttonCancel);
            this.Controls.Add(this.groupBoxAvailableInstances);
            this.Name = "PickVsInstanceDialog";
            this.ShowIcon = false;
            this.Text = "Pick a Visual Studio instance";
            this.groupBoxAvailableInstances.ResumeLayout(false);
            this.ResumeLayout(false);

        }

        #endregion

        private System.Windows.Forms.GroupBox groupBoxAvailableInstances;
        private System.Windows.Forms.Button buttonCancel;
        private System.Windows.Forms.Button buttonOk;
        private System.Windows.Forms.ListView listViewAvailableInstances;
        private System.Windows.Forms.ColumnHeader columnHeaderSolution;
        private System.Windows.Forms.ColumnHeader columnHeaderVersion;
        private System.Windows.Forms.Button buttonRefresh;
    }
}