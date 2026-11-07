// LRM 3.7.2: logic as a port type is SystemVerilog
// expect: E4003 4:18 logic port is SystemVerilog
`timescale 1ns/1ps
module m (output logic q);
endmodule
