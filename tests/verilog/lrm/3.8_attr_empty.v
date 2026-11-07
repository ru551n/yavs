// LRM 3.8: an attribute instance holds at least one attribute name
// expect: E4100 5:3 empty attribute instance
`timescale 1ns/1ps
module m;
  (* *) wire w;
endmodule
