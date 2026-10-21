#!/usr/bin/env python3
"""Run sim1 and sim2 over tests/diag/sem/**/*.vhd and print one TSV row per file.

Columns: path, expected (code@line ...), sim1 (OK|REJ|WARN + error lines), sim2 (same).
Commands: keys negtest_cmd (bash; env FILE STD STD2 WD RELAXED), negtest_err (regex with {path}, group 1 = line) and
negtest_warn (regex) of each label's section in tools/sims.local.conf.
Work dirs live under ~/.cache/rsim-negtests, never in the repo. Max 8 in parallel.
Usage: check_ref.py [-j N] [files...]   (default: all files)
"""
import glob, os, re, shutil, subprocess, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', '..', '..', 'tools'))
import simconf  # noqa: E402
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
    env = dict(os.environ)
    rt = '/run/user/%d' % os.getuid()
    if 'XDG_RUNTIME_DIR' not in env and os.path.isdir(rt):
        env['XDG_RUNTIME_DIR'] = rt  # systemd-run --user needs the user bus; a scrubbed environment lacks it
    p = subprocess.run(full, cwd=cwd, capture_output=True, text=True, env=env)
    out = p.stdout + p.stderr
    if p.returncode and out.startswith('Failed to '):
        # a wrapper failure must not look like a rejection by the tool
        sys.exit('check_ref.py: systemd-run failed: ' + out.strip())
    return p.returncode, out


def one(path):
    path = os.path.abspath(path)  # the tools run in a work dir, so relative paths would not open
    std, flags, exp = header(path)
    rel = os.path.relpath(path, ROOT)
    wd = os.path.join(CACHE, rel.replace('/', '_'))
    shutil.rmtree(wd, ignore_errors=True)
    os.makedirs(wd)
    res = {}
    env = {'FILE': path, 'STD': std, 'STD2': std[2:], 'WD': wd, 'RELAXED': '1' if '--relaxed' in flags else ''}
    for lab in ('sim1', 'sim2'):
        cmd = simconf.get(lab, 'negtest_cmd')
        if cmd is None:
            sys.exit('check_ref.py: ' + lab + ': key negtest_cmd not configured (tools/sims.local.conf)')
        rc, out = run(['env'] + ['%s=%s' % kv for kv in env.items()] + ['bash', '-c', cmd], wd)
        errs = sorted({int(x) for x in re.findall(simconf.get(lab, 'negtest_err').replace('{path}', re.escape(path)), out)})
        warn = re.search(simconf.get(lab, 'negtest_warn'), out) is not None
        res[lab] = ('REJ' if rc else ('WARN' if warn else 'OK')) + ':' + ','.join(map(str, errs))
    return '\t'.join([rel, ' '.join('%s@%d' % e for e in exp) or 'none', res['sim1'], res['sim2']])


if __name__ == '__main__':
    a = sys.argv[1:]
    j = 8
    if a[:1] == ['-j']:
        j = min(8, int(a[1])); a = a[2:]
    files = a or sorted(glob.glob(ROOT + '/**/*.vhd', recursive=True))
    with ThreadPoolExecutor(j) as ex:
        for r in ex.map(one, files):
            print(r, flush=True)
