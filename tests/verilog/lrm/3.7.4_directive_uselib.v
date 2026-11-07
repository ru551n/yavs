// LRM 3.7.4: `uselib is not supported
// expect: E4010 4:1 `uselib not supported
`timescale 1ns/1ps
`uselib lib = x
module m;
endmodule
