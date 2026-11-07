// LRM 17.9: $random with and without a seed
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  integer i, seed;
  initial begin
    i = $random;
    i = $random(seed);
  end
endmodule
