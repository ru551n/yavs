// LRM 10.2: a name is declared once in a task scope (known gap G2: duplicates inside task, function and block scopes are not checked)
// units: m
// gap: G2 duplicate declarations are checked at module level only
// expect: none
`timescale 1ns/1ps
module m;
  task t;
    input a;
    reg a;
  endtask
endmodule
