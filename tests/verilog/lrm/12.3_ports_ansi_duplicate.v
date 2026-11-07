// LRM 12.3: two ports of one ANSI header have different names
// expect: E4203 4:26 a declared twice
`timescale 1ns/1ps
module m (input a, input a);
endmodule
