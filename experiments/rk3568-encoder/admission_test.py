#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-2.0
"""Actual external H264 admission/finish paths; host framework doubles, no device.

This checks the source admission boundary, not DMA quiescence. Internal codec/job
tests deliberately exercise unreachable preparatory functions separately.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.dont_write_bytecode = True
from prestart_test import function


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('kernel', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--sanitize', action='store_true')
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    driver = args.kernel / 'drivers/media/platform/verisilicon'
    source = (driver / 'hantro_drv.c').read_text()
    ops = source.split('static const struct v4l2_m2m_ops vpu_m2m_ops = {')[1].split('};')[0]
    assert '.device_run = hantro_device_run,' in ops
    subject = source[:source.index('#include')]
    subject += function(source, 'hantro_job_finish_no_pm') + '\n'
    subject += function(source, 'hantro_device_run') + '\n'
    (args.output / 'admission_subject.inc').write_text(subject, newline='\n')
    cc = os.environ.get('CC') or shutil.which('cc') or shutil.which('clang')
    if not cc and os.name == 'nt':
        cc = 'C:/Program Files (x86)/Microsoft Visual Studio/2022/BuildTools/VC/Tools/Llvm/ARM64/bin/clang.exe'
    assert cc
    binary = args.output / ('admission-test.exe' if os.name == 'nt' else 'admission-test')
    test = Path(__file__).with_suffix('.c')
    shim = args.output / 'host-shims/linux'
    shim.mkdir(parents=True, exist_ok=True)
    (shim / 'types.h').write_text('#include <stdint.h>\ntypedef uint64_t __u64;\n')
    command = [cc, '-std=c11', '-O1', '-Wall', '-Wextra', '-Werror', '-Wno-unused-parameter',
               '-I', str(shim.parent), '-I', str(driver), '-I', str(args.output)]
    if args.sanitize:
        command += ['-fsanitize=address,undefined', '-g']
    if os.name == 'nt':
        command += ['-Xlinker', '/MANIFEST:EMBED', '-Xlinker',
                    "/MANIFESTUAC:level='asInvoker' uiAccess='false'"]
    command += [str(test), '-o', str(binary)]
    subprocess.run(command, check=True, timeout=60)
    result = subprocess.run([str(binary.resolve())], check=True, capture_output=True,
                            text=True, timeout=30)
    evidence = {'scope': __doc__, 'command': command, 'output': result.stdout,
                'sanitize': args.sanitize,
                'source_sha256': hashlib.sha256(source.encode()).hexdigest(),
                'subject_sha256': hashlib.sha256(subject.encode()).hexdigest(),
                'test_sha256': hashlib.sha256(test.read_bytes()).hexdigest(),
                'header_sha256': hashlib.sha256((driver / 'hantro_h264_enc_ref.h').read_bytes()).hexdigest()}
    (args.output / 'result.json').write_text(json.dumps(evidence, indent=2) + '\n')
    print(result.stdout, end='')


if __name__ == '__main__':
    main()
