#!/usr/bin/env python3
"""Lists keyword constants of tokens/names.rs never referenced in the Rust sources."""
import os, re, sys
root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "amos-core", "src")
names = open(os.path.join(root, "tokens", "names.rs")).read()
consts = re.findall(r"/// `([^`]*)` ([^\n]*)\npub const (\w+)", names)
code = ""
for d, _, files in os.walk(root):
    for f in files:
        if f.endswith(".rs") and f not in ("names.rs", "table.rs"):
            code += open(os.path.join(d, f)).read()
missing = [(n, kw, sig) for kw, sig, n in consts if not re.search(r"\b%s\b" % n, code)]
prefix = sys.argv[1] if len(sys.argv) > 1 else None
for n, kw, sig in missing:
    if prefix is None or n.startswith(prefix):
        print("%-28s %-26s %s" % (n, kw, sig))
print(len(missing), "of", len(consts), "keywords not referenced", file=sys.stderr)
