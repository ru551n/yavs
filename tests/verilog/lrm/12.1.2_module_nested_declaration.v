// LRM 12.1.2: a module declaration does not nest in 1364-2005
// expect: E4003 5:3 nested module declaration is SystemVerilog
`timescale 1ns/1ps
module outer;
  module inner;
  endmodule
endmodule
