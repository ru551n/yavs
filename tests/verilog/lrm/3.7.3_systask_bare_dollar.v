// LRM 3.7.3: a $ must be followed by a name
// expect: E4100 5:11 $ without a name
`timescale 1ns/1ps
module m;
  initial $ (1);
endmodule
