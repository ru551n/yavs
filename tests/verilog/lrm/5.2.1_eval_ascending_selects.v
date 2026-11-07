// LRM 5.2.1: selects of a vector declared [0:7] count from its left (most significant) bit: a bit-select, a part-select, +: and -: with constant and variable bases, read and written
// expect: none
// output: 1 0010 1100 010 001
// output: 01110000
`timescale 1ns/1ps
module m;
  wire [0:7] c = 8'b11001010;
  reg [0:7] r;
  integer i = 2;
  initial begin
    #1 $display("%b %b %b %b %b", c[1], c[2:5], c[0 +: 4], c[7 -: 3], c[i +: 3]);
    r = 0;
    r[1] = 1;
    r[i +: 2] = 2'b11;
    $display("%b", r);
  end
endmodule
