// LRM 6.2.1: a variable declaration assignment is only allowed at module level
// expect: E4100 5:13 initial value in a task
module m;
  task t;
    integer i = 0;
    begin end
  endtask
endmodule
