// LRM 19.4: `ifdef is followed by a macro name
// expect: E4009 4:1 no macro name
`timescale 1ns/1ps
`ifdef
`endif
module a;
endmodule
