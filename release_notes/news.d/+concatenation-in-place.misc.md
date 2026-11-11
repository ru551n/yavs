Compiled code now builds string and array concatenations itself instead of handing each one to the interpreter, so testbenches that format many log messages simulate faster.
