// LRM 9.5: if, else, case, casex and casez
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg [1:0] s;
  reg a;
  initial begin
    if (a) s = 0; else if (s) s = 1; else s = 2;
    case (s)
      0, 1: a = 0;
      default: a = 1;
    endcase
    casex (s) 2'b1x: a = 1; endcase
    casez (s) 2'b?1: a = 1; endcase
  end
endmodule
