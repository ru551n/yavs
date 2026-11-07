// LRM 12.3.4: ports in a list are separated by commas
// expect: E4100 4:19 missing comma
`timescale 1ns/1ps
module m (input a input b);
endmodule
