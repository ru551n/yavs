// LRM 3.4: ==? is not a 1364 operator
// expect: E4003 6:17 ==? is SystemVerilog
`timescale 1ns/1ps
module m;
  integer a, b, c;
  initial c = a ==? b;
endmodule
