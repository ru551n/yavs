// LRM 3.7.2: SystemVerilog keywords are ordinary identifiers in 1364-2005
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  wire logic;
  reg bit;
  integer int;
  reg ref;
  wire string;
endmodule
