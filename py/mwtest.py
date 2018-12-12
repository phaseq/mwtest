#! python

import os
import subprocess
import argparse
import sys
import inspect
import multiprocessing
from multiprocessing import Pool
import adaptors
import shutil
import uuid
import config
import shlex
from collections import namedtuple

VERBOSITY_DEFAULT = 1
XGE_TIMEOUT_IN_SECONDS = 6 * 60


def guess_static_paths(given_testcases_root):
    build_path = None
    build_type = None
    testcases_root = None
    preset = None
    output_dir = os.path.abspath('test_output')

    dev_root = find_dev_root_path()
    if dev_root is not None:
        build_path = os.path.join(dev_root, 'dev')
        build_type = "dev-releaseunicode"
        testcases_root = os.path.join(dev_root, 'testcases')
        preset = 'ci'
        if not os.path.exists(testcases_root):
            testcases_root = None
            preset = None
    cwd_files = os.listdir('.')
    if "mwVerifier.dll" in cwd_files and "5axutil.dll" in cwd_files:
        build_path = os.path.abspath('.')
        build_type = 'quickstart'
    if given_testcases_root:
        testcases_files = os.listdir(given_testcases_root)
        if "cutsim" in testcases_files or \
                "5axis" in testcases_files or \
                "machsim" in testcases_files or \
                "cncsim" in testcases_files or \
                "CollisionChecker" in testcases_files:
            preset = 'ci'
        else:
            preset = 'all'
    return build_path, build_type, testcases_root, preset, output_dir


def find_dev_root_path():
    cwd = os.path.abspath('.').split(os.sep)
    if b'dev' in cwd:
        dev_idx = cwd.index(b'dev')
        return '/'.join(cwd[:dev_idx]) + '/'
    else:
        return None


def resolve_test_paths(args):
    build_path, build_type, testcases_root, preset, output_dir = guess_static_paths(args.testcases_dir_path)
    if args.build_dir_path is None:
        args.build_dir_path = build_path
    args.build_dir_path = os.path.abspath(os.path.normpath(args.build_dir_path))
    if args.build_type is None:
        args.build_type = build_type
    if args.testcases_dir_path is None:
        args.testcases_dir_path = testcases_root
    if args.testcases_dir_path is not None:
        args.testcases_dir_path = os.path.abspath(os.path.normpath(args.testcases_dir_path))
    if args.preset is None:
        args.preset = preset
    if args.output_dir_path is None:
        args.output_dir_path = output_dir
    args.output_dir_path = os.path.abspath(os.path.normpath(args.output_dir_path))

    if not args.build_dir_path:
        print "you need to specify a build directory (--build-dir)"
        exit(-1)
    if not args.build_type:
        print "you need to specify a build type (--build-type)"
        exit(-1)
    if not args.testcases_dir_path:
        print "you need to specify a testcases directory (--testcases-dir)"
        exit(-1)


# the script currently expects the directory layout to be equal to SVN.

def generate_tmp_dir(artifacts_dir):
    if not artifacts_dir:
        return None
    return os.path.join(artifacts_dir, 'tmp', uuid.uuid4().hex)


def run_one(test_id, test_group):
    cmd, cwd, tmp_dir = command_for(test_id, test_group)
    return_code, output = adaptors.run_exe(cmd, cwd=cwd)
    result = process_result(test_id, return_code, output, tmp_dir, test_group.artifacts_path,
                            test_group.testcases_path, test_group.app_properties.input_is_dir)
    return test_group.app_name, test_id, result


def process_result(test_id, returncode, output, tmp_path, artifacts_path, testcases_path, input_is_dir):
    success = returncode == 0
    artifact_report = []
    if tmp_path:
        artifact_report = adaptors.move_artifacts(test_id, success, tmp_path, artifacts_path, testcases_path,
                                                  input_is_dir)
    return {'success': success, 'output': output, 'artifacts': artifact_report}


def command_for(test_id, test_group):
    tmp_dir = generate_tmp_dir(test_group.artifacts_path)
    input = os.path.join(test_group.testcases_path,
                         test_id.rel_path) if test_group.testcases_path else test_id.display_id
    cmd, cwd = test_group.app_properties.get_command_for(input, tmp_dir)
    return cmd, cwd, tmp_dir


def run_one_multiprocessing(args):
    return list(run_one(*args))


def run(test_group):
    for t in test_group.test_ids:
        yield run_one(t, test_group)


def run_in_pool(test_group, pool):
    return pool.imap_unordered(run_one_multiprocessing, ((t, test_group) for t in test_group.test_ids))


def run_all(tests_with_ids):
    for test_group in tests_with_ids:
        for result in run(test_group):
            yield result


def count_test_ids(tests_with_ids):
    return sum(len(test_group.test_ids) for test_group in tests_with_ids)


def init_worker():
    import signal
    signal.signal(signal.SIGINT, signal.SIG_IGN)


def run_all_multiprocessing(tests_with_ids):
    concurrency = multiprocessing.cpu_count()
    pool = Pool(concurrency, init_worker)
    try:
        for test_group in tests_with_ids:
            if test_group.parallel:
                for result in run_in_pool(test_group, pool):
                    yield result
            else:
                for result in run(test_group):
                    yield result
    except KeyboardInterrupt:
        pool.terminate()
        raise


def run_all_xge(test_apps_with_ids, repeat_if_failed):
    xge_tests, local_tests = split_by_xge(test_apps_with_ids)

    queue = XGEQueue()
    for test_group in local_tests + xge_tests:
        for test_id in test_group.test_ids:
            queue.enqueue(test_id, test_group, not test_group.xge)

    repeats = {}

    while queue.has_tasks():
        result = queue.get_result()
        if not result:
            break

        test_id, test_group, tmp_dir, returncode, output = result
        result = process_result(test_id, returncode, output, tmp_dir,
                                test_group.artifacts_path,
                                test_group.testcases_path,
                                test_group.app_properties.input_is_dir)

        if not result['success']:
            n_repeated = repeats.get(test_id, 0)
            if n_repeated < repeat_if_failed:
                repeats[test_id] = n_repeated + 1
                queue.enqueue(test_id, test_group, not test_group.xge)

        yield test_group.app_name, test_id, result

    for _id, (test_id, test_group, _tmp_dir) in queue.queued_tests.iteritems():
        yield test_group.app_name, test_id, {'success': False, 'output': 'Failed to start!'}


class XGEQueue:
    def __init__(self):
        import xge
        self.processor = xge.XGE()
        self.queued_tests = {}
        self.queue_id_counter = 0

    def __del__(self):
        self.processor.close()

    def enqueue(self, test_id, test_group, local):
        cmd, cwd, tmp_dir = command_for(test_id, test_group)
        queue_id = self.queue_id_counter
        self.queue_id_counter += 1
        self.queued_tests[queue_id] = (test_id, test_group, tmp_dir)
        if local:
            self.processor.run_local(str(queue_id), test_id.display_id, shlex.split(cmd, posix=False), cwd)
        else:
            self.processor.run(str(queue_id), test_id.display_id, shlex.split(cmd, posix=False), cwd)

    def has_tasks(self):
        return self.queued_tests != []

    def get_result(self):
        result = self.processor.poll(XGE_TIMEOUT_IN_SECONDS)
        if not result:
            return None
        queue_id_str, returncode, output = result
        queue_id = int(queue_id_str)

        if queue_id in self.queued_tests:
            test_id, test_group, tmp_dir = self.queued_tests[queue_id]
            del self.queued_tests[queue_id]
            return (test_id, test_group, tmp_dir, returncode, output)
        return None



def split_by_xge(tests_with_ids):
    xge_tests, local_tests = [], []
    for test_group in tests_with_ids:
        if test_group.xge:
            xge_tests.append(test_group)
        else:
            local_tests.append(test_group)
    return xge_tests, local_tests


def args_to_filter(args):
    if hasattr(args, 'id') and args.id:
        return lambda (id, _): id == args.id
    if not args.filter:
        return lambda _: True
    args.filter = [f.replace('\\', '/') for f in args.filter]
    return lambda (id, _): any(id.lower().find(f.lower()) != -1 for f in args.filter)


def filter_test_ids(preset_group, args):
    if args.id:
        for t in preset_group.get_tests():
            if args.id == t.display_id:
                yield t
                return
        return

    is_included = args_to_filter(args)
    for t in preset_group.get_tests():
        if is_included(t):
            yield t


def prepare_artifacts_dir(args):
    output_path = args.output_dir_path
    print("Test artifacts will be written to {}".format(output_path))
    if os.path.exists(output_path):
        shutil.rmtree("\\\\?\\" + output_path)
    os.makedirs(output_path + '/tmp')
    return output_path


def repeat_artifacts_path(artifacts_dir, repeat):
    if repeat > 1:
        artifact_paths = []
        for i in range(repeat):
            path = artifacts_dir + '/' + str(i)
            artifact_paths.append(path)
        return artifact_paths
    else:
        return [artifacts_dir]


TestGroup = namedtuple("TestGroup",
                       ['app_name', 'app_properties', 'test_ids', 'artifacts_path', 'testcases_path', 'parallel',
                        'xge'])


def get_filtered_tests(args):
    tests_with_ids = []
    artifacts_dir = args.output_dir_path

    repeat = args.repeat if hasattr(args, 'repeat') else 1
    artifact_paths = repeat_artifacts_path(artifacts_dir, repeat)

    static_paths = config.StaticPaths.from_args(args)
    app_properties = config.load_app_properties_default(static_paths.files)
    preset = config.load_preset(args.preset, static_paths, app_properties)

    for test in args.test:
        if not os.path.exists(static_paths.files[test]["exe"]):
            print("Error: could not find test executable for {} at {}.\n"
                  "Did you forget to build?".format(test,
                                                    static_paths.files[test]["exe"]))
            exit(-1)

    test_names = app_properties.keys() if args.test == ['all'] else args.test

    for app_name, test_groups in preset.iteritems():
        if app_name not in test_names:
            continue

        for test_group in test_groups:
            test_ids = list(filter_test_ids(test_group, args))
            if test_ids == []:
                continue
            for artifacts_path in artifact_paths:
                tests_with_ids.append(TestGroup(
                    app_name=app_name,
                    app_properties=app_properties[app_name],
                    test_ids=test_ids,
                    artifacts_path=None if test_group.is_gtest else artifacts_path,
                    testcases_path=None if test_group.is_gtest else static_paths.testcases_dir,
                    parallel=test_group.parallel,
                    xge=test_group.xge
                ))
    return tests_with_ids, artifacts_dir


def cmd_list(args):
    if len(args.test) == 0:
        registered_tests = config.get_registered_tests()
        for t in registered_tests:
            print(t)
    else:
        tests_with_ids, _ = get_filtered_tests(args)
        for test_group in tests_with_ids:
            for test_id in test_group.test_ids:
                print('{} --id "{}"'.format(test_group.app_name, test_id.display_id))


def cmd_run(args):
    if len(args.test) == 0:
        print "you have to run at least one test!"
        exit(1)
    prepare_artifacts_dir(args)
    tests_with_ids, artifacts_dir = get_filtered_tests(args)
    verbosity = VERBOSITY_DEFAULT
    if args.verbose:
        verbosity = 2
    if args.quiet:
        verbosity = 0
    result = None
    if args.id and not args.xge:
        if len(args.test) != 1:
            print("when using --id you have specify exactly one test application")
            exit(1)
        else:
            result = run_all(tests_with_ids)
    elif args.xge:
        result = run_all_xge(tests_with_ids, repeat_if_failed=args.repeat_if_failed)
    elif args.local_parallel:
        result = run_all_multiprocessing(tests_with_ids)
    else:
        result = run_all(tests_with_ids)
    test_count = count_test_ids(tests_with_ids)
    from report import report_results
    return report_results(result, artifacts_dir, verbosity, test_count)


def cmd_debug(args):
    if len(args.test) != 1:
        print("you have to pass exactly one test framework!")
        exit(1)
    if not args.id:
        print("argument --id required!")
        exit(1)
    tests_with_ids, artifacts_dir = get_filtered_tests(args)
    for test_group in tests_with_ids:
        for test_id in test_group.test_ids:
            prepare_artifacts_dir(args)
            cmd, cwd, tmp_dir = command_for(test_id, test_group)

            print("command:\n{}\n".format(cmd))
            print("working directory:\n{}".format(cwd))

            start_debugger = os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())),
                                          'StartDebugger.exe')
            debug_command = start_debugger + ' ' + cmd
            subprocess.call(debug_command, cwd=cwd)


def cmd_build(args):
    static_paths = config.StaticPaths.from_args(args)
    to_build = {}
    for app_name, values in static_paths.files.iteritems():
        if app_name in args.test:
            solution = os.path.join(static_paths.build_dir, values['solution'])
            if solution not in to_build:
                to_build[solution] = []
            to_build[solution].append(values['project'])
    for solution, projects in to_build.items():
        projects = ','.join(projects)
        print("solution: {}".format(solution))
        print("  -> building projects: {}...".format(projects))
        cmd = ['buildConsole', solution, '/build', '/silent', '/cfg=ReleaseUnicode|x64', '/prj=' + projects,
               '/openmonitor']
        subprocess.call(cmd)


"""
def cmd_checkout(args):
    print("THIS IS NOT IMPLEMENTED YET")
    dev_dir = args.build_dir_path
    svn_root = subprocess.Popen(['svn', 'info', dev_dir, '--show-item=url'], stdout=subprocess.PIPE).communicate()[
        0].strip()
    testcases_root = args.testcases_dir_path
    with open(args.preset) as f:
        preset = json.load(f)
        for app in args.test:
            if app not in preset:
                continue
            for testcase in preset[app]:
                if 'path' not in testcase:
                    continue
                rel_dir = testcase['path']
                svn_dir = svn_root + '/../' + rel_dir
                fs_dir = testcases_root + '/' + rel_dir
                print 'directory in SVN:', svn_dir
                print 'directory in filesystem:', fs_dir
"""


def cli():
    registered_tests = config.get_registered_tests()

    parser = argparse.ArgumentParser(description="ModuleWorks CI Tests")
    parser.add_argument('--build-dir', dest='build_dir_path', help='the path to the build directory (usually /dev/)')
    parser.add_argument('--testcases-dir', dest='testcases_dir_path', help='the path to the testcases directory')
    parser.add_argument('--output-dir', '-o', dest='output_dir_path', help='the path to the output directory')
    parser.add_argument('--preset', '-p', help='description of what tests to run (see presets folder)')
    parser.add_argument('--build-type', '-b', dest='build_type', help='can be "dev-releaseunicode" or "quickstart"')

    parser_command = parser.add_subparsers()

    parser_list = parser_command.add_parser('list')
    parser_list.add_argument('test', nargs='*', choices=registered_tests + [[], 'all'],
                             help="the test applications you want to run")
    parser_list.add_argument('--filter', '-f', nargs='*', help="every test that contains this string will be selected")
    parser_list.add_argument('--id', '-i')
    parser_list.set_defaults(func=cmd_list)

    parser_run = parser_command.add_parser('run')
    parser_run.add_argument('test', nargs='+', choices=registered_tests + ['all'],
                            help="the test applications you want to run")
    parser_run.add_argument('--filter', '-f', nargs='*',
                            help="every test id that contains this string will be selected. If multiple filters are specified it is sufficient if one of them matches.")
    parser_run.add_argument('--id', '-i', help="the test id you want to run")
    parallelize_group = parser_run.add_mutually_exclusive_group()
    parallelize_group.add_argument('--xge', '-x', action='store_true', help="run tests via XGE")
    parallelize_group.add_argument('--local-parallel', '-p', action='store_true', help="run tests locally in parallel")
    parser_run.add_argument('--repeat', '-r', type=int, default=1)
    parser_run.add_argument('--repeat-if-failed', type=int, default=0)
    verbosity_group = parser_run.add_mutually_exclusive_group()
    verbosity_group.add_argument('--verbose', '-v', action='store_true', help="show full test results also on success")
    verbosity_group.add_argument('--quiet', '-q', action='store_true', help="don't report succeeded tests")
    parser_run.set_defaults(func=cmd_run)

    parser_debug = parser_command.add_parser('debug')
    parser_debug.add_argument('test', choices=registered_tests, help="the test application you want to run")
    parser_debug.add_argument('--id', '-i', help="the test id you want to run")
    parser_debug.add_argument('--vs', type=int, choices=[2008, 2010, 2012, 2013, 2015, 2017],
                              help="the visual studio version you want to launch")
    parser_debug.set_defaults(func=cmd_debug)

    parser_build = parser_command.add_parser('build')
    parser_build.add_argument('test', nargs='+', choices=registered_tests + ['all'],
                              help="the test applications you want to run")
    parser_build.set_defaults(func=cmd_build)

    # parser_checkout = parser_command.add_parser('checkout')
    # parser_checkout.add_argument('test', nargs='+', choices=registered_tests + ['all'],
    #                             help="the test applications you want to run")
    # parser_checkout.set_defaults(func=cmd_checkout)

    args = parser.parse_args(sys.argv[1:])
    if not isinstance(args.test, list):
        args.test = [args.test]

    resolve_test_paths(args)

    if not os.path.exists(args.preset):
        preset = os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())), 'presets/%s.json' % args.preset)
        if not os.path.exists(preset):
            print("Can't find preset: {}".format(args.preset))
            exit(1)
        args.preset = preset
    if not os.path.exists(args.build_type):
        build_type = os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())),
                                  'build_types/%s.json' % args.build_type)
        if not os.path.exists(build_type):
            print("Can't find build_type: {}".format(args.build_type))
            exit(1)
        args.build_type = build_type

    try:
        args.func(args)
    except KeyboardInterrupt:
        print("\nuser aborted.")
        exit(1)


if __name__ == '__main__':
    cli()
