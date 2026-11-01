"""External tools, and the mitex typst package built from vendor/mitex.

Python dependencies (Pillow, Pagefind, blake3) come from pyproject.toml,
and the converter (nlab._conv) is built by `uv` itself. What remains
outside: Rust (rustup; rust-toolchain.toml selects stable + the wasm
target), typst 0.15.x, svgo, and git (for the vendor/mitex submodule).

Usage: nlab setup
"""

import argparse
import json
import os
import pathlib
import platform
import shutil
import subprocess

from .paths import MITEX, ROOT

TYPST_WANT = "0.15"
TOOLS = {
    "git": "install git",
    "cargo": "install rustup (https://rustup.rs) or `brew install rustup && rustup-init`",
    "typst": f"typst {TYPST_WANT}.x: `cargo install --locked typst-cli` or `brew install typst`",
    "svgo": "`brew install svgo` or `npm install -g svgo`",
}


def check_tools() -> None:
    missing = [f"  {t}: {hint}" for t, hint in TOOLS.items() if not shutil.which(t)]
    if missing:
        raise SystemExit("missing tools:\n" + "\n".join(missing))
    version = subprocess.run(["typst", "--version"], capture_output=True,
                             text=True).stdout.split()
    if len(version) < 2 or not version[1].startswith(TYPST_WANT):
        print(f"warning: {' '.join(version)}; the pipeline was developed on "
              f"typst {TYPST_WANT}.x")


def mitex_package_dir() -> pathlib.Path:
    """Where typst looks for @local/mitex:0.2.7."""
    if platform.system() == "Darwin":
        base = pathlib.Path.home() / "Library/Application Support"
    else:
        base = pathlib.Path(os.environ.get("XDG_DATA_HOME")
                            or pathlib.Path.home() / ".local/share")
    return base / "typst/packages/local/mitex/0.2.7"


def git(*args: str, cwd: pathlib.Path = ROOT) -> str:
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True,
                          check=True).stdout.strip()


def ensure_mitex_checkout() -> None:
    if not (MITEX / ".git").exists():
        if not (ROOT / ".git").exists():
            raise SystemExit(f"error: {MITEX} is missing and {ROOT} is not a git "
                             "checkout; clone with `git clone --recursive`")
        subprocess.run(["git", "submodule", "update", "--init", "vendor/mitex"],
                       cwd=ROOT, check=True)
    pinned = git("ls-tree", "HEAD", "vendor/mitex").split()[2]
    actual = git("rev-parse", "HEAD", cwd=MITEX)
    if actual != pinned:
        print(f"note: vendor/mitex is at {actual[:7]}, this repo pins {pinned[:7]} "
              "(fine while developing the fork; `git submodule update` resets it)")
    if git("status", "--porcelain", cwd=MITEX):
        print("note: vendor/mitex has uncommitted changes; building them")


def install_mitex_package() -> pathlib.Path:
    """Build the mitex wasm plugin and install packages/mitex as
    @local/mitex:0.2.7 (rust-toolchain.toml at the root picks stable)."""
    subprocess.run(
        ["cargo", "build", "--release", "--target", "wasm32-unknown-unknown",
         "--manifest-path", str(MITEX / "crates/mitex-wasm/Cargo.toml"),
         "--features", "typst-plugin"],
        cwd=ROOT, check=True)
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps",
         "--manifest-path", str(MITEX / "Cargo.toml")],
        cwd=ROOT, capture_output=True, text=True, check=True).stdout)
    wasm = (pathlib.Path(meta["target_directory"])
            / "wasm32-unknown-unknown/release/mitex_wasm.wasm")
    pkg = mitex_package_dir()
    shutil.rmtree(pkg, ignore_errors=True)
    shutil.copytree(MITEX / "packages/mitex", pkg)
    shutil.copy(wasm, pkg / "mitex.wasm")
    print(f"installed @local/mitex:0.2.7 -> {pkg}")
    return pkg


def setup() -> None:
    check_tools()
    ensure_mitex_checkout()
    install_mitex_package()


def main(argv=None):
    argparse.ArgumentParser(prog="nlab setup", description=__doc__.split("\n\n")[0]
                            ).parse_args(argv)
    setup()


if __name__ == "__main__":
    main()
