using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;

namespace StartDebugger
{
    public class RunningObjectTable
    {
        [DllImport("ole32.dll")]
        private static extern int CreateBindCtx(
            uint reserved,
            out IBindCtx ppbc);

        [DllImport("ole32.dll")]
        private static extern void GetRunningObjectTable(
            int reserved,
            out IRunningObjectTable prot);

        public RunningObjectTable()
        {
            GetRunningObjectTable(0, out this.rot);
            if (this.rot == null)
                throw new InvalidOperationException("Failed to get the Running Object Table");
        }

        public T ProbeAs<T>(IMoniker moniker) where T : class
        {
            object comObject;
            this.rot.GetObject(moniker, out comObject);

            return comObject as T;
        }

        public string GetDisplayName(IMoniker moniker)
        {
            IBindCtx bindCtx;
            CreateBindCtx(0, out bindCtx);
            if (bindCtx == null)
                return null;
            string displayName;
            moniker.GetDisplayName(bindCtx, null, out displayName);
            return displayName;
        }

        public IEnumerable<IMoniker> EnumerateRunningInstances(string progId)
        {
            // get enumerator for ROT entries
            IEnumMoniker monikerEnumerator = null;
            this.rot.EnumRunning(out monikerEnumerator);

            if (monikerEnumerator == null)
                yield break;

            monikerEnumerator.Reset();

            IntPtr pNumFetched = new IntPtr();
            IMoniker[] monikers = new IMoniker[1];

            // go through all entries and identifies app instances
            while (monikerEnumerator.Next(1, monikers, pNumFetched) == 0)
            {
                yield return monikers[0];
            }
        }

        private IRunningObjectTable rot;
    }
}
