// LRM 12.1: macromodule is the same as module; not supported
// expect: E4101 4:1 macromodule not supported
`timescale 1ns/1ps
macromodule m;
endmodule
