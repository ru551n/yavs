// LRM 19.4: `ifndef skips its text when the macro is defined
// units: b
// expect: none
`timescale 1ns/1ps
`define X
`ifndef X
module a;
endmodule
`endif
module b;
endmodule
