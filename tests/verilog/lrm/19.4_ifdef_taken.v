// LRM 19.4: `ifdef compiles its text when the macro is defined
// units: a
// expect: none
`timescale 1ns/1ps
`define X
`ifdef X
module a;
endmodule
`else
module b;
endmodule
`endif
