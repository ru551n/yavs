Compiled code now frees an object it deallocates without handing the work to the interpreter, so designs that build and free many strings or lists in small functions simulate faster.
