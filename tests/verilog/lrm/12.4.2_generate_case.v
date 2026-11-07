// LRM 12.4.2: a generate case selects a block by a constant expression
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  parameter P = 1;
  generate
    case (P)
      0: begin : a
        wire x;
      end
      1, 2: begin : b
        wire y;
      end
      default: begin : c
        wire z;
      end
    endcase
  endgenerate
endmodule
