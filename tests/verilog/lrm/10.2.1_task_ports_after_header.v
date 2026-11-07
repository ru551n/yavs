// LRM 10.2.1: the ports of a task are in its header or in its body, not in both
// expect: E4100 5:5 input after header ports
module m;
  task t (input a);
    input b;
    begin end
  endtask
endmodule
