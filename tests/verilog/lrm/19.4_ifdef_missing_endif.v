// LRM 19.4: every `ifdef is closed by an `endif in the same file
// expect: E4004 4:1 no `endif
`timescale 1ns/1ps
`ifdef X
module a;
endmodule
