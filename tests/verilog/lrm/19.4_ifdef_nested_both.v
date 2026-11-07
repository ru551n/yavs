// LRM 19.4: conditionals nest
// units: a
// expect: none
`timescale 1ns/1ps
`define X
`define Y
`ifdef X
`ifdef Y
module a;
endmodule
`endif
`endif
