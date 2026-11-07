// LRM 19.4: a conditional has at most one `else
// expect: E4004 6:1 second `else
`timescale 1ns/1ps
`ifdef X
`else
`else
`endif
module a;
endmodule
