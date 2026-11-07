// LRM 12.3.4: a module declares its ports in the header or in the body, not in both
// expect: E4202 5:9 direction in the body
`timescale 1ns/1ps
module m (input a);
  input b;
endmodule
