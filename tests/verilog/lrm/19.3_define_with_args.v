// LRM 19.3: a macro may have formal arguments, replaced by the actual arguments of the call
// units: m
// expect: none
`timescale 1ns/1ps
`define ADD(a, b) ((a) + (b))
module m;
  parameter p = `ADD(1, 2);
endmodule
