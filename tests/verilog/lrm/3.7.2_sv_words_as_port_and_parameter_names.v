// LRM 3.7.2: logic, bit, int and string are not keywords of 1364-2005: they name ports, task arguments and parameters
// units: m,n
// expect: none
`timescale 1ns/1ps
module m (a, int, bit);
  input a;
  output int;
  input bit;
  localparam string = 1;
  parameter logic = 2;
  task t;
    input logic, x;
    begin end
  endtask
endmodule
module n (input bit, output int, input logic);
endmodule
