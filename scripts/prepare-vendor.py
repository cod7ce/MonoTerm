"""Apply MonoTerm's reviewed terminal patch without overwriting local edits."""
from pathlib import Path
import subprocess
import sys

root = Path(sys.argv[1]).resolve()
component = root / "vendor/gpui-ghostty"
patch = root / "patches/gpui-ghostty.patch"
base = ["git", "-C", str(component), "apply"]
if subprocess.run(base + ["--reverse", "--check", str(patch)], capture_output=True).returncode == 0:
    print("MonoTerm terminal patch is already applied.")
elif subprocess.run(base + ["--check", str(patch)], capture_output=True).returncode == 0:
    subprocess.run(base + [str(patch)], check=True)
    print("Applied MonoTerm's Ghostty 1.4 adapter and renderer patch.")
else:
    raise SystemExit("Terminal source differs from the pinned baseline and patch; inspect local changes before proceeding.")
