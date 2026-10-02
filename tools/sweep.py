#!/usr/bin/env python3
"""Runs every example program headlessly and reports why each one stopped."""
import collections, concurrent.futures, glob, os, re, shutil, subprocess, sys, tempfile
root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
cli = os.path.join(root, "target", "main", "release", "amos-cli")
frames = sys.argv[1] if len(sys.argv) > 1 else "150"
# Programs may write files (high scores...): run them on a copy of the
# distribution so the original sources are never modified.
work = tempfile.mkdtemp(prefix="amos-sweep-")
shutil.copytree(os.path.join(root, "AMOS-Professional-365", "AMOS"), os.path.join(work, "AMOS"))
files = [f for f in glob.glob(os.path.join(work, "AMOS", "**", "*"), recursive=True)
         if f.lower().endswith(".amos")]

def run(f):
    try:
        out = subprocess.run([cli, "run", f, "--frames", frames], capture_output=True, timeout=120).stdout.decode("latin-1")
    except subprocess.TimeoutExpired:
        return f, "TIMEOUT"
    lines = out.splitlines()
    state = next((l for l in lines if l.startswith("-- state")), "")
    if "Running" in state:
        return f, "running"
    i = lines.index(state) if state in lines else len(lines)
    reason = lines[i - 1] if i > 0 else "?"
    if "Stop(End)" in state:
        reason = "END"
    return f, reason

with concurrent.futures.ThreadPoolExecutor(8) as ex:
    results = list(ex.map(run, files))
c = collections.Counter(r for _, r in results)
for r, n in c.most_common():
    print("%4d  %s" % (n, r[:90]))
if "-v" in sys.argv:
    for f, r in sorted(results):
        print(r[:60].ljust(60), os.path.relpath(f, work))
shutil.rmtree(work, ignore_errors=True)
