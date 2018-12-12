import json
from collections import namedtuple
import os
import re
import subprocess
import inspect


def get_registered_tests():
    path = os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())), 'apps.json')
    with open(path) as f:
        return json.load(f).keys()


def load_app_properties_default(build_paths):
    return load_app_properties(
        os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())), 'apps.json'), build_paths)


def load_app_properties(file_name, build_paths):
    results = {}
    with open(file_name) as f:
        preset = json.load(f)
        for app, properties in preset.iteritems():
            if app in build_paths:
                results[app] = AppProperties(properties, build_paths[app])
    return results


class AppProperties(object):
    def __init__(self, properties, build_path):
        self._command = properties["command"]
        for k, v in build_path.iteritems():
            self._command = self._command.replace('{{' + k + '}}', v)
        self._cwd = build_path["cwd"] if "cwd" in build_path else None
        self.input_is_dir = properties.get("input_is_dir", False)

    def get_command_for(self, input, out_dir):
        cwd = self._cwd
        if not cwd:
            if self.input_is_dir:
                cwd = input
            else:
                cwd = os.path.dirname(input)
        command = self._command.replace("{{input}}", input)
        if out_dir:
            if command.find("{{out_dir}}") >= 0:
                os.makedirs(out_dir)
                command = command.replace("{{out_dir}}", out_dir)
            else:
                command = command.replace("{{out_file}}", out_dir)
        return command, cwd


class StaticPaths(object):
    def __init__(self, build_dir, testcases_dir, build_type, artifacts_dir):
        self.build_dir = os.path.normpath(build_dir)
        self.testcases_dir = os.path.normpath(testcases_dir)
        self.build_type = build_type
        self.files = json.load(open(build_type))
        for test_name, paths in self.files.iteritems():
            for k, v in paths.iteritems():
                if k == 'project':
                    self.files[test_name][k] = v
                else:
                    self.files[test_name][k] = os.path.join(build_dir, v)
        self.artifacts_dir = artifacts_dir

    @classmethod
    def from_args(cls, args):
        return StaticPaths(args.build_dir_path, args.testcases_dir_path, args.build_type, args.output_dir_path)


TestId = namedtuple('TestId', ['display_id', 'rel_path'])


def load_preset(preset_file_name, static_paths, app_properties):
    results = {}
    with open(preset_file_name) as f:
        preset = json.load(f)
        for app, groups in preset.iteritems():
            if app in static_paths.files:
                results[app] = [PresetGroup(group,
                                            static_paths.files[app]['exe'],
                                            static_paths.testcases_dir,
                                            app_properties[app].input_is_dir) for group in groups]
    return results


class PresetGroup:
    def __init__(self, group, build_path, testcases_dir, input_is_dir):
        self.globber = json_to_preset_group(group, build_path, testcases_dir)
        self.is_gtest = 'find_gtest' in group
        self.parallel = group.get('parallel', True)
        self.xge = group.get('xge', True)
        self.input_is_dir = input_is_dir

    def get_tests(self):
        tests = list(self.globber.get_tests())
        if self.input_is_dir:
            tests = [TestId(display_id=os.path.dirname(t.display_id),
                            rel_path=os.path.dirname(t.rel_path)) for t in tests]
        return tests


def json_to_preset_group(json, exe_path, testcases_dir):
    if "find_glob" in json:
        return FileGlob(json["find_glob"], testcases_dir, json["id_pattern"])
    else:
        return GTestGlob(json["find_gtest"], exe_path)


class FileGlob:
    def __init__(self, glob_expr, testcases_dir, pattern):
        self.glob_expr = glob_expr
        self.testcases_dir = testcases_dir
        self.pattern = pattern

    def get_tests(self):
        import fnmatch
        exp = re.compile(self.pattern)
        # for abs_path in glob.iglob(self.testcases_dir + '/' + self.glob_expr, recursive=True):
        root_dir = self.testcases_dir
        if self.glob_expr.find('**') >= 0:
            root_dir = os.path.join(root_dir, self.glob_expr.split('**')[0])
        else:
            root_dir = os.path.dirname(os.path.join(root_dir, self.glob_expr))

        for root, dirs, files in os.walk(root_dir):
            for f in files:
                abs_path = os.path.join(root, f)
                rel_path = os.path.relpath(abs_path, self.testcases_dir).replace('\\', '/')
                if fnmatch.fnmatch(rel_path, self.glob_expr) or \
                        (self.glob_expr.startswith('**/') and fnmatch.fnmatch(rel_path, self.glob_expr[3:])):
                    id = exp.match(rel_path).group(1)
                    yield TestId(display_id=id, rel_path=os.path.normpath(rel_path))


class GTestGlob:
    def __init__(self, gtest_filter, exe_path):
        self.gtest_filter = gtest_filter
        self.exe_path = exe_path

    def get_tests(self):
        tests = []
        test_list_output = subprocess.Popen(
            [self.exe_path, '--gtest_list_tests', '--gtest_filter=' + self.gtest_filter], stdout=subprocess.PIPE,
            stderr=subprocess.PIPE)
        output, err = test_list_output.communicate()
        group = ""
        for line in output.split('\n'):
            line = line.split('#')[0]
            if not line.startswith(' '):
                group = line.strip()
            else:
                test_name = group + line.strip()
                if test_name.find('DISABLED') == -1:
                    tests.append(TestId(display_id=test_name, rel_path=None))
        return tests
