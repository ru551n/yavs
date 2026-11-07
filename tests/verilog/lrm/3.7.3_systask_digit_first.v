// LRM 3.7.3: a system name may start with a digit: the characters after the $ are letters, digits, $ and underscores
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $1abc(1);
endmodule
