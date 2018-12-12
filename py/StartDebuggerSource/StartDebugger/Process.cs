using System;
using System.Runtime.ConstrainedExecution;
using System.Runtime.InteropServices;
using System.Security;

namespace StartDebugger
{
    public class Process : IDisposable
    {
        [StructLayout(LayoutKind.Sequential)]
        private struct SECURITY_ATTRIBUTES
        {
            public int nLength;
            public IntPtr lpSecurityDescriptor;
            public int bInheritHandle;
        }

        [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
        private struct STARTUPINFO
        {
            public Int32 cb;
            public string lpReserved;
            public string lpDesktop;
            public string lpTitle;
            public Int32 dwX;
            public Int32 dwY;
            public Int32 dwXSize;
            public Int32 dwYSize;
            public Int32 dwXCountChars;
            public Int32 dwYCountChars;
            public Int32 dwFillAttribute;
            public Int32 dwFlags;
            public Int16 wShowWindow;
            public Int16 cbReserved2;
            public IntPtr lpReserved2;
            public IntPtr hStdInput;
            public IntPtr hStdOutput;
            public IntPtr hStdError;
        }

        [StructLayout(LayoutKind.Sequential)]
        private struct PROCESS_INFORMATION
        {
            public IntPtr hProcess;
            public IntPtr hThread;
            public int dwProcessId;
            public int dwThreadId;
        }

        [DllImport("kernel32.dll", SetLastError = true, CharSet = CharSet.Auto)]
        private static extern bool CreateProcess(
          string lpApplicationName,
          string lpCommandLine,
          //ref SECURITY_ATTRIBUTES lpProcessAttributes,
          //ref SECURITY_ATTRIBUTES lpThreadAttributes,
          IntPtr lpProcessAttributes,
          IntPtr lpThreadAttributes,
          bool bInheritHandles,
          uint dwCreationFlags,
          IntPtr lpEnvironment,
          string lpCurrentDirectory,
          [In] ref STARTUPINFO lpStartupInfo,
          out PROCESS_INFORMATION lpProcessInformation);

        private static readonly uint CREATE_SUSPENDED = 0x00000004;
        private static readonly uint CREATE_NEW_CONSOLE = 0x00000010;

        [DllImport("kernel32.dll", SetLastError = true)]
        static extern uint ResumeThread(IntPtr hThread);

        [DllImport("kernel32.dll", SetLastError = true)]
        [ReliabilityContract(Consistency.WillNotCorruptState, Cer.Success)]
        [SuppressUnmanagedCodeSecurity]
        [return: MarshalAs(UnmanagedType.Bool)]
        static extern bool CloseHandle(IntPtr hObject);

        private Process(PROCESS_INFORMATION processInfo)
        {
            this.processInfo = processInfo;
        }

        public int PID
        {
            get
            {
                return this.processInfo.dwProcessId;
            }
        }

        public void Resume()
        {
            ResumeThread(processInfo.hThread);
        }

        public static Process CreateSuspended(string commandLine)
        {
            var startupInfo = new STARTUPINFO();
            var processInfo = new PROCESS_INFORMATION();

            CreateProcess(null, commandLine, IntPtr.Zero, IntPtr.Zero, false, CREATE_SUSPENDED | CREATE_NEW_CONSOLE, IntPtr.Zero, null, ref startupInfo, out processInfo);

            return new Process(processInfo);
        }

        private PROCESS_INFORMATION processInfo;

        #region IDisposable Support
        private bool disposedValue = false; // To detect redundant calls

        protected virtual void Dispose(bool disposing)
        {
            if (!disposedValue)
            {
                CloseHandle(this.processInfo.hThread);
                CloseHandle(this.processInfo.hProcess);

                disposedValue = true;
            }
        }

         ~Process()
        {
            // Do not change this code. Put cleanup code in Dispose(bool disposing) above.
            Dispose(false);
        }

        // This code added to correctly implement the disposable pattern.
        public void Dispose()
        {
            // Do not change this code. Put cleanup code in Dispose(bool disposing) above.
            Dispose(true);
            GC.SuppressFinalize(this);
        }
        #endregion
    }
}
