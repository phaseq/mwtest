#! python

import os
import subprocess
import shutil
import errno
import json
import re


def strip_invalid_chars(text):
    # this is a workaround for problems with various encodings across test applications
    return re.sub('[\x80-\xFF]+', '?', text)


def run_exe(params, cwd="."):
    # print('INVOKE: {}'.format(' '.join(params)))  # DEBUG
    #if not os.path.exists(params[0]):
    #    print("missing test exe: {}\nDid you forget to build?".format(params[0]))
    #    exit(1)
    try:
        cwd = os.path.normpath(cwd)
        child = subprocess.Popen(params, stdout=subprocess.PIPE, stderr=subprocess.PIPE, cwd=cwd)
        output, err = child.communicate()
        output = strip_invalid_chars(output)
        output = output.replace('\r\n', '\n').strip()
        return child.returncode, output + err
    except (OSError, ValueError):
        print("failed to run command: " + params)
        print('cwd: ' + cwd)
        raise


def move_artifacts(test_id, success, tmp_path, artifacts_dir, testcases_dir, test_id_is_dir):
    if not os.path.exists(tmp_path):
        return []
    output_is_dir = os.path.isdir(tmp_path)
    artifact_report = []
    if output_is_dir:
        artifacts = os.listdir(tmp_path)
        if len(artifacts) > 0:
            result_dir = os.path.join(artifacts_dir, ['different', 'equal'][success], test_id.rel_path)
            reference_dir = os.path.join(testcases_dir, test_id.rel_path)
            if not test_id_is_dir:
                result_dir = os.path.dirname(result_dir)
                reference_dir = os.path.dirname(reference_dir)
            if not os.path.exists(result_dir):
                os.makedirs(result_dir)
            for f in artifacts:
                if "__tmp" in f:
                    continue
                source = os.path.join(tmp_path, f)
                reference = os.path.join(reference_dir, f)
                result = os.path.join(result_dir, f)
                shutil.move(source, result)
                artifact_report.append({
                    'reference': reference,
                    'location': result})
        shutil.rmtree(tmp_path)
    else:
        source = tmp_path
        reference = os.path.join(testcases_dir, test_id.rel_path)
        result = os.path.join(artifacts_dir, ['different', 'equal'][success], test_id.rel_path)
        result_dir = os.path.dirname(result)
        if not os.path.exists(result_dir):
            os.makedirs(result_dir)
        shutil.move(source, result)
        artifact_report.append({
            'reference': reference,
            'location': result})
    return artifact_report

