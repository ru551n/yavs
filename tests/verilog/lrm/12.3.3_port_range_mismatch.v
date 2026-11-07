// LRM 12.3.3: the two declarations of a port give it the same range (known gap G11: not checked)
// units: m
// gap: G11 rules that need the elaborated design are not checked
// expect: none
`timescale 1ns/1ps
module m (a);
  input [3:0] a;
  wire [7:0] a;
endmodule
