// LRM 10.2.2: a task without arguments is enabled without a list
// expect: E4100 7:12 empty parentheses
module m;
  task t;
    begin end
  endtask
  initial t();
endmodule
