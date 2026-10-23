Release builds run the interpreter's dispatch loop with up to 9 percent fewer instructions on call-heavy and loop-heavy code: the op census hook is compiled out instead of being an empty call.
