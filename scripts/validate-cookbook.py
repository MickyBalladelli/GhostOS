#!/usr/bin/env python3
"""Build all executable cookbook examples using the locked workspace graph."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def main() -> int:
    result = subprocess.run(
        ["cargo", "check", "--locked", "-p", "ghostos-cookbook", "--bins"],
        cwd=ROOT,
        check=False,
    )
    if result.returncode:
        print("cookbook validation failed", file=sys.stderr)
        return result.returncode
    print("cookbook valid: seven executable public-interface examples")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
