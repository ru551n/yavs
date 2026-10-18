#!/usr/bin/env python3
"""Run NVC and GHDL over tests/diag/sem/**/*.vhd and print one TSV row per file.

Columns: path, expected (code@line ...), nvc (OK|REJ|WARN + error lines), ghdl (same).
Work dirs live under ~/.cache/rsim-negtests, never in the repo. Max 8 in parallel.
Usage: check_ref.py [-j N] [files...]   (default: all files)
"""
import glob, os, re, shutil, subprocess, sys
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.abspath(__file__))
CACHE = os.path.expanduser('~/.cache/rsim-negtests/work')


def header(path):
    std, flags, exp = '2008', [], []
    for l in open(path):
        if not l.startswith('--'):
            break
        m = re.match(r'-- yavs-test:(.*)', l)
        if m:
            for t in m.group(1).split():
                if t.startswith('std='):
                    std = t[4:]
                elif t.startswith('flags='):
                    flags += t[6:].split(',')
        m = re.match(r'-- expect: ([EW]\d{4}) (\d+):(\d+)', l)
        if m:
            exp.append((m.group(1), int(m.group(2))))
    return std, flags, exp


def run(cmd, cwd):
    full = ['timeout', '60', 'systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=1G'] + cmd
    p = subprocess.run(full, cwd=cwd, capture_output=True, text=True)
    return p.returncode, p.stdout + p.stderr


def one(path):
    path = os.path.abspath(path)  # the tools run in a work dir, so relative paths would not open
    std, flags, exp = header(path)
    rel = os.path.relpath(path, ROOT)
    wd = os.path.join(CACHE, rel.replace('/', '_'))
    shutil.rmtree(wd, ignore_errors=True)
    os.makedirs(wd)
    res = {}
    cmd = ['nvc', '--std=' + std, '-a'] + (['--relaxed'] if '--relaxed' in flags else []) + [path]
    rc, out = run(cmd, wd)
    errs = sorted({int(x) for x in re.findall(r'> ' + re.escape(path) + r':(\d+)', out)})
    warn = '** Warning' in out
    res['nvc'] = ('REJ' if rc else ('WARN' if warn else 'OK')) + ':' + ','.join(map(str, errs))
    gstd = {'2008': '08', '2019': '19'}[std]
    rc, out = run(['ghdl', '-a', '--std=' + gstd, '--workdir=' + wd, path], wd)
    errs = sorted({int(x) for x in re.findall(re.escape(path) + r':(\d+):\d+:\s*error', out)})
    warn = 'warning:' in out
    res['ghdl'] = ('REJ' if rc else ('WARN' if warn else 'OK')) + ':' + ','.join(map(str, errs))
    return '\t'.join([rel, ' '.join('%s@%d' % e for e in exp) or 'none', res['nvc'], res['ghdl']])


if __name__ == '__main__':
    a = sys.argv[1:]
    j = 8
    if a[:1] == ['-j']:
        j = min(8, int(a[1])); a = a[2:]
    files = a or sorted(glob.glob(ROOT + '/**/*.vhd', recursive=True))
    with ThreadPoolExecutor(j) as ex:
        for r in ex.map(one, files):
            print(r, flush=True)
