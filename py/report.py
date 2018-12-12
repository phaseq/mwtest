import sys
import os
import shlex
import struct
import platform
import subprocess
import xml.sax.saxutils


def report_results(test_results, artifacts_dir, verbosity, test_count):
    if test_count == 0:
        print("WARNING: you have not selected any tests!")

    failed = {}
    test_results_backup = []
    i = 0
    for name, id, result in test_results:
        i += 1
        success = result['success']
        message = result['output']
        test_results_backup.append((name, id, result))
        print_result_to_log(artifacts_dir, success, name, id, message)
        progress = (i, test_count)
        print_result_to_stdout(success, name, id, message, verbosity, progress)
        if not success:
            if not (name, id) in failed:
                failed[(name, id)] = [1, 0]
            else:
                failed[(name, id)][0] += 1
        elif (name, id) in failed:
            failed[(name, id)][1] += 1

    print_result_to_junit_xml(test_results_backup, artifacts_dir)
    overall_success = True
    if failed:
        instable_list = [(k[0], k[1], v[0]) for k, v in failed.iteritems() if v[1] != 0]
        failed_list = [(k[0], k[1], v[0]) for k, v in failed.iteritems() if v[1] == 0]
        if instable_list:
            print("\nTests that are instable:")
            for name, id, fail_count in sorted(instable_list):
                print('  {} --id "{}" (failed {} times)'.format(name, id.display_id, fail_count))
        if failed_list:
            overall_success = False
            print("\nTests that failed:")
            for name, id, fail_count in sorted(failed_list):
                if fail_count > 1:
                    print('  {} --id "{}" (failed {} times)'.format(name, id.display_id, fail_count))
                else:
                    print('  {} --id "{}"'.format(name, id.display_id))
    else:
        print("\nAll tests succeeded.")
    for (dirpath, dirnames, filenames) in os.walk(artifacts_dir, topdown=False):
        if not filenames and not dirnames:
            os.rmdir(dirpath)
    return overall_success


def print_result_to_log(artifacts_dir, success, name, id, message):
    f = open(os.path.join(artifacts_dir, name + '.txt'), 'a')
    f.write("{}: {} {}\n{}\n".format("Ok" if success else "Failed", name, id.display_id, message.replace('\r\n', '\n')))
    f.flush()


def print_result_to_junit_xml(test_results, artifacts_dir):
    f = open(os.path.join(artifacts_dir, 'results.xml'), 'w')
    f.write('<?xml version="1.0" encoding="UTF-8"?>\n<testsuites>\n')
    results_per_testsuite = {}
    for name, id, result in test_results:
        if not name in results_per_testsuite:
            results_per_testsuite[name] = []
        results_per_testsuite[name].append((id.display_id, result))
    import re
    re_verifier = re.compile("^TEST_TIME: ([^ ]*)", re.MULTILINE)
    for testsuite, tests in results_per_testsuite.iteritems():
        nFailures = sum([not t[1]['success'] for t in tests])
        f.write('<testsuite name="%s" test="%d" failures="%d">\n' % (testsuite, len(tests), nFailures))
        for test_id, result in tests:
            time_verifier = re_verifier.search(result['output'])
            time_str = ' time="{}"'.format(time_verifier.group(1))
            f.write('<testcase name="%s"%s>\n' % (xml.sax.saxutils.escape(test_id), time_str))
            if not result['success']:
                f.write('<failure/>\n')
            if 'artifacts' in result:
                for artifact in result['artifacts']:
                    ref_path = os.path.abspath(artifact['reference'])
                    loc_path = os.path.abspath(artifact['location'])
                    f.write('<artifact reference="%s" location="%s" />' % (xml.sax.saxutils.escape(ref_path),
                                                                           xml.sax.saxutils.escape(loc_path)))
            f.write('<system-out>%s</system-out>\n' % xml.sax.saxutils.escape(result['output']))
            f.write('</testcase>\n')
        f.write('</testsuite>\n')
    f.write('</testsuites>')


def print_result_to_stdout(success, name, id, message, verbosity, progress):
    if success:
        if verbosity == 2:
            print('Ok: {} --id "{}"\n{}'.format(name, id.display_id, message))
        elif verbosity == 1:
            print_statusline('[{}/{}] Ok: {} --id "{}"\r'.format(progress[0], progress[1], name, id.display_id))
    else:
        if verbosity == 1:
            print_statusline('\nFailed: {} --id "{}"'.format(name, id.display_id))
            print('\n' + message)
        else:
            print('\nFailed: {} --id "{}"\n{}'.format(name, id.display_id, message))
    sys.stdout.flush()


def print_statusline(msg):
    max_len = get_terminal_size()[0] - 1
    if len(msg) > max_len:
        msg = msg[:max_len]  # avoid line breaks
    last_msg_length = len(print_statusline.last_msg) if hasattr(print_statusline, 'last_msg') else 0
    print '{}\r'.format(' ' * last_msg_length),
    print '{}\r'.format(msg),
    sys.stdout.flush()
    print_statusline.last_msg = msg


def get_terminal_size():
    """ getTerminalSize()
     - get width and height of console
     - works on linux,os x,windows,cygwin(windows)
     originally retrieved from:
     http://stackoverflow.com/questions/566746/how-to-get-console-window-width-in-python
    """
    current_os = platform.system()
    tuple_xy = None
    if current_os == 'Windows':
        tuple_xy = _get_terminal_size_windows()
        if tuple_xy is None:
            tuple_xy = _get_terminal_size_tput()
            # needed for window's python in cygwin's xterm!
    if current_os in ['Linux', 'Darwin'] or current_os.startswith('CYGWIN'):
        tuple_xy = _get_terminal_size_linux()
    if tuple_xy is None:
        print "default"
        tuple_xy = (80, 25)  # default value
    return tuple_xy


def _get_terminal_size_windows():
    try:
        from ctypes import windll, create_string_buffer
        # stdin handle is -10
        # stdout handle is -11
        # stderr handle is -12
        h = windll.kernel32.GetStdHandle(-12)
        csbi = create_string_buffer(22)
        res = windll.kernel32.GetConsoleScreenBufferInfo(h, csbi)
        if res:
            (bufx, bufy, curx, cury, wattr,
             left, top, right, bottom,
             maxx, maxy) = struct.unpack("hhhhHhhhhhh", csbi.raw)
            sizex = right - left + 1
            sizey = bottom - top + 1
            return sizex, sizey
    except:
        pass


def _get_terminal_size_tput():
    # get terminal width
    # src: http://stackoverflow.com/questions/263890/how-do-i-find-the-width-height-of-a-terminal-window
    try:
        cols = int(subprocess.check_call(shlex.split('tput cols')))
        rows = int(subprocess.check_call(shlex.split('tput lines')))
        return (cols, rows)
    except:
        pass


def _get_terminal_size_linux():
    def ioctl_GWINSZ(fd):
        try:
            import fcntl
            import termios
            cr = struct.unpack('hh',
                               fcntl.ioctl(fd, termios.TIOCGWINSZ, '1234'))
            return cr
        except:
            pass

    cr = ioctl_GWINSZ(0) or ioctl_GWINSZ(1) or ioctl_GWINSZ(2)
    if not cr:
        try:
            fd = os.open(os.ctermid(), os.O_RDONLY)
            cr = ioctl_GWINSZ(fd)
            os.close(fd)
        except:
            pass
    if not cr:
        try:
            cr = (os.environ['LINES'], os.environ['COLUMNS'])
        except:
            return None
    return int(cr[1]), int(cr[0])
