// LRM 3.7.2: package and import are SystemVerilog
// expect: E4003 4:1 package is SystemVerilog
`timescale 1ns/1ps
package p;
endpackage
module m;
endmodule
