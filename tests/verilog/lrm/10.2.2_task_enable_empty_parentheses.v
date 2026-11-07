// LRM 10.2.2: a task without arguments is enabled without a list; accepted with a warning, other tools accept it (V14)
// units: m
// expect: W4205 9:12 empty_parentheses
`timescale 1ns/1ps
module m;
  task t;
    begin end
  endtask
  initial t();
endmodule
