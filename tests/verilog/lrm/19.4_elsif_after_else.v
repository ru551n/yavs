// LRM 19.4: `elsif comes before `else
// expect: E4004 6:1 `elsif after `else
`timescale 1ns/1ps
`ifdef X
`else
`elsif Y
`endif
module a;
endmodule
