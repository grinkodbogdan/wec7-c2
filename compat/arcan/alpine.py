#!/usr/bin/env python3
"""Fetch Alpine Linux aarch64 packages into a sysroot.

    alpine.py CACHE DEST LOCKFILE [--update] PACKAGE...

Packages and their dependencies are resolved from the Alpine index once and
recorded in LOCKFILE (name, version, sha256). Later runs fetch exactly the
locked files and verify their checksums; if Alpine has replaced a locked
version, run again with --update to re-resolve and review the new lock.
"""
import hashlib
import os
import subprocess
import sys
import tarfile

REL = "v3.22"
MIRROR = "https://dl-cdn.alpinelinux.org/alpine"
SKIP = {"busybox", "alpine-baselayout", "apk-tools", "scanelf", "ssl_client", "musl-utils"}


def fetch(url, path):
    subprocess.run(["curl", "-fsSL", "--retry", "3", "-o", path, url], check=True)


def index(cache):
    idx, prov = {}, {}
    for repo in ["main", "community"]:
        path = f"{cache}/APKINDEX-{repo}"
        if not os.path.exists(path):
            fetch(f"{MIRROR}/{REL}/{repo}/aarch64/APKINDEX.tar.gz", path + ".tgz")
            with tarfile.open(path + ".tgz") as t:
                open(path, "wb").write(t.extractfile("APKINDEX").read())
        for blk in open(path).read().split("\n\n"):
            d = {}
            for line in blk.splitlines():
                if len(line) > 2 and line[1] == ":":
                    d[line[0]] = line[2:]
            if "P" not in d:
                continue
            d["repo"] = repo
            idx.setdefault(d["P"], d)
            for p in d.get("p", "").split():
                prov.setdefault(p.split("=")[0], d["P"])
    return idx, prov


def resolve(cache, want):
    idx, prov = index(cache)

    def name(dep):
        dep = dep.lstrip("!")
        for op in ["<=", ">=", "=", "<", ">", "~"]:
            dep = dep.split(op)[0]
        return dep if dep in idx else prov.get(dep)

    todo, seen = list(want), set()
    while todo:
        n = name(todo.pop())
        if not n or n in seen or n in SKIP:
            continue
        seen.add(n)
        todo.extend(d for d in idx[n].get("D", "").split() if not d.startswith("!"))
    return [(n, idx[n]["V"], idx[n]["repo"]) for n in sorted(seen)]


def main():
    cache, dest, lock = sys.argv[1:4]
    args = sys.argv[4:]
    update = "--update" in args
    want = [a for a in args if a != "--update"]
    os.makedirs(cache, exist_ok=True)
    if update or not os.path.exists(lock):
        pkgs = [(n, v, r, None) for n, v, r in resolve(cache, want)]
    else:
        pkgs = [tuple(line.split()) for line in open(lock) if line.strip() and not line.startswith("#")]
    out = []
    for n, v, repo, sha in pkgs:
        f = f"{cache}/{n}-{v}.apk"
        if not os.path.exists(f):
            try:
                fetch(f"{MIRROR}/{REL}/{repo}/aarch64/{n}-{v}.apk", f)
            except subprocess.CalledProcessError:
                sys.exit(f"alpine.py: {n}-{v} is no longer on the mirror; rerun with --update and review the lock")
        got = hashlib.sha256(open(f, "rb").read()).hexdigest()
        if sha and got != sha:
            os.unlink(f)
            sys.exit(f"alpine.py: checksum mismatch for {n}-{v}: {got}")
        out.append(f"{n} {v} {repo} {got}")
        os.makedirs(dest, exist_ok=True)
        subprocess.run(
            ["tar", "xzf", f, "-C", dest, "--exclude=.PKGINFO", "--exclude=.SIGN*", "--exclude=.pre-*",
             "--exclude=.post-*", "--exclude=.trigger"],
            check=True, stderr=subprocess.DEVNULL)
    if update or not os.path.exists(lock):
        with open(lock, "w") as fh:
            fh.write(f"# Alpine {REL} aarch64 packages: name version repository sha256\n")
            fh.write("\n".join(out) + "\n")


main()
