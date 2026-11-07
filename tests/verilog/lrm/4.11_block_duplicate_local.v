// LRM 4.11: a name is declared once in a named block
// expect: E4203 7:13 declared twice
`timescale 1ns/1ps
module m;
  initial begin : blk
    reg r;
    integer r;
  end
endmodule
