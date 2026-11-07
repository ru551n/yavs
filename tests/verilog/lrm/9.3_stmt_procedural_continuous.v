// LRM 9.3: assign, deassign, force and release are procedural continuous assignments
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  reg a, b;
  initial begin
    assign a = 1;
    deassign a;
    force b = 1;
    release b;
  end
endmodule
