// LRM 12.3.4: a port type outside 1364 (here logic) is SystemVerilog
// expect: E4003 4:17 logic port is SystemVerilog
`timescale 1ns/1ps
module m (input logic a);
endmodule
