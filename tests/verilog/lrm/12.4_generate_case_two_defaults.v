// LRM 12.4: a generate case has at most one default item
// expect: E4100 8:7 second default
`timescale 1ns/1ps
module m;
  generate
    case (1)
      default: wire a;
      default: wire b;
    endcase
  endgenerate
endmodule
