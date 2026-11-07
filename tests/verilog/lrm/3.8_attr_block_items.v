// LRM 3.8: an attribute instance may precede block item declarations and the port declarations of a task
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  task t;
    (* a *) input x;
    (* b *) reg r;
    begin end
  endtask
  initial begin : blk
    (* c *) integer i;
  end
endmodule
