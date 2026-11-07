// LRM 3.8: an attribute instance ends with *)
// expect: E4100 5:3 attribute never closed
`timescale 1ns/1ps
module m;
  (* keep wire w;
endmodule
