// LRM 3.7.3: a system name is $ followed by letters, digits, $ and underscores
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $a_1$b(1);
endmodule
