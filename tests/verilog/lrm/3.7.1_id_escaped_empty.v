// LRM 3.7.1: an escaped identifier has at least one character (the BNF of A.9.3 lets the braces be empty; an identifier with no characters cannot be told from another, so it is refused)
// expect: E4014 5:8 empty escaped identifier
`timescale 1ns/1ps
module m;
  wire \ ;
endmodule
