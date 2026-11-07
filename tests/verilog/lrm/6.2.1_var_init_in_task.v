// LRM 6.2.1: a variable declaration assignment is only allowed at module level; in a task it is accepted with a warning, other tools accept it (V14)
// units: m
// expect: W4207 7:13 initial_value_in_a_task
`timescale 1ns/1ps
module m;
  task t;
    integer i = 0;
    begin end
  endtask
endmodule
