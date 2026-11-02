#!/usr/bin/env python3
"""Pack the release binaries for one platform into dist/.

    python scripts/package_release.py                       # host build in target/release
    python scripts/package_release.py --target aarch64-apple-darwin

Output: dist/MINT-<platform>-<arch>-<commit>.zip (+ .sha256). The GitHub release
title must END with the short commit hash (the in-app updater compares it).
Run `cargo build --release [--target T]` first.
"""
import argparse, hashlib, os, subprocess, sys, zipfile

TARGETS = {
    "x86_64-pc-windows-msvc": ("windows", "x64", ".exe"),
    "x86_64-unknown-linux-gnu": ("linux", "x64", ""),
    "aarch64-unknown-linux-gnu": ("linux", "arm64", ""),
    "aarch64-apple-darwin": ("macos", "arm64", ""),
    "x86_64-apple-darwin": ("macos", "x64", ""),
}

def host():
    out = subprocess.run(["rustc", "-vV"], capture_output=True, text=True, check=True).stdout
    return next(l.split(": ")[1] for l in out.splitlines() if l.startswith("host:"))

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--target", help="cargo target triple (default: host build in target/release)")
    a = ap.parse_args()
    triple = a.target or host()
    if triple not in TARGETS:
        sys.exit(f"unsupported target {triple}")
    plat, arch, ext = TARGETS[triple]
    commit = subprocess.run(["git", "rev-parse", "--short=7", "HEAD"], capture_output=True, text=True, check=True).stdout.strip()
    if subprocess.run(["git", "status", "--porcelain"], capture_output=True, text=True).stdout.strip():
        print(f"warning: uncommitted changes, the build will not match commit {commit}", file=sys.stderr)
    bindir = os.path.join("target", a.target, "release") if a.target else os.path.join("target", "release")
    files = [(os.path.join(bindir, f"mint-gui{ext}"), True), (os.path.join(bindir, f"mint-cli{ext}"), True),
             ("LICENSE", False), ("THIRD_PARTY_NOTICES.md", False), ("README.md", False)]
    os.makedirs("dist", exist_ok=True)
    zpath = os.path.join("dist", f"MINT-{plat}-{arch}-{commit}.zip")
    if os.path.exists(zpath):
        os.remove(zpath)
    with zipfile.ZipFile(zpath, "w", zipfile.ZIP_DEFLATED) as z:
        for path, is_bin in files:
            if not os.path.exists(path):
                sys.exit(f"missing {path} (did you run cargo build --release?)")
            info = zipfile.ZipInfo.from_file(path, os.path.basename(path))
            info.compress_type = zipfile.ZIP_DEFLATED
            info.external_attr = (0o755 if is_bin else 0o644) << 16  # keep the exec bit on Linux/macOS
            with open(path, "rb") as f:
                z.writestr(info, f.read())
    digest = hashlib.sha256(open(zpath, "rb").read()).hexdigest()
    with open(zpath + ".sha256", "w") as f:
        f.write(f"{digest}  {os.path.basename(zpath)}")
    print(f"Built {zpath} ({os.path.getsize(zpath) / 1e6:.1f} MB, sha256 {digest[:12]}...)")

main()
