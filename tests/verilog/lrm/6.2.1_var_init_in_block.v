// LRM 6.2.1: a variable declaration assignment is only allowed at module level; in a block it is accepted with a warning, other tools accept it (V14)
// units: m
// expect: W4207 7:13 initial_value_in_a_block
`timescale 1ns/1ps
module m;
  initial begin : b
    integer i = 0;
  end
endmodule
