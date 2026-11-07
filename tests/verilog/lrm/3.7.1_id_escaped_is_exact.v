// LRM 3.7.1: the backslash is not part of the name: \cpu3 and cpu3 are the same identifier
// expect: E4203 6:8 declared twice
`timescale 1ns/1ps
module m;
  wire \cpu3 ;
  wire cpu3;
endmodule
