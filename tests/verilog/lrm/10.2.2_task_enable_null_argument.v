// LRM 10.2.2: a null expression is not an argument of a task enable
// expect: E4100 8:12 empty argument
module m;
  task t;
    input a, b;
    begin end
  endtask
  initial t(, 1'b1);
endmodule
