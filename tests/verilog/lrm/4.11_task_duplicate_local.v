// LRM 4.11: a name is declared once in the name space of a task
// expect: E4203 8:13 declared twice
`timescale 1ns/1ps
module m;
  task t;
    input a;
    reg r;
    integer r;
    begin end
  endtask
endmodule
