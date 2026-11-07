// LRM 3.7.2: typedef is SystemVerilog
// expect: E4003 5:3 typedef is SystemVerilog
`timescale 1ns/1ps
module m;
  typedef reg r_t;
endmodule
