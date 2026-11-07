// LRM 9.5: a case statement has at most one default item
// expect: E4100 8:5 second default
`timescale 1ns/1ps
module m;
  reg a;
  initial case (a)
    default: a = 0;
    default: a = 1;
  endcase
endmodule
