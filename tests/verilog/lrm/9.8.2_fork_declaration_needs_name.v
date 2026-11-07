// LRM 9.8.2: a parallel block has declarations only if it is named; accepted with a warning, other tools accept it (V14)
// units: m
// expect: W4206 7:5 declaration_in_unnamed_fork
`timescale 1ns/1ps
module m;
  initial fork
    integer i;
  join
endmodule
