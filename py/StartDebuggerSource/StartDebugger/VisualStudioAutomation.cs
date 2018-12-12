using EnvDTE;

namespace StartDebugger
{
    public struct VisualStudioVersion
    {
        public VisualStudioVersion(string progId, string friendlyName)
        {
            this.ProgId = progId;
            this.FriendlyName = friendlyName;
        }

        public string ProgId { get; private set; }
        public string FriendlyName { get; private set; }
    }

    public static class VisualStudioAutomation
    {
        public static readonly VisualStudioVersion[] Versions = new[]
        {
            new VisualStudioVersion("VisualStudio.DTE.15.0", "Visual Studio 2017"),
            new VisualStudioVersion("VisualStudio.DTE.14.0", "Visual Studio 2015"),
            new VisualStudioVersion("VisualStudio.DTE.12.0", "Visual Studio 2013"),
            new VisualStudioVersion("VisualStudio.DTE.11.0", "Visual Studio 2012"),
            new VisualStudioVersion("VisualStudio.DTE.10.0", "Visual Studio 2010"),
            new VisualStudioVersion("VisualStudio.DTE.9.0", "Visual Studio 2008")
        };

        public static void AttachToProcess(EnvDTE80.DTE2 dte, Process process, bool continueDebugging)
        {
            AttachToProcess(dte, process.PID, continueDebugging);
        }

        public static void AttachToProcess(EnvDTE80.DTE2 dte, int pid, bool continueDebugging)
        {
            // Setup the debug Output window.  
            Window w = (Window)dte.Windows.Item(EnvDTE.Constants.vsWindowKindOutput);
            w.Visible = true;
            OutputWindow ow = (OutputWindow)w.Object;
            OutputWindowPane owp = ow.OutputWindowPanes.Add("Local Processes Test");
            owp.Activate();

            Processes processes = dte.Debugger.LocalProcesses;
            if (processes.Count == 0)
            {
                owp.OutputString("No processes are running on this machine.");
            }
            else
            {
                owp.OutputString("Processes running on this machine:");
                foreach (EnvDTE.Process proc in processes)
                {
                    owp.OutputString("Process: [" + proc.ProcessID + "] " + proc.Name + "\n");
                    if (proc.ProcessID == pid)
                    {
                        proc.Attach();
                        if (continueDebugging)
                        {
                            TryContinueDebugging(dte);
                        }
                        break;
                    }
                }
            }
        }

        private static void TryContinueDebugging(EnvDTE80.DTE2 dte)
        {
            for (int i = 0; i < 5; ++i)
            {
                try
                {
                    dte.Debugger.Go(false);
                    break;
                }
                catch
                {
                    System.Threading.Thread.Sleep(1000);
                }
            }
        }
    }
}
