#!/usr/bin/env python3
"""Release gate (#1017): the release waits for the conformance run of its own tag and stops if it is red.

  tools/release/conformance_gate.py wait TAG SHA   # release.yml, tag push: needs GH_TOKEN, GH_REPO, actions: read
  tools/release/conformance_gate.py list SHA       # dry run: show the conformance runs of SHA, never fails on them
  tools/release/conformance_gate.py --selftest

conformance.yml regenerates the report at the tagged commit and, on a tag, fails when the committed
docs/conformance.md differs (the file release.yml publishes). Only that run proves the report belongs
to the release: a nightly run on main passes with a warning when the report is stale.
"""
import json, subprocess, sys, time

APPEAR_MIN, FINISH_MIN, POLL_S = 15, 130, 60  # tag run appears at once; it has a 90 minute timeout plus queueing


def runs_of(sha):
    p = subprocess.run(["gh", "run", "list", "--workflow", "conformance.yml", "--commit", sha, "--limit", "50",
                        "--json", "databaseId,event,headBranch,status,conclusion,url"], capture_output=True, text=True)
    if p.returncode:
        sys.exit(f"could not list the conformance runs of {sha} (needs gh, GH_TOKEN, GH_REPO and actions: read):\n{p.stderr}")
    return json.loads(p.stdout)


def verdict(runs, tag):
    """('missing' | 'running' | 'ok' | 'failed', run) for the newest push run of the tag."""
    mine = [r for r in runs if r["event"] == "push" and r["headBranch"] == tag]
    if not mine:
        return "missing", None
    r = max(mine, key=lambda r: r["databaseId"])
    if r["status"] != "completed":
        return "running", r
    return ("ok" if r["conclusion"] == "success" else "failed"), r


def wait(tag, sha):
    start = time.time()
    while True:
        state, r = verdict(runs_of(sha), tag)
        waited = (time.time() - start) / 60
        if state == "ok":
            print(f"conformance run {r['url']} passed: the committed report is the one of {sha}")
            return
        if state == "failed":
            sys.exit(f"::error::the conformance run of {tag} is {r['conclusion']}: {r['url']}\nThe committed docs/conformance.md is stale or a conformance job failed. "
                     "Refresh it on main (RELEASING.md step 1); a published tag is never moved, so release the next patch version.")
        if state == "missing" and waited > APPEAR_MIN or waited > FINISH_MIN:
            sys.exit(f"::error::no finished conformance run of {tag} on {sha} after {waited:.0f} minutes ({state}); is conformance.yml triggered by tags?")
        print(f"conformance run of {tag}: {state}, {waited:.0f} min")
        time.sleep(POLL_S)


def selftest():
    def run(i, event, branch, status="completed", conclusion="success"):
        return dict(databaseId=i, event=event, headBranch=branch, status=status, conclusion=conclusion, url=f"u{i}")
    main_ok = run(1, "push", "main")
    assert verdict([main_ok], "v1")[0] == "missing"  # a green run on main is no evidence
    assert verdict([main_ok, run(2, "workflow_dispatch", "v1")], "v1")[0] == "missing"
    assert verdict([main_ok, run(2, "push", "v1", conclusion="failure")], "v1")[0] == "failed"
    assert verdict([run(2, "push", "v1", "in_progress", "")], "v1")[0] == "running"
    assert verdict([run(2, "push", "v1", conclusion="failure"), run(3, "push", "v1")], "v1")[0] == "ok"  # a re-run
    assert verdict([run(2, "push", "v1")], "v2")[0] == "missing"
    print("conformance_gate selftest: ok")


if __name__ == "__main__":
    a = sys.argv[1:]
    if a == ["--selftest"]:
        selftest()
    elif len(a) == 3 and a[0] == "wait":
        wait(a[1], a[2])
    elif len(a) == 2 and a[0] == "list":
        for r in runs_of(a[1]):
            print(f"{r['event']:18} {r['headBranch']:12} {r['status']} {r['conclusion'] or '-':8} {r['url']}")
    else:
        sys.exit(__doc__)
