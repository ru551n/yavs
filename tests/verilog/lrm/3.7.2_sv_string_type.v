// LRM 3.7.2: string as a variable type is SystemVerilog
// expect: E4003 5:3 string is SystemVerilog
`timescale 1ns/1ps
module m;
  string s;
endmodule
