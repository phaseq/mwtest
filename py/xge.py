#! python
import sys
import subprocess
import time
import socket
import json
import threading
from adaptors import strip_invalid_chars


def recv_lines(conn):
    buffer = conn.recv(1024)
    while True:
        if '\n' in buffer:
            line, buffer = buffer.split('\n', 1)
            yield line
        else:
            more = conn.recv(1024)
            if more:
                buffer += more
            else:
                break


def server():
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.bind(('127.0.0.1', 0))
    print "port: {}".format(s.getsockname()[1])
    sys.stdout.flush()
    s.listen(1)
    conn, addr = s.accept()
    for line in recv_lines(conn):
        run_local, caption, cwd, command = json.loads(line)
        command_str = ' '.join(command)
        if run_local == 'local':
            subprocess.Popen('xgSubmit /allowremote=off /caption=\"{}\" /command {}'.format(caption, command_str),
                             cwd=cwd)
        else:
            subprocess.Popen('xgSubmit /caption=\"{}\" /command {}'.format(caption, command_str), cwd=cwd)


def client_async_reader(child, results, xge):
    for line in iter(child.stdout.readline, ''):
        if line.startswith('wrapped '):
            id, retcode, output = json.loads(line[8:])

            # sometimes, XGE throws an error like this when running MachsimTest, and I don't know why:
            # IncrediBuild [PID 9148, 64bit]: Exception at +0x00000000000FAAE3: Cannot get file record: Null position
            # ($CALLTRACE:000000006FE783E0:6FF1AAE3,2065B,1E70C,F3DD,E51BEF,E349E,248,3D2,98C,7FF875094BE8,000000000000)
            if output.find('Cannot get file record: Null position') >= 0:
                xge.socket.send(xge.running[id] + '\n')
            else:
                del xge.running[id]
                results.append((id, retcode, output))


class XGE:
    def __init__(self):
        import inspect
        import os
        self.this_file_location = inspect.getfile(inspect.currentframe())
        profile_path = os.path.join(os.path.dirname(self.this_file_location), "profile.xml")
        self.child = subprocess.Popen(
            'xgConsole /profile="{}" /command="python.exe {} server" /openmonitor'.format(profile_path,
                                                                                          self.this_file_location),
            stdout=subprocess.PIPE)
        port = 0
        for line in iter(self.child.stdout.readline, ''):
            if line.startswith('port: '):
                port = int(line[6:])
                break
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self.socket.connect(('127.0.0.1', port))
        self.results = []
        self.done = False
        self.thread = threading.Thread(target=client_async_reader, args=(self.child, self.results, self))
        self.thread.daemon = True
        self.thread.start()
        self.running = {}

    def run(self, cmd_id, caption, cmd, cwd):
        cmd_wrapped = ['python', self.this_file_location, 'wrap', str(cmd_id)] + cmd
        msg = json.dumps(['remote', caption, cwd, cmd_wrapped])
        self.running[cmd_id] = msg
        self.socket.send(msg + '\n')

    def run_local(self, cmd_id, caption, cmd, cwd):
        cmd_wrapped = ['python', self.this_file_location, 'wrap', str(cmd_id)] + cmd
        msg = json.dumps(['local', caption, cwd, cmd_wrapped])
        self.running[cmd_id] = msg
        self.socket.send(msg + '\n')

    def get_result(self):
        if self.results:
            return self.results.pop()
        return None

    def poll(self, timeout):
        result = self.get_result()
        if result:
            return result
        start = time.time()
        while not result and not self.is_done() and (time.time() - start) < timeout:
            time.sleep(0.01)
            result = self.get_result()
        return result

    def is_done(self):
        return self.running == {} and self.results == []

    def close(self):
        self.socket.shutdown(socket.SHUT_RDWR)
        self.socket.close()
        if not self.is_done():
            self.child.terminate()  # abort remaining XGE processes
        self.thread.join()


if __name__ == '__main__':
    if sys.argv[1] == 'sleep':
        time.sleep(3)
        print 'done sleeping.'
        exit(3)
    elif sys.argv[1] == 'server':
        server()
    elif sys.argv[1] == 'wrap':
        params = sys.argv[3:]
        child = subprocess.Popen(params, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        output, err = child.communicate()
        output = strip_invalid_chars(output + err).replace('\r\n', '\n')
        print 'wrapped ' + json.dumps([sys.argv[2], child.returncode, output])
        exit(child.returncode)
