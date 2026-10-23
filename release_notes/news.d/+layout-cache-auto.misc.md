Compiled code under `--tier=auto` keeps record layouts in the process frame as `--tier=llvm` does, which saves the host call per field access once a process is compiled.
