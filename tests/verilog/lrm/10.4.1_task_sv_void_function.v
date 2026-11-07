// LRM 10.4.1: a void function is SystemVerilog
// expect: E4003 5:12 void is SystemVerilog
`timescale 1ns/1ps
module m;
  function void f (input a);
  endfunction
endmodule
