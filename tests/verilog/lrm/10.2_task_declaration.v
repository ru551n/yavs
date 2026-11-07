// LRM 10.2: a task has ports, declarations and a statement; tasks may be automatic
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  task t;
    input a;
    output b;
    reg c;
    begin b = a; end
  endtask
  task automatic u (input x, output y);
    y = x;
  endtask
endmodule
