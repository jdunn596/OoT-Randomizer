import os
import subprocess

TOOLCHAIN = 'nightly' # good: 2025-02-05

subprocess.run(['rustup', 'toolchain', 'install', TOOLCHAIN], check=True)
subprocess.run(['rustup', 'component', 'add', 'rust-src', '--toolchain', TOOLCHAIN], check=True)
subprocess.run(
    ['cargo', f'+{TOOLCHAIN}', 'build', '-Zbuild-std=core', '-Zbuild-std-features=compiler-builtins-mem', '--release', '--package=rust-n64-test', '--target=crate/rust-n64-test/assets/mips-ultra64-cpu.json'],
    env={**os.environ, 'RUSTFLAGS': '-Csymbol-mangling-version=v0'}, # symbol-mangling-version (https://github.com/rust-lang/rust/issues/60705) required for armips to accept the symbols
    check=True,
)
