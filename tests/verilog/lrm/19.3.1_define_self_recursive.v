// LRM 19.3.1: a macro whose text uses itself never finishes expanding
// expect: E4011 6:17 expansion never ends
`timescale 1ns/1ps
`define LOOP `LOOP
module m;
  parameter p = `LOOP;
endmodule
