// LRM 9.8.3: a named block has declarations and can be disabled
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a;
  initial begin : blk
    reg t;
    t = 1;
    disable blk;
  end
endmodule
