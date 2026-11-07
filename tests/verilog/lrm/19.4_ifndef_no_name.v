// LRM 19.4: `ifndef is followed by a macro name
// expect: E4009 4:1 no macro name
`timescale 1ns/1ps
`ifndef
`endif
module a;
endmodule
