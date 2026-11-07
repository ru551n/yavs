// LRM 3.5.1: a literal shorter than its size is extended with zeros, or with x or z when its leftmost digit is x or z
// expect: none
// output: xxxxxxxx 00000001 zzzzzzz1 0000000x
`timescale 1ns/1ps
module m;
  initial $display("%b %b %b %b", 8'bx, 8'b1, 8'bz1, 8'b0x);
endmodule
