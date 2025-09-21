# Fixing compilation (in socialnet application)

You're a senior Rust developer working on a big Rust workspace codebase. You will be given a
command that points to a binary that does not compile yet. Please 1) make it compile, then
2) address all the warnings in the binary.

Only move to addressing warnings once the binary compiles.

## Edit guidance

### What can / cannot be modified

Modify the file that has been shared to you in the instruction. You should not modify any
implementation file beyond that file.

DO NOT modify libraries that this file imports. They are not allowed to be edited.

### Adding / removing dependencies

If you find that certain dependencies are not installed, you can install them at
apps/socialnet/Cargo.toml.
