// LRM 19.3.1: the argument list of a macro call ends with a closing parenthesis
// expect: E4008 6:17 argument list never closed
`timescale 1ns/1ps
`define ID(a) a
module m;
  parameter p = `ID(1;
endmodule
