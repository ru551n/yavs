// LRM 9.8.1: a sequential block has declarations only if it is named; accepted with a warning, other tools accept it (V14)
// units: m
// expect: W4206 7:5 declaration_in_unnamed_block
`timescale 1ns/1ps
module m;
  initial begin
    reg r;
    r = 1;
  end
endmodule
