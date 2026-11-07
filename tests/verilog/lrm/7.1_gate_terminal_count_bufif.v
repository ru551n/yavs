// LRM 7.1: bufif1 has exactly three terminals
// expect: E4100 6:3 two terminals
`timescale 1ns/1ps
module m;
  wire o, i;
  bufif1 b (o, i);
endmodule
