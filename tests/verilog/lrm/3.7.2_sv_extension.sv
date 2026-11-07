// LRM 3.7.2: tool behaviour, not a rule of 1364-2005: a .sv file is SystemVerilog, which yavs does not read
// expect: E4002 1:1 SystemVerilog file extension
`timescale 1ns/1ps
module m;
endmodule
