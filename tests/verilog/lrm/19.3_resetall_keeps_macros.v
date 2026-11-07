// LRM 19.3: `resetall does not affect the text macros
// units: m
// expect: none
`define A 1
`resetall
`timescale 1ns/1ps
module m;
  parameter p = `A;
endmodule
