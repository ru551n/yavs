// LRM 19.5: an included file may include another
// yavs-test: args=-I inc
// units: m
// expect: none
`timescale 1ns/1ps
`include "outer.vh"
module m;
  parameter p = `LRM_OUTER + `LRM_INNER;
endmodule
