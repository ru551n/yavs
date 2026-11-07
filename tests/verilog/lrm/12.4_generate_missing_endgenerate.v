// LRM 12.4: generate ends with endgenerate
// expect: E4100 8:1 endgenerate missing
`timescale 1ns/1ps
module m;
  generate
    if (1) begin : g
    end
endmodule
