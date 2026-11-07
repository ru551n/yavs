// LRM 12.4: an end label after a block is SystemVerilog
// expect: E4003 7:9 end label is SystemVerilog
`timescale 1ns/1ps
module m;
  generate
    if (1) begin : g
    end : g
  endgenerate
endmodule
