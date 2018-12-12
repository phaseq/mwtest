using System;
using System.Windows.Forms;

namespace StartDebugger
{
    class Program
    {
        [STAThread]
        static int Main(string[] args)
        {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(true);

            if (args.Length < 1)
            {
                Console.WriteLine("Usage: StartDebugger.exe <program> [args]");
                return 1;
            }

            var dte = PickVsInstanceDialog.AskVsInstance();
            if (dte == null)
                return 1;

            string commandLine = string.Join(" ", args);

            using (var process = Process.CreateSuspended(commandLine))
            {
                VisualStudioAutomation.AttachToProcess(dte, process, false);
                process.Resume();
            }

            return 0;
        }
    }
}
