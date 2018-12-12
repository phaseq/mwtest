#! python

import argparse
from xml.dom import minidom
import subprocess
#import msvcrt
import os
import inspect
import Tkinter
import shutil


def view_contours_rhino(rhino_path, reference, location):
    rhino_tmp = os.path.abspath('rhino.txt')
    f = open(rhino_tmp, 'w')
    f.write("""
-SetMaximizedViewport Top
-Layer New Reference Current Reference _enter
-ReadCommandFile %s
-Layer New Different Current Different Color Different 255,0,0 _enter
-ReadCommandFile %s""" % (reference, location))
    f.close()
    subprocess.Popen([rhino_path, rhino_tmp])


def view_wp_vas(vas_path, file):
    subprocess.Popen([vas_path, '/file', file])


def view_stl_rhino(rhino_path, reference, location):
    rhino_tmp = os.path.abspath('rhino.txt')
    f = open(rhino_tmp, 'w')
    f.write("""
-Layer New Reference Current Reference _enter
-Import %s _enter
-Layer New Different Current Different Color Different 255,0,0 _enter
-Import %s _enter""" % (reference, location))
    f.close()
    subprocess.Popen([rhino_path, rhino_tmp])


def view_stl_vas(vas_path, reference, location):
    nc_tmp = os.path.abspath('compare.nc')
    f = open(nc_tmp, 'w')
    f.write("STOCKFILE %s;\nTARGETFILE %s;" % (reference, location))
    f.close()
    subprocess.Popen([vas_path, nc_tmp])


def view_stl_viscam(viscam_path, file):
    subprocess.Popen([viscam_path, file])


def view_diff_tortoise(tortoise_path, reference, location):
    file_name = os.path.basename(reference)
    cmd = '"{}" /base:"{}" /basename:"Reference:{}" /theirs:"{}" /theirsname:"Different:{}"'.format(tortoise_path, reference, file_name, location, file_name)
    subprocess.Popen(cmd)


def view_sampleintegration(sampleintegration_path, reference, location):
    cmd = '"{}" "{}" "{}" /w'.format(sampleintegration_path, reference, location)
    subprocess.Popen(cmd)


def view_explorer(location):
    if not os.path.isdir(location):
        location = os.path.dirname(location)
    subprocess.Popen('explorer "' + location + '"')


def guess_file_type(file_name):
    extension = os.path.splitext(file_name)[1]
    if extension == '.wp':
        return 'verifier-wp'
    if extension == '.stl':
        return 'stl'
    if extension == '.txt' or extension == '.xml':
        with open(file_name) as f:
            content = f.read()
            if content.find('_Line') != -1 or content.find('_Arc') != -1 or content.find('_Points') != -1:
                return 'rhino-text'
        return 'text'
    if extension == '.bin':
        return 'exactoutput-bin'
    return 'unknown'


def parse_config_file(config_file):
    with open(config_file) as f:
        comparers = [tuple(line.split('=', 1)) for line in f.read().splitlines()]
        comparers = dict(comparers)
        return comparers


def view(reference, location, comparers, dialog):
    choices = []
    type = guess_file_type(reference)
    if type == 'verifier-wp':
        choices.append(('1', 'VAS comparison',
                        lambda: subprocess.Popen([comparers['vas'], '/compare', location, '/file', reference])))
        choices.append(('2', 'VAS (different)',
                        lambda: view_wp_vas(comparers['vas'], location)))
        choices.append(('3', 'VAS (reference)',
                        lambda: view_wp_vas(comparers['vas'], reference)))
    elif type == 'rhino-text':
        choices.append(('1', 'Rhino comparison', lambda: view_contours_rhino(comparers['rhino'], reference, location)))
        choices.append(('2', 'Tortoise', lambda: view_diff_tortoise(comparers['tortoiseDiff'], reference, location)))
    elif type == 'stl':
        choices.append(('1', 'Rhino', lambda: view_stl_rhino(comparers['rhino'], reference, location)))
        choices.append(('2', 'VAS G&E', lambda: view_stl_vas(comparers['vas'], reference, location)))
        choices.append(('3', 'VisCAM (different)', lambda: view_stl_viscam(comparers['viscam'], location)))
        choices.append(('4', 'VisCAM (reference)', lambda: view_stl_viscam(comparers['viscam'], reference)))
    elif type == 'text':
        choices.append(('1', 'Tortoise', lambda: view_diff_tortoise(comparers['tortoiseDiff'], reference, location)))
    elif type == 'exactoutput-bin':
        choices.append(('1', 'Sample Integration', lambda: view_sampleintegration(comparers['sampleintegration'], reference, location)))
    elif type == 'unknown':
        choices.append(('1', 'Tortoise', lambda: view_diff_tortoise(comparers['tortoiseDiff'], reference, location)))
    else:
        print "type not supported:", type
        exit(1)

    def reset_and_update(location, reference, dialog):
        shutil.move(location, reference)
        dialog.prev_now = None
    choices.append(('r', 'reset', lambda: reset_and_update(location, reference, dialog)))
    return choices


class Dialog(Tkinter.Frame):
    def __init__(self, report, comparers):
        self.report = report
        self.comparers = comparers

        self.master = Tkinter.Tk()
        Tkinter.Frame.__init__(self, self.master)
        self.main_option = None

        self.master.resizable(False, False)
        self.master.title('MWTest Comparison')

        list_frame = Tkinter.Frame(self.master, width=50, height=50)
        list_frame.pack(side='left', fill='y', expand=1, anchor='w')
        self.list = Tkinter.Listbox(list_frame, width=50, height=50)
        self.list.pack(side='left', fill='y', expand=1)
        scrollbar = Tkinter.Scrollbar(list_frame, command=self.list.yview)
        scrollbar.pack(side='right', fill='y', expand=1)
        self.list['yscrollcommand'] = scrollbar.set

        self.info_frame = Tkinter.Frame(self.master)
        self.info_frame.pack(side='right', fill='y')
        label_frame = Tkinter.Frame(self.info_frame)
        label_frame.pack(fill='y')
        self.label = Tkinter.Text(label_frame, height=35)
        self.label.pack(side='left', fill='y')
        label_scrollbar = Tkinter.Scrollbar(label_frame, command=self.label.yview)
        label_scrollbar.pack(side='right', fill='y')
        self.label['yscrollcommand'] = label_scrollbar.set

        self.prev_now = None
        self.buttons = []

        for test in self.report:
            self.list.insert(Tkinter.END, test["app"] + '  ' + test["name"])

        self.master.bind('<Return>', self.pressed_return)
        self.poll()
        Tkinter.mainloop()

    def poll(self):
        now = self.list.curselection()
        if len(now) > 0 and self.prev_now != now:
            self.prev_now = now
            for b in self.buttons:
                b.pack_forget()
            self.buttons = []
            self.main_option = None
            test = self.report[now[0]]
            self.label.delete('1.0', Tkinter.END)
            self.label.insert(Tkinter.END, test['system-out'])

            for artifact in test['artifacts']:
                reference = artifact['reference']
                location = artifact['artifact']
                if not os.path.isabs(reference):
                    reference = os.path.abspath(os.path.join(self.xml_dir, reference))
                if not os.path.isabs(location):
                    location = os.path.abspath(os.path.join(self.xml_dir, location))
                if artifact == test['artifacts'][0]:
                    button_frame = Tkinter.Frame(self.info_frame, padx=5, pady=5)
                    button_frame.pack(fill='x')
                    self.buttons.append(button_frame)
                    button_reference = Tkinter.Button(button_frame,
                                                      text="Open reference location",
                                                      command=lambda: view_explorer(artifact["reference"]),
                                                      padx=10)
                    button_reference.pack(side='left')
                    self.buttons.append(button_reference)
                    button_artifact = Tkinter.Button(button_frame,
                                                     text="Open artifact location",
                                                     command=lambda: view_explorer(artifact["artifact"]),
                                                     padx=10)
                    button_artifact.pack(side='left')
                    self.buttons.append(button_reference)

                if os.path.exists(artifact["artifact"]):
                    options = view(artifact["reference"], artifact["artifact"], self.comparers, self)
                    button_frame = Tkinter.Frame(self.info_frame, padx=5, pady=5)
                    button_frame.pack(fill='x')
                    self.buttons.append(button_frame)
                    Tkinter.Label(button_frame, text=os.path.basename(artifact["reference"])).pack(side='left')
                    for option in options:
                        if self.main_option == None:
                            self.main_option = option[2]
                        button = Tkinter.Button(button_frame, text=option[1], command=option[2], padx=10)
                        button.pack(side='left')
                        self.buttons.append(button)
        self.after(250, self.poll)

    def pressed_return(self, event):
        if self.main_option is not None:
            self.main_option()


def load_report(file_name, testcases_root):
    tests = []
    doc = minidom.parse(file_name)
    testsuites = doc.getElementsByTagName('testsuite')
    for testsuite in testsuites:
        test_name = testsuite.attributes['name'].value
        for test in testsuite.getElementsByTagName('testcase'):
            if len(test.getElementsByTagName('error')) + len(test.getElementsByTagName('failure')) == 0:
                continue
            artifact = test.getElementsByTagName("artifact")[0]
            reference_path = artifact.attributes['reference'].value
            artifact_path = artifact.attributes['location'].value
            if not os.path.isabs(artifact_path):
                artifact_path = os.path.abspath(os.path.join(os.path.dirname(file_name), artifact_path))
            if not os.path.isabs(reference_path):
                if not testcases_root:
                    print("you need to specify --testcases-dir!")
                    exit(1)
                reference_path = os.path.abspath(os.path.join(testcases_root, reference_path))
            if os.path.isfile(reference_path) and os.path.isdir(artifact_path):
                reference_path = os.path.dirname(reference_path)

            artifacts = []
            if os.path.isdir(artifact_path):
                for (dirpath, dirnames, filenames) in os.walk(artifact_path):
                    rel_dirpath = os.path.relpath(dirpath, artifact_path)
                    for f in filenames:
                        artifacts.append({
                            "reference": os.path.join(reference_path, rel_dirpath, f),
                            "artifact": os.path.join(artifact_path, rel_dirpath, f)
                        })
            else:
                artifacts.append({
                    "reference": reference_path,
                    "artifact": artifact_path
                })
            test_result = {
                "app": test_name,
                "name": test.attributes['name'].value,
                "system-out": test.getElementsByTagName("system-out")[0].firstChild.nodeValue,
                "artifacts": artifacts
            }
            tests.append(test_result)

    return tests


def comparers_from_args(args):
    if not args.config:
        args.config = os.path.join(os.path.dirname(inspect.getfile(inspect.currentframe())), 'comparer_paths.ini')
    if not os.path.exists(args.config):
        print('Could not find config file at {}.\nYou can use comparer_paths.example.ini '
              'as a basis and customize the paths.'.format(args.config))
        exit(1)
    comparers = parse_config_file(args.config)
    import distutils.spawn
    for k, v in comparers.iteritems():
        if not distutils.spawn.find_executable(v):
            print('WARNING: Could not find comparer "{}" at {}.'.format(k, v))
    return comparers


def find_report():
    path = os.path.join('test_output', 'results.xml')
    if os.path.exists(path):
        return path
    return None


def cli():
    parser = argparse.ArgumentParser(description="MWTest Result Comparison")
    parser.add_argument('file', nargs='?', default=find_report(), help='location of the mwtest XML file')
    parser.add_argument('--config', '-c', help='the config file containing the paths to the comparison tools')
    parser.add_argument('--testcases-dir')
    args = parser.parse_args()

    if not args.file:
        args.file = find_report()
        print args.file
        if not os.path.exists(args.file):
            print("could not find results.xml!")
            exit(1)
    elif not os.path.exists(args.file):
        print("could not find results.xml at {}!".format(args.file))
        exit(1)

    comparers = comparers_from_args(args)
    report = load_report(args.file, args.testcases_dir)
    Dialog(report, comparers)


if __name__ == '__main__':
    cli()
