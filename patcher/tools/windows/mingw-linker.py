#!/usr/bin/env python3
"""Linux -> Windows GNU linker adapter for hosts that inject Linux-only mold flags.

Use CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=<absolute path to this file>.
The Rust build wrapper, including any resource checks, remains in effect.
"""
import os
import sys

os.execvp(
    "x86_64-w64-mingw32-gcc",
    ["x86_64-w64-mingw32-gcc", *[arg for arg in sys.argv[1:] if arg != "-fuse-ld=mold"]],
)
